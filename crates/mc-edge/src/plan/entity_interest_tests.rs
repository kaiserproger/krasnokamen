use super::*;

fn type_id_of(name: &str) -> u32 {
    mc_data::entity_types::krasnokamen_required_entity_types()
        .id_of(&mc_data::Identifier::parse(name).unwrap())
        .unwrap()
}

fn plan_entities_for(
    out: &TickOutput,
    viewer: &mut ViewerState,
    credit: usize,
) -> Vec<PlannedPacket> {
    let index = TickPlanIndex::with_previous(out, viewer.previous_entities());
    let mut scratch = PlanScratch::default();
    let mut packets = Vec::new();
    let mut credit = WorldCredit::frames(credit);
    plan_entities(
        &out.snapshot,
        viewer,
        4,
        &mut scratch,
        &index,
        &mut packets,
        &mut credit,
    );
    packets
}

#[test]
fn unchanged_revision_has_no_entity_work_after_delivery() {
    let mut viewer = viewer_at(1, [8.0, 64.0, 8.0]);
    viewer.loaded_chunks.insert((0, 0), 0);
    let out = output(
        vec![],
        (10..110).map(|id| entity(id, [9.0, 64.0, 8.0])).collect(),
        ChangeSet::default(),
    );
    assert!(
        plan_entities_for(&out, &mut viewer, usize::MAX)
            .iter()
            .any(|packet| packet.id == AddEntity::ID)
    );
    let index = TickPlanIndex::with_previous(&out, viewer.previous_entities());
    let mut interest = std::mem::take(&mut viewer.entity_interest);
    let mut scratch = PlanScratch::default();
    interest.prepare(
        &out.snapshot,
        &mut viewer,
        4,
        &index,
        &mut scratch,
        usize::MAX,
    );
    assert!(
        scratch.visible.is_empty(),
        "quiet rows must not be visited again"
    );
    assert!(scratch.spawns.is_empty());
    assert!(scratch.removed.is_empty());
    assert_eq!(viewer.tracked_entities.len(), 100);
}

#[test]
fn late_player_profile_respawns_previously_tracked_stationary_player() {
    use mc_core::boundary::PlayerChange;
    use mc_protocol::packets::play::{PlayerInfoUpdate, RemoveEntities};

    let mut player = entity(10, [9.0, 64.0, 8.0]);
    player.type_id = type_id_of("minecraft:player");
    let mut viewer = viewer_at(1, [8.0, 64.0, 8.0]);
    viewer.loaded_chunks.insert((0, 0), 0);
    let out = output(vec![], vec![player.clone()], ChangeSet::default());
    assert!(
        plan_entities_for(&out, &mut viewer, 32)
            .iter()
            .any(|p| p.id == AddEntity::ID)
    );
    // Seed the incremental interest index while the entity is already tracked.
    plan_entities_for(&out, &mut viewer, 32);
    let roster = PlayerChange::Roster {
        players: Arc::from([mc_core::dimensions::RosterPlayer {
            connection: ConnectionId(10),
            entity: player.id,
            uuid: player.uuid,
            name: "Existing".into(),
            game_mode: 0,
        }]),
        removed: Arc::from([]),
    };
    let packets = crate::plan::plan_player_changes(&[roster], &mut viewer);
    assert!(packets.iter().any(|p| p.id == RemoveEntities::ID));
    assert!(packets.iter().any(|p| p.id == PlayerInfoUpdate::ID));
    assert!(
        plan_entities_for(&out, &mut viewer, 32)
            .iter()
            .any(|p| p.id == AddEntity::ID),
        "player must be spawned again after its profile arrives"
    );
}

