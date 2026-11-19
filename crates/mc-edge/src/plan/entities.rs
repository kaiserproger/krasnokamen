//! Entity removals, spawns, appearances and moves for one viewer.
//!
//! Retained interest drives the work; the wire encoders live in
//! `entity_motion`, `entity_appearance` and `entity_events`.
use super::*;

/// Recorded type of an entity whose frames were lost: no entity has it, so
/// the identity check fails and the plan removes and respawns the entity.
pub(super) const UNDELIVERED_TYPE: u32 = u32::MAX;

/// Whether the client lacks `entity`'s current appearance.
fn appearance_stale(viewer: &ViewerState, entity: &EntityView) -> bool {
    viewer
        .tracked_entities
        .get(&entity.id)
        .is_none_or(|sent| !Arc::ptr_eq(&sent.appearance, &entity.appearance))
}

/// Vanilla tracking uses an inclusive horizontal square, not a 3-D sphere.
pub(super) fn entity_trackable(
    entity: &EntityView,
    viewer: &ViewerState,
    view_distance: i32,
) -> bool {
    if entity.id == viewer.entity_id {
        return true;
    }
    let Some(range) =
        entity_interest::tracking_range(entity.type_id, view_distance, viewer.recorder)
    else {
        return false;
    };
    let range = f64::from(range * 16);
    (entity.position[0] - viewer.position[0]).abs() <= range
        && (entity.position[2] - viewer.position[2]).abs() <= range
}

/// Horizontal distance at which a pending entity update weighs half as much
/// as one at the viewer.
const UPDATE_PRIORITY_HALF_BLOCKS: f64 = 32.0;

/// Priority of a pending entity update under limited world credit, in the
/// spirit of a priority accumulator: entity plans waited since its last
/// delivery, weighted down with horizontal distance. Near units keep their
/// vanilla cadence; far units wait longer but never starve, because
/// waiting raises their priority every publish.
fn update_priority(waited: u64, entity: [f64; 3], viewer: [f64; 3]) -> f64 {
    let distance = (entity[0] - viewer[0]).hypot(entity[2] - viewer[2]);
    (waited as f64 + 1.0) * UPDATE_PRIORITY_HALF_BLOCKS / (UPDATE_PRIORITY_HALF_BLOCKS + distance)
}

/// One entity's pending update for this publish, ranked by [`update_priority`].
#[derive(Debug)]
pub(super) struct EntityUpdate {
    /// Canonical (chunk/id) visit order, the tie-break between priorities.
    order: usize,
    id: EntityId,
    priority: f64,
    stale: bool,
    /// [`TrackedEntity::resend`]: the whole state goes out again.
    resend: bool,
    /// An item entity, counted by [`item_probe`].
    item: bool,
    due: bool,
    movement: Option<shared_moves::SharedMove>,
    head: Option<u8>,
}

pub(super) fn entity_movement_due(
    entity: &EntityView,
    tick: u64,
    previous_tick: Option<u64>,
) -> bool {
    mc_data::entity_types::entity_type_contract_26_1_2_by_protocol_id(entity.type_id).is_some_and(
        |contract| {
            let interval = u64::from(contract.update_interval);
            let phase =
                i64::from(entity.id.0).rem_euclid(i64::from(contract.update_interval)) as u64;
            let position = (tick % interval + phase) % interval;
            let elapsed = previous_tick.map_or(0, |previous| tick.saturating_sub(previous));
            position == 0 || position < elapsed
        },
    )
}

/// Whether the viewer's record of `id` was invalidated by lost frames.
pub(super) fn is_undelivered(viewer: &ViewerState, id: EntityId) -> bool {
    viewer
        .tracked_entities
        .get(&id)
        .is_some_and(|tracked| tracked.type_id == UNDELIVERED_TYPE)
}