#[test]
fn movement_waiting_for_its_phase_survives_an_unchanged_snapshot() {
    let mut mob = entity(10, [9.0, 64.0, 8.0]);
    mob.type_id = type_id_of("minecraft:cow");
    let mut viewer = viewer_at(1, [8.0, 64.0, 8.0]);
    viewer.loaded_chunks.insert((0, 0), 0);
    let mut baseline = mob.clone();
    baseline.position[0] -= 1.0;
    viewer
        .tracked_entities
        .insert(mob.id, TrackedEntity::new(&baseline, 0, 0));
    let mut out = output(vec![], vec![mob.clone()], ChangeSet::default());
    let interval = u64::from(
        mc_data::entity_types::entity_type_contract_26_1_2_by_protocol_id(mob.type_id)
            .unwrap()
            .update_interval,
    );
    Arc::get_mut(&mut out.snapshot).unwrap().tick = (0..interval)
        .find(|&tick| !entity_movement_due(&mob, tick, None))
        .unwrap();
    assert!(plan_entities_for(&out, &mut viewer, 32).is_empty());
    for _ in 0..interval {
        Arc::get_mut(&mut out.snapshot).unwrap().tick += 1;
        let packets = plan_entities_for(&out, &mut viewer, 32);
        if packets.iter().any(|packet| packet.id == MoveEntityPos::ID) {
            assert_eq!(viewer.tracked_entities[&mob.id].baseline.0, mob.position);
            return;
        }
    }
    panic!("stopped entity's movement debt was lost");
}

#[test]
fn credit_blocked_movement_is_retained_without_new_entity_changes() {
    let mut mob = entity(10, [9.0, 64.0, 8.0]);
    mob.type_id = type_id_of("minecraft:cow");
    let mut viewer = viewer_at(1, [8.0, 64.0, 8.0]);
    viewer.loaded_chunks.insert((0, 0), 0);
    let mut baseline = mob.clone();
    baseline.position[0] -= 1.0;
    viewer
        .tracked_entities
        .insert(mob.id, TrackedEntity::new(&baseline, 0, 0));
    let mut out = output(vec![], vec![mob.clone()], ChangeSet::default());
    while !entity_movement_due(&mob, out.snapshot.tick, None) {
        Arc::get_mut(&mut out.snapshot).unwrap().tick += 1;
    }
    assert!(plan_entities_for(&out, &mut viewer, 0).is_empty());
    assert_eq!(
        viewer.tracked_entities[&mob.id].baseline.0,
        baseline.position
    );
    Arc::get_mut(&mut out.snapshot).unwrap().tick += 3;
    assert!(
        plan_entities_for(&out, &mut viewer, 32)
            .iter()
            .any(|packet| packet.id == MoveEntityPos::ID)
    );
    assert_eq!(viewer.tracked_entities[&mob.id].baseline.0, mob.position);
}

#[test]
fn walking_inside_negative_chunk_reconciles_exact_enter_leave_strips() {
    let mut viewer = viewer_at(1, [-0.25, 64.0, -0.25]);
    for x in -7..=5 {
        for z in -7..=5 {
            viewer.loaded_chunks.insert((x, z), 0);
        }
    }
    let positions = [
        [95.8, 64.0, 8.0],
        [-96.2, 64.0, 8.0],
        [8.0, 64.0, 95.8],
        [8.0, 64.0, -96.2],
        [63.8, 64.0, 8.0],
        [-64.2, 64.0, 8.0],
        [8.0, 64.0, 63.8],
        [8.0, 64.0, -64.2],
        [8.0, 64.0, 8.0],
    ];
    let mut rows = Vec::new();
    for (i, position) in positions.into_iter().enumerate() {
        let mut row = entity(10 + i as i32, position);
        row.type_id = type_id_of(if i < 4 {
            "minecraft:cow"
        } else {
            "minecraft:snowball"
        });
        rows.push(row);
    }
    let out = output(vec![], rows, ChangeSet::default());
    for position in [
        [-0.25, 64.0, -0.25],
        [-0.1, 64.0, -0.25],
        [-0.1, 64.0, -0.1],
        [-0.25, 64.0, -0.25],
    ] {
        viewer.position = position;
        let index = TickPlanIndex::with_previous(&out, viewer.previous_entities());
        let mut scratch = PlanScratch::default();
        let mut packets = Vec::new();
        let mut credit = WorldCredit::frames(usize::MAX);
        plan_entities(
            &out.snapshot,
            &mut viewer,
            6,
            &mut scratch,
            &index,
            &mut packets,
            &mut credit,
        );
        let expected: HashSet<_> = out
            .snapshot
            .entities
            .iter()
            .filter(|row| entity_trackable(row, &viewer, 6))
            .map(|row| row.id)
            .collect();
        assert_eq!(
            viewer
                .tracked_entities
                .keys()
                .copied()
                .collect::<HashSet<_>>(),
            expected,
            "viewer {position:?}"
        );
    }
}

/// Updates of equal priority (same distance, same wait) drain one per
/// publish in canonical chunk/id order.
#[test]
fn equal_priority_delivery_debt_drains_in_canonical_order() {
    let mut viewer = viewer_at(1, [8.0, 64.0, 8.0]);
    viewer.loaded_chunks.insert((0, 0), 0);
    let mut rows = Vec::new();
    for id in [10, 13, 16] {
        let mut mob = entity(id, [9.0, 64.0, 8.0]);
        mob.type_id = type_id_of("minecraft:cow");
        let mut baseline = mob.clone();
        baseline.position[0] -= 1.0;
        viewer
            .tracked_entities
            .insert(mob.id, TrackedEntity::new(&baseline, 0, 0));
        rows.push(mob);
    }
    let mut out = output(vec![], rows, ChangeSet::default());
    while !entity_movement_due(
        out.snapshot.entities.get(EntityId(10)).unwrap(),
        out.snapshot.tick,
        None,
    ) {
        Arc::get_mut(&mut out.snapshot).unwrap().tick += 1;
    }
    assert_eq!(drain_one_move_per_publish(&out, &mut viewer), [10, 13, 16]);
}

/// Under credit for one update per publish, the nearest pending update
/// goes first, then the next nearest.
#[test]
fn limited_credit_sends_the_nearest_pending_update_first() {
    let mut viewer = viewer_at(1, [8.0, 64.0, 8.0]);
    viewer.loaded_chunks.insert((0, 0), 0);
    let mut rows = Vec::new();
    for (id, x) in [(10, 15.0), (13, 12.0), (16, 9.0)] {
        let mut mob = entity(id, [x, 64.0, 8.0]);
        mob.type_id = type_id_of("minecraft:cow");
        let mut baseline = mob.clone();
        baseline.position[2] -= 0.5;
        viewer
            .tracked_entities
            .insert(mob.id, TrackedEntity::new(&baseline, 0, 0));
        rows.push(mob);
    }
    let mut out = output(vec![], rows, ChangeSet::default());
    while !entity_movement_due(
        out.snapshot.entities.get(EntityId(10)).unwrap(),
        out.snapshot.tick,
        None,
    ) {
        Arc::get_mut(&mut out.snapshot).unwrap().tick += 1;
    }
    assert_eq!(drain_one_move_per_publish(&out, &mut viewer), [16, 13, 10]);
}

/// Byte credit is soft: any remaining byte admits one more whole update
/// group, and nothing goes out once the bytes are spent. Updates that do
/// not fit are counted as deferred.
#[test]
fn byte_credit_admits_whole_groups_until_spent() {
    let mut viewer = viewer_at(1, [8.0, 64.0, 8.0]);
    viewer.loaded_chunks.insert((0, 0), 0);
    // Interval-1 rows: due on every publish, nearest first.
    let rows: Vec<_> = [(10, 9.0), (11, 10.0), (12, 11.0)]
        .into_iter()
        .map(|(id, x)| entity(id, [x, 64.0, 8.0]))
        .collect();
    for row in &rows {
        let mut baseline = row.clone();
        baseline.position[2] -= 0.25;
        viewer
            .tracked_entities
            .insert(row.id, TrackedEntity::new(&baseline, 0, 0));
    }
    let out = output(vec![], rows, ChangeSet::default());
    let plan = |viewer: &mut ViewerState, bytes: usize| {
        let index = TickPlanIndex::with_previous(&out, viewer.previous_entities());
        let mut packets = Vec::new();
        plan_entities(
            &out.snapshot,
            viewer,
            4,
            &mut PlanScratch::default(),
            &index,
            &mut packets,
            &mut WorldCredit { frames: 64, bytes },
        );
        packets
            .iter()
            .map(|packet| {
                let id = MoveEntityPos::decode(&mut &body_of(packet)[..])
                    .unwrap()
                    .entity_id;
                (id, packet.data.len())
            })
            .collect::<Vec<_>>()
    };
    assert!(plan(&mut viewer, 0).is_empty(), "spent bytes send nothing");
    assert_eq!(viewer.deferred_updates, 3);
    let first = plan(&mut viewer, 1);
    assert_eq!(first.len(), 1, "one byte left admits one whole group");
    assert_eq!(first[0].0, 10);
    assert_eq!(viewer.deferred_updates, 2);
    let rest = plan(&mut viewer, first[0].1 + 1);
    assert_eq!(
        rest.iter().map(|&(id, _)| id).collect::<Vec<_>>(),
        [11, 12],
        "a group's bytes are charged; the last byte admits one more"
    );
    assert_eq!(viewer.deferred_updates, 0);
    assert!(plan(&mut viewer, usize::MAX).is_empty(), "nothing left");
}