pub(super) fn plan_entities(
    snapshot: &WorldSnapshot,
    viewer: &mut ViewerState,
    view_distance: i32,
    scratch: &mut PlanScratch,
    index: &TickPlanIndex,
    out: &mut Vec<PlannedPacket>,
    credit: &mut WorldCredit,
) {
    let conn = viewer.connection;
    let mut tally = item_probe::Tally::new();
    let dimension = viewer.dimension;
    let previous_tick = viewer.entity_tick.replace(snapshot.tick);
    viewer.entity_round += 1;
    let round = viewer.entity_round;
    let mut interest = std::mem::take(&mut viewer.entity_interest);
    let spawn_credit = if credit.spent() { 0 } else { credit.frames };
    interest.prepare(
        snapshot,
        viewer,
        view_distance,
        index,
        scratch,
        spawn_credit,
    );
    if viewer.camera.is_some_and(|camera| {
        camera != viewer.entity_id
            && (scratch.removed.contains(&camera.0)
                || scratch.spawns.iter().any(|(id, _)| *id == camera))
    }) && !camera::send(viewer, viewer.entity_id, out, credit)
    {
        viewer.entity_interest = interest;
        return;
    }
    viewer.boss_baseline.sync(
        conn,
        scratch
            .bosses
            .iter()
            .map(|&id| snapshot.entities.get(id).expect("visible boss")),
        out,
    );
    viewer
        .boss_baseline
        .sync_raids(conn, viewer.position, &snapshot.raid_bars, out);

    // The removal of a visible entity whose frames were lost rides with its
    // spawn below: sent alone, it would hide the entity until the credit
    // carries the spawn, for every entity at once.
    if !scratch.removed.is_empty() && !scratch.spawns.is_empty() {
        let repaired: HashSet<i32> = scratch
            .spawns
            .iter()
            .filter(|(id, _)| is_undelivered(viewer, *id))
            .map(|(id, _)| id.0)
            .collect();
        scratch.removed.retain(|id| !repaired.contains(id));
    }
    let mut removals_deferred = scratch.removed.len();
    if !scratch.removed.is_empty() && !credit.spent() {
        let packet = RemoveEntities {
            entity_ids: std::mem::take(&mut scratch.removed),
        };
        if try_push(out, credit, |out| push_best_effort(out, conn, &packet)) {
            removals_deferred = 0;
            for &id in &packet.entity_ids {
                let id = EntityId(id);
                viewer.untrack(id);
                interest.removed(id);
                viewer.world_removed.push(id);
            }
        }
        scratch.removed = packet.entity_ids;
    }

    // The client's own entity gets no AddEntity but still needs its
    // SetEntityData (pose/flags/health), or the client player object never
    // resolves; resent whenever its appearance changes.
    let own_start = out.len();
    if let Some(own) = own_entity(snapshot, viewer)
        && appearance_stale(viewer, own)
        && try_push(out, credit, |out| {
            // Vanilla sends equipment only to trackers, never to the owner:
            // the client writes MAINHAND into its *currently* selected slot.
            push_appearance_with(
                out,
                conn,
                own.id,
                &own.appearance,
                own.type_id,
                viewer
                    .tracked_entities
                    .get(&own.id)
                    .map(|sent| sent.appearance.as_ref()),
                false,
            )
        })
    {
        viewer.track(own);
        if out.len() > own_start {
            viewer.world_entities.push(own.id);
        }
    }
    interest.spawned(viewer.entity_id);

    // Spawns, nearest first, as many as the credit carries.
    let nearest = |a: &(EntityId, f64), b: &(EntityId, f64)| a.1.total_cmp(&b.1);
    let spawn_candidates = scratch.pending_spawns;
    let mut spawns_sent = 0;
    if credit.spent() {
        scratch.spawns.clear();
    } else if scratch.spawns.len() > credit.frames {
        scratch
            .spawns
            .select_nth_unstable_by(credit.frames, nearest);
        scratch.spawns.truncate(credit.frames);
    }
    scratch.spawns.sort_unstable_by(nearest);
    for &(id, _) in &scratch.spawns {
        let entity = snapshot.entities.get(id).expect("live spawn");
        let position = entity_spawn_position(entity);
        // A passenger's pairing also carries its vehicle's passengers (vanilla
        // `ServerEntity.sendPairingData`, the `isPassenger` branch, sends it
        // before the leash; the two packets are independent): the client skips
        // passenger ids it does not know yet, so a rider that comes into view
        // after its vehicle is mounted only by this packet. A vehicle that is
        // no longer tracked has no passengers to announce.
        let vehicle = viewer.vehicles.get(&id).and_then(|vehicle| {
            let tracked = viewer.tracked_entities.get(vehicle)?;
            Some(mc_protocol::packets::play::ClientboundSetPassengers {
                vehicle_id: vehicle.0,
                passenger_ids: tracked
                    .appearance
                    .passengers
                    .iter()
                    .map(|id| id.0)
                    .collect(),
            })
        });
        // A respawn after lost frames removes the client's copy first, and
        // seats a seated villager again (the removal drops its link).
        let repair = is_undelivered(viewer, id);
        let seat = viewer
            .seats
            .get(&id)
            .filter(|_| repair)
            .map(|&(seat, _)| seat);
        let introduced_from = out.len();
        let spawned = try_push(out, credit, |out| {
            if repair {
                push_best_effort(
                    out,
                    conn,
                    &RemoveEntities {
                        entity_ids: vec![id.0],
                    },
                );
            }
            push_best_effort(
                out,
                conn,
                &AddEntity {
                    entity_id: entity.id.0,
                    uuid: entity.uuid,
                    entity_type_id: entity.type_id as i32,
                    x: position[0],
                    y: position[1],
                    z: position[2],
                    movement: per_tick(entity.velocity),
                    pitch: entity.pitch,
                    yaw: entity.yaw,
                    head_yaw: entity.head_yaw,
                    data: entity.appearance.spawn_data,
                },
            );
            push_appearance(out, conn, entity, None);
            if let Some(link) = &vehicle {
                push_best_effort(out, conn, link);
            }
            if let Some(seat) = seat {
                push_best_effort(
                    out,
                    conn,
                    &mc_protocol::packets::play::ClientboundSetPassengers {
                        vehicle_id: seat,
                        passenger_ids: vec![id.0],
                    },
                );
            }
        });
        if !spawned {
            break;
        }
        tally.spawned(entity.type_id, &out[introduced_from..]);
        if repair {
            viewer.untrack(id);
            interest.removed(id);
        }
        viewer.track(entity);
        viewer.world_entities.push(id);
        interest.spawned(entity.id);
        spawns_sent += 1;
    }

    // Changed rows and retained delivery/cadence debt, most overdue and
    // nearest first while the credit lasts; canonical chunk/id order breaks
    // ties. An update left without credit keeps its debt and gains priority
    // every publish until sent.
    scratch.updates.clear();
    for &id in &scratch.visible {
        if id == viewer.entity_id {
            continue;
        }
        let Some(tracked) = viewer.tracked_entities.get(&id) else {
            continue;
        };
        let entity = snapshot.entities.get(id).expect("visible entity");
        let item = tally.visit(entity.type_id);
        let stale = !Arc::ptr_eq(&tracked.appearance, &entity.appearance);
        let due = interest.ready(id) || entity_movement_due(entity, snapshot.tick, previous_tick);
        let movement = if due {
            index.moves.movement(entity, tracked.baseline)
        } else {
            None
        };
        let head = due
            .then(|| pack_degrees(entity.head_yaw))
            .filter(|&packed| tracked.head_yaw != packed);
        let resend = tracked.resend;
        if !stale && !resend && movement.is_none() && head.is_none() {
            interest.completed(entity, tracked, snapshot.tick, due, false);
            continue;
        }
        scratch.updates.push(EntityUpdate {
            order: scratch.updates.len(),
            id,
            priority: update_priority(
                round.saturating_sub(tracked.updated),
                entity.position,
                viewer.position,
            ),
            stale,
            resend,
            item,
            due,
            movement,
            head,
        });
    }
    scratch.updates.sort_unstable_by(|a, b| {
        b.priority
            .total_cmp(&a.priority)
            .then(a.order.cmp(&b.order))
    });
    let pending_updates = scratch.updates.len();
    let mut updates_sent = 0;
    for update in scratch.updates.drain(..) {
        let tracked = viewer
            .tracked_entities
            .get_mut(&update.id)
            .expect("tracked update");
        let entity = snapshot.entities.get(update.id).expect("visible entity");
        let update_start = out.len();
        // Against an appearance that differs in every field, a resend carries
        // the whole state, passengers included.
        let resend_baseline;
        let baseline = if update.resend {
            resend_baseline = EntityAppearance {
                passengers: vec![EntityId(i32::MIN)],
                ..Default::default()
            };
            &resend_baseline
        } else {
            tracked.appearance.as_ref()
        };
        let sent = try_push(out, credit, |out| {
            if update.stale || update.resend {
                push_appearance(out, conn, entity, Some(baseline));
            }
            if let Some(packet) = &update.movement {
                out.push(PlannedPacket {
                    connection: conn,
                    id: packet.id,
                    reliable: false,
                    data: packet.data.clone(),
                    chunk: None,
                    chunk_dimension: dimension,
                });
            }
            if update.head.is_some() {
                let (id, data) = index.moves.head(entity);
                out.push(PlannedPacket {
                    connection: conn,
                    id,
                    reliable: false,
                    data,
                    chunk: None,
                    chunk_dimension: dimension,
                });
            }
        });
        if !sent {
            break;
        }
        if update.item {
            tally.updated(
                &out[update_start..],
                update.movement.as_ref().map(|packet| packet.data.len()),
            );
        }
        if out.len() > update_start {
            viewer.world_entities.push(update.id);
        }
        if update.stale || update.resend {
            relink_passengers(
                &mut viewer.vehicles,
                update.id,
                &tracked.appearance.passengers,
                &entity.appearance.passengers,
            );
            tracked.appearance = Arc::clone(&entity.appearance);
            tracked.resend = false;
        }
        let moved = update.movement.is_some() || update.head.is_some();
        if let Some(packet) = update.movement {
            tracked.baseline = packet.next;
        }
        if let Some(head) = update.head {
            tracked.head_yaw = head;
        }
        tracked.updated = round;
        interest.completed(entity, tracked, snapshot.tick, update.due, moved);
        updates_sent += 1;
    }
    viewer.deferred_updates =
        removals_deferred + (spawn_candidates - spawns_sent) + (pending_updates - updates_sent);
    viewer.entity_interest = interest;
}