/// Spawns the credit cuts count as deferred until they go out.
#[test]
fn spawns_left_without_credit_count_as_deferred() {
    let held = || {
        let mut viewer = viewer_at(1, [8.0, 64.0, 8.0]);
        viewer.loaded_chunks.insert((0, 0), 0);
        viewer
    };
    let one = output(
        vec![],
        vec![entity(10, [9.0, 64.0, 8.0])],
        ChangeSet::default(),
    );
    let group = plan_entities_for(&one, &mut held(), usize::MAX).len();
    let two = output(
        vec![],
        vec![entity(10, [9.0, 64.0, 8.0]), entity(11, [10.0, 64.0, 8.0])],
        ChangeSet::default(),
    );
    let mut viewer = held();
    let spawns = |packets: Vec<PlannedPacket>| {
        packets
            .iter()
            .filter(|packet| packet.id == AddEntity::ID)
            .count()
    };
    let sent = spawns(plan_entities_for(&two, &mut viewer, group));
    assert_eq!((sent, viewer.deferred_updates), (1, 1));
    let sent = spawns(plan_entities_for(&two, &mut viewer, usize::MAX));
    assert_eq!((sent, viewer.deferred_updates), (1, 0));
}

/// With no credit at all, the spawn the plan holds back and the removal it
/// cannot send still count as deferred, on a rebuilt interest window and on
/// an incremental one; both clear once credit returns.
#[test]
fn zero_credit_counts_held_back_spawns_and_removals() {
    let mut viewer = viewer_at(1, [8.0, 64.0, 8.0]);
    viewer.loaded_chunks.insert((0, 0), 0);
    // Tracked by the client, gone from the snapshot: a pending removal.
    viewer.tracked_entities.insert(
        EntityId(20),
        TrackedEntity::new(&entity(20, [10.0, 64.0, 8.0]), 0, 0),
    );
    let out = output(
        vec![],
        vec![entity(10, [9.0, 64.0, 8.0])],
        ChangeSet::default(),
    );
    for window in ["rebuilt", "incremental"] {
        assert!(
            plan_entities_for(&out, &mut viewer, 0).is_empty(),
            "{window}"
        );
        assert_eq!(
            viewer.deferred_updates, 2,
            "{window}: one spawn, one removal"
        );
    }
    let packets = plan_entities_for(&out, &mut viewer, usize::MAX);
    assert!(packets.iter().any(|packet| packet.id == AddEntity::ID));
    assert!(packets.iter().any(|packet| packet.id == RemoveEntities::ID));
    assert_eq!(viewer.deferred_updates, 0);
}

/// A near unit that moves every publish cannot starve a far one: waiting
/// raises the far update's priority until it wins a publish, while the
/// near unit keeps most of the credit.
#[test]
fn far_updates_wait_longer_but_never_starve_under_constant_pressure() {
    let mut viewer = viewer_at(1, [8.0, 64.0, 8.0]);
    viewer.loaded_chunks.insert((0, 0), 0);
    viewer.loaded_chunks.insert((4, 0), 0);
    // Interval-1 rows: both are due on every publish.
    let mut near = entity(10, [9.0, 64.0, 8.0]);
    let mut far = entity(11, [68.0, 64.0, 8.0]);
    for row in [&near, &far] {
        let mut baseline = row.clone();
        baseline.position[2] -= 0.25;
        viewer
            .tracked_entities
            .insert(row.id, TrackedEntity::new(&baseline, 0, 0));
    }
    let mut sent = Vec::new();
    for tick in 1..=12 {
        let mut out = output(
            vec![],
            vec![near.clone(), far.clone()],
            ChangeSet::default(),
        );
        Arc::get_mut(&mut out.snapshot).unwrap().tick = tick;
        let packets = plan_entities_for(&out, &mut viewer, 1);
        assert_eq!(packets.len(), 1, "tick {tick}: one update fits");
        sent.push(
            MoveEntityPos::decode(&mut &body_of(&packets[0])[..])
                .unwrap()
                .entity_id,
        );
        near.position[2] += 0.25;
        far.position[2] += 0.25;
    }
    assert_eq!(sent[0], 10, "nearest first");
    let far_sends: Vec<_> = (0..sent.len()).filter(|&i| sent[i] == 11).collect();
    assert!(far_sends.len() >= 2, "far unit starved: {sent:?}");
    assert!(
        far_sends.windows(2).all(|pair| pair[1] - pair[0] <= 6) && far_sends[0] < 6,
        "far unit waited more than six publishes: {sent:?}"
    );
    assert!(
        sent.len() - far_sends.len() >= 2 * far_sends.len(),
        "{sent:?}"
    );
}

/// Arrows track at 4 chunks for players; a recorder tracks them to its
/// whole view distance. The override is scoped to the recorder viewer, and
/// a type vanilla never sends (marker, range 0) stays unsent for both.
#[test]
fn recorder_tracks_arrows_past_their_range_and_players_do_not() {
    let of_type = |name, id, x| EntityView {
        type_id: type_id_of(name),
        ..entity(id, [x, 64.0, 8.0])
    };
    // 40 and 100 blocks east of the viewer: inside and past 4 chunks; the
    // marker one block away.
    let out = output(
        vec![],
        vec![
            of_type("minecraft:arrow", 10, 48.0),
            of_type("minecraft:arrow", 11, 108.0),
            of_type("minecraft:marker", 12, 9.0),
        ],
        ChangeSet::default(),
    );
    let spawned = |recorder: bool| {
        let mut viewer = viewer_at(1, [8.0, 64.0, 8.0]);
        viewer.recorder = recorder;
        viewer.loaded_chunks.insert((0, 0), 0);
        viewer.loaded_chunks.insert((3, 0), 0);
        viewer.loaded_chunks.insert((6, 0), 0);
        let index = TickPlanIndex::with_previous(&out, viewer.previous_entities());
        let mut packets = Vec::new();
        plan_entities(
            &out.snapshot,
            &mut viewer,
            8,
            &mut PlanScratch::default(),
            &index,
            &mut packets,
            &mut WorldCredit::frames(64),
        );
        let mut ids: Vec<i32> = packets
            .iter()
            .filter(|packet| packet.id == AddEntity::ID)
            .map(|packet| AddEntity::decode(&mut body_of(packet)).unwrap().entity_id)
            .collect();
        ids.sort_unstable();
        ids
    };
    assert_eq!(spawned(true), [10, 11]);
    assert_eq!(spawned(false), [10]);
}

/// Plan the same snapshot with credit for one frame until no move is left;
/// returns the moved entity ids in send order.
fn drain_one_move_per_publish(out: &TickOutput, viewer: &mut ViewerState) -> Vec<i32> {
    let mut sent = Vec::new();
    loop {
        let packets = plan_entities_for(out, viewer, 1);
        let Some(packet) = packets.first() else {
            return sent;
        };
        assert_eq!(packets.len(), 1);
        sent.push(
            MoveEntityPos::decode(&mut &body_of(packet)[..])
                .unwrap()
                .entity_id,
        );
        assert!(sent.len() <= 16, "debt never drains: {sent:?}");
    }
}

#[test]
fn changed_motion_preserves_deadline_appearance_and_credit_cadence() {
    use mc_core::blueprint::MovementClass;
    use mc_core::entities::{ColdFields, EntityKind, EntityStore};

    let mut store = EntityStore::default();
    let id = EntityId(10);
    let type_id = type_id_of("minecraft:cow");
    let slot = store.spawn(
        id,
        uuid::Uuid::from_u128(10),
        EntityKind::Mob { type_id },
        ColdFields {
            pos: [9.0, 64.0, 8.0],
            vel: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            ground: true,
            health: 20.0,
            breed_cooldown: 0,
            pickup_delay: 0,
            item: None,
            appearance: Arc::new(EntityAppearance::default()),
            movement: MovementClass::Walker,
            half_width: 0.3,
            height: 1.8,
            speed: 0.1,
            attack_damage: 1.0,
            sight_range: 16.0,
            attack_reach: 1.0,
        },
    );
    let mut viewer = viewer_at(1, [8.0, 64.0, 8.0]);
    viewer.loaded_chunks.insert((0, 0), 0);
    let mut out = output(vec![], vec![], ChangeSet::default());
    let snapshot = Arc::get_mut(&mut out.snapshot).unwrap();
    snapshot.entities = store.views();
    snapshot.tick = (0..10)
        .find(|&tick| entity_movement_due(snapshot.entities.get(id).unwrap(), tick, None))
        .unwrap();
    assert!(!plan_entities_for(&out, &mut viewer, 32).is_empty());
    assert!(plan_entities_for(&out, &mut viewer, 32).is_empty());

    store.set_pos(slot, [9.25, 64.0, 8.0]);
    let snapshot = Arc::get_mut(&mut out.snapshot).unwrap();
    snapshot.tick += 1;
    snapshot.entities = store.views();
    assert!(!entity_movement_due(
        snapshot.entities.get(id).unwrap(),
        snapshot.tick,
        None
    ));
    let index = TickPlanIndex::with_previous(&out, viewer.previous_entities());
    let mut interest = std::mem::take(&mut viewer.entity_interest);
    let mut scratch = PlanScratch::default();
    interest.prepare(&out.snapshot, &mut viewer, 4, &index, &mut scratch, 32);
    assert_eq!(
        scratch.visible,
        [id],
        "deadline arming must follow canonical delivery traversal"
    );
    viewer.entity_interest = interest;
    assert!(plan_entities_for(&out, &mut viewer, 32).is_empty());

    // Appearance still bypasses the movement cadence, including when the
    // same entity already has a future movement deadline.
    Arc::make_mut(&mut store.row_mut(slot).appearance).shared_flags ^= 1;
    Arc::get_mut(&mut out.snapshot).unwrap().entities = store.views();
    let packets = plan_entities_for(&out, &mut viewer, 32);
    assert!(
        packets
            .iter()
            .any(|packet| packet.id == ClientboundSetEntityData::ID)
    );
    assert!(!packets.iter().any(|packet| packet.id == MoveEntityPos::ID));

    for _ in 0..10 {
        Arc::get_mut(&mut out.snapshot).unwrap().tick += 1;
        let packets = plan_entities_for(&out, &mut viewer, 32);
        if packets.iter().any(|packet| packet.id == MoveEntityPos::ID) {
            assert_eq!(viewer.tracked_entities[&id].baseline.0, [9.25, 64.0, 8.0]);
            break;
        }
    }
    assert_eq!(
        viewer.tracked_entities[&id].baseline.0,
        [9.25, 64.0, 8.0],
        "queued motion must be sent even if the entity stops changing"
    );

    // A newly changed row first seen on its due tick does not already
    // have a queued deadline. Credit rejection must not invent a ready
    // deadline that sends it on the next, non-due tick.
    let interval = u64::from(
        mc_data::entity_types::entity_type_contract_26_1_2_by_protocol_id(type_id)
            .unwrap()
            .update_interval,
    );
    store.set_pos(slot, [9.5, 64.0, 8.0]);
    let snapshot = Arc::get_mut(&mut out.snapshot).unwrap();
    snapshot.tick += interval;
    snapshot.entities = store.views();
    assert!(entity_movement_due(
        snapshot.entities.get(id).unwrap(),
        snapshot.tick,
        None
    ));
    assert!(plan_entities_for(&out, &mut viewer, 0).is_empty());
    store.set_pos(slot, [9.75, 64.0, 8.0]);
    let snapshot = Arc::get_mut(&mut out.snapshot).unwrap();
    snapshot.tick += 1;
    snapshot.entities = store.views();
    assert!(plan_entities_for(&out, &mut viewer, 32).is_empty());
    assert_eq!(viewer.tracked_entities[&id].baseline.0, [9.25, 64.0, 8.0]);
}

/// A cow that moves every tick is visited about twice per movement period (the visit that arms its
/// deadline, then the deadline), not every tick, and still receives exactly the moves a per-tick visit
/// would send: one per due tick, against the latest position. Once it stops, it costs one more visit.
#[test]
fn armed_rows_skip_intermediate_ticks_and_deliver_at_the_deadline() {
    use mc_core::blueprint::MovementClass;
    use mc_core::entities::{ColdFields, EntityKind, EntityStore};

    let mut store = EntityStore::default();
    let id = EntityId(10);
    let type_id = type_id_of("minecraft:cow");
    let interval = u64::from(
        mc_data::entity_types::entity_type_contract_26_1_2_by_protocol_id(type_id)
            .unwrap()
            .update_interval,
    );
    let slot = store.spawn(
        id,
        uuid::Uuid::from_u128(10),
        EntityKind::Mob { type_id },
        ColdFields {
            pos: [9.0, 64.0, 8.0],
            vel: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            ground: true,
            health: 20.0,
            breed_cooldown: 0,
            pickup_delay: 0,
            item: None,
            appearance: Arc::new(EntityAppearance::default()),
            movement: MovementClass::Walker,
            half_width: 0.3,
            height: 1.8,
            speed: 0.1,
            attack_damage: 1.0,
            sight_range: 16.0,
            attack_reach: 1.0,
        },
    );
    let mut viewer = viewer_at(1, [8.0, 64.0, 8.0]);
    viewer.loaded_chunks.insert((0, 0), 0);
    let mut out = output(vec![], vec![], ChangeSet::default());
    let snapshot = Arc::get_mut(&mut out.snapshot).unwrap();
    snapshot.entities = store.views();
    snapshot.tick = (0..interval * 2)
        .find(|&tick| entity_movement_due(snapshot.entities.get(id).unwrap(), tick, None))
        .unwrap();
    assert!(
        !plan_entities_for(&out, &mut viewer, 32).is_empty(),
        "spawn"
    );

    let periods = 5;
    let mut visits = 0;
    let mut sent = 0;
    let mut due_ticks = 0;
    let mut x = 9.0;
    for _ in 0..periods * interval {
        x += 0.2;
        store.set_pos(slot, [x, 64.0, 8.0]);
        let snapshot = Arc::get_mut(&mut out.snapshot).unwrap();
        snapshot.tick += 1;
        snapshot.entities = store.views();
        let due = entity_movement_due(snapshot.entities.get(id).unwrap(), snapshot.tick, None);

        let index = TickPlanIndex::with_previous(&out, viewer.previous_entities());
        let mut interest = std::mem::take(&mut viewer.entity_interest);
        let mut scratch = PlanScratch::default();
        interest.prepare(&out.snapshot, &mut viewer, 4, &index, &mut scratch, 32);
        visits += scratch.visible.len();
        viewer.entity_interest = interest;

        let packets = plan_entities_for(&out, &mut viewer, 32);
        let moves = packets
            .iter()
            .filter(|packet| packet.id == MoveEntityPos::ID)
            .count();
        assert_eq!(
            moves,
            usize::from(due),
            "one move per due tick, none between"
        );
        if due {
            due_ticks += 1;
            sent += moves;
            assert_eq!(viewer.tracked_entities[&id].baseline.0, [x, 64.0, 8.0]);
        }
    }
    assert_eq!(sent, due_ticks);
    assert!(
        visits <= periods as usize + 1,
        "{visits} visits in {periods} periods: armed rows must not be visited between deadlines"
    );

    // Stopped: one visit may remain (the deadline armed by the last move), then none.
    let mut after = 0;
    for _ in 0..3 * interval {
        let snapshot = Arc::get_mut(&mut out.snapshot).unwrap();
        snapshot.tick += 1;
        let index = TickPlanIndex::with_previous(&out, viewer.previous_entities());
        let mut interest = std::mem::take(&mut viewer.entity_interest);
        let mut scratch = PlanScratch::default();
        interest.prepare(&out.snapshot, &mut viewer, 4, &index, &mut scratch, 32);
        after += scratch.visible.len();
        viewer.entity_interest = interest;
        assert!(plan_entities_for(&out, &mut viewer, 32).is_empty());
    }
    assert!(after <= 1, "a stopped row was visited {after} times");
}

/// A cow standing in view at `x`, spawned on its due tick: the scene of the armed-row tests.
struct Mover {
    store: mc_core::entities::EntityStore,
    slot: mc_core::entities::EntitySlot,
    out: TickOutput,
    viewer: ViewerState,
    id: EntityId,
    x: f64,
}

impl Mover {
    fn new() -> Self {
        use mc_core::blueprint::MovementClass;
        use mc_core::entities::{ColdFields, EntityKind, EntityStore};

        let mut store = EntityStore::default();
        let id = EntityId(10);
        let type_id = type_id_of("minecraft:cow");
        let slot = store.spawn(
            id,
            uuid::Uuid::from_u128(10),
            EntityKind::Mob { type_id },
            ColdFields {
                pos: [9.0, 64.0, 8.0],
                vel: [0.0; 3],
                yaw: 0.0,
                pitch: 0.0,
                ground: true,
                health: 20.0,
                breed_cooldown: 0,
                pickup_delay: 0,
                item: None,
                appearance: Arc::new(EntityAppearance::default()),
                movement: MovementClass::Walker,
                half_width: 0.3,
                height: 1.8,
                speed: 0.1,
                attack_damage: 1.0,
                sight_range: 16.0,
                attack_reach: 1.0,
            },
        );
        let mut viewer = viewer_at(1, [8.0, 64.0, 8.0]);
        viewer.loaded_chunks.insert((0, 0), 0);
        let mut out = output(vec![], vec![], ChangeSet::default());
        let snapshot = Arc::get_mut(&mut out.snapshot).unwrap();
        snapshot.entities = store.views();
        snapshot.tick = (0..20)
            .find(|&tick| entity_movement_due(snapshot.entities.get(id).unwrap(), tick, None))
            .unwrap();
        let mut mover = Self {
            store,
            slot,
            out,
            viewer,
            id,
            x: 9.0,
        };
        assert!(!plan_entities_for(&mover.out, &mut mover.viewer, 32).is_empty());
        mover
    }

    /// One tick: the cow walks 0.2 blocks (or stands), then the planner runs with `credit`.
    /// Returns whether it sent a move and whether this tick is the cow's due tick.
    fn tick(&mut self, walks: bool, credit: usize) -> (bool, bool) {
        if walks {
            self.x += 0.2;
            self.store.set_pos(self.slot, [self.x, 64.0, 8.0]);
        }
        let snapshot = Arc::get_mut(&mut self.out.snapshot).unwrap();
        snapshot.tick += 1;
        snapshot.entities = self.store.views();
        let due = entity_movement_due(snapshot.entities.get(self.id).unwrap(), snapshot.tick, None);
        let packets = plan_entities_for(&self.out, &mut self.viewer, credit);
        (
            packets.iter().any(|packet| packet.id == MoveEntityPos::ID),
            due,
        )
    }
}

/// A continuous mover whose armed deadline is rejected for credit keeps its move as delivery debt:
/// it goes out on the next tick, as when every tick visited the row, not a whole interval later.
#[test]
fn armed_deadline_rejected_for_credit_sends_on_the_next_tick() {
    let mut mover = Mover::new();
    let interval = 3;
    // The ticks before the deadline arm it and move the cow again.
    for _ in 0..interval - 1 {
        assert_eq!(mover.tick(true, 32), (false, false));
    }
    // Its due tick, credit 0: nothing can be sent.
    assert_eq!(mover.tick(true, 0), (false, true));
    // Credit is back and the tick is not due: the debt is paid now.
    assert_eq!(mover.tick(true, 32), (true, false));
    assert_eq!(
        mover.viewer.tracked_entities[&mover.id].baseline.0,
        [mover.x, 64.0, 8.0]
    );
}

/// The last motion lands the tick before an armed deadline and nothing follows: the deadline still
/// delivers that final position.
#[test]
fn armed_row_that_stops_before_its_deadline_delivers_its_last_position() {
    let mut mover = Mover::new();
    assert_eq!(mover.tick(true, 32), (false, false));
    assert_eq!(mover.tick(true, 32), (false, false));
    // Deadline tick, the cow no longer moves.
    assert_eq!(mover.tick(false, 32), (true, true));
    assert_eq!(
        mover.viewer.tracked_entities[&mover.id].baseline.0,
        [mover.x, 64.0, 8.0]
    );
    for _ in 0..6 {
        assert!(!mover.tick(false, 32).0);
    }
}
