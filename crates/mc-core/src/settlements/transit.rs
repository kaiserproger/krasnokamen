//! Settlement migration across unsimulated ground (ADR 0023): emigration to
//! the outside and the durable transit custody of a migrating unit between two
//! settlements of one dimension.
//!
//! A unit (a household or a lone resident) leaves its source with
//! [`households::leave`]. Emigration then removes the bodies for good. Transit
//! removes them into a [`Record`] holding every member's complete native entity
//! record (the WAL entity codec) until the destination anchor is simulated
//! again; then the same entities re-enter at its simulated edge and
//! [`households::join`] admits them. Invariant: at every committed tick each
//! member UUID is in exactly one place, a world entity or one record.
//!
//! Module boundary: state ([`Ledger`], one field of the settlements registry),
//! systems ([`depart`], [`emigrate`], [`tick`]), the `transit/<uuid>` row and
//! `Transit` WAL codec and the recovery check all live here. The host reaches
//! it only through the registry field, the settlements persistence hookups and
//! the settlements tick wiring.
//!
//! Geometry: a bearing is degrees clockwise from north (−Z), so 90° is east
//! (+X). Exit and entry points lie on the edge of the entity-simulated area,
//! independent of the civil settlement radius.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mc_physics::mth;
use mc_world::native::NativeDomainRecord;
use serde::{Deserialize, Serialize};

use super::SettlementId;
use super::households::{self, Departed, Household, Unit};
use crate::boundary::{BlockPos, ChangeSet, EntityId};
use crate::config::CoreError;
use crate::dimensions::Dimension;
use crate::entities::{EntityKind, EntitySlot};
use crate::journal::{JournalEntry, Replay, ReplayEntity};
use crate::tick::TickState;

/// A point closer than this to the anchor is not an exit or entry point.
pub(crate) const MIN_EDGE_DISTANCE: i32 = 16;
/// Search limit along a bearing; far beyond any simulation distance.
const MAX_EDGE_DISTANCE: i32 = 1024;
/// Every living member stands within this horizontal distance of the exit.
pub(crate) const EXIT_REACH: f64 = 2.0;
/// Abstract travel time per block from the exit to the destination anchor.
pub(crate) const TICKS_PER_BLOCK: u64 = 10;
/// Entry searches/restores admitted per tick, including refused attempts.
const MAX_ARRIVALS_PER_TICK: usize = 4;
const ARRIVAL_RETRY_TICKS: u64 = 100;
/// Villager body used to judge a standable surface cell.
const BODY_HALF_WIDTH: f64 = 0.3;
const BODY_HEIGHT: f64 = 1.95;
/// Native row version of the transit codec.
const ROW_VERSION: u32 = 1;

/// Identity of one transit record: its lowest member UUID. A UUID is in at
/// most one place, so no second sequence exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct TransitId(pub(crate) uuid::Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Status {
    Travelling,
    /// The arrival time passed while the destination anchor was not simulated.
    AwaitingEntry,
}

/// Durable custody of one migrating unit. None of `members` is a world entity
/// while the record exists.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Record {
    pub(crate) source: SettlementId,
    pub(crate) destination: SettlementId,
    pub(crate) destination_anchor: [i32; 3],
    /// Game time of the departure tick and of the earliest arrival.
    pub(crate) departed: u64,
    pub(crate) arrives: u64,
    /// From the source towards the destination.
    pub(crate) bearing: f32,
    pub(crate) exit: [i32; 3],
    pub(crate) status: Status,
    /// The household record as `leave` removed it; none for a lone resident.
    pub(crate) household: Option<Household>,
    /// Complete native entity records, ascending by UUID.
    pub(crate) members: Vec<ReplayEntity>,
    /// Failed entry attempts wait in game time; never part of custody/WAL.
    #[serde(skip)]
    retry_at: u64,
}

/// One dimension's transit records with their touched-only WAL images.
#[derive(Debug, Default)]
pub(crate) struct Ledger {
    records: BTreeMap<uuid::Uuid, Record>,
    touched: BTreeSet<uuid::Uuid>,
    #[cfg(test)]
    entry_attempts: usize,
}

/// One arrival that joined its destination, returned by [`tick`] for the
/// migration policy of the same tick. Never persisted: the joined bodies and
/// household are the durable outcome.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Arrival {
    pub(crate) source: SettlementId,
    pub(crate) destination: SettlementId,
    /// The arriving household's id; `None` for a lone resident.
    pub(crate) household: Option<String>,
    /// Member UUIDs, ascending.
    pub(crate) members: Vec<uuid::Uuid>,
}

impl Ledger {
    fn put(&mut self, id: uuid::Uuid, record: Record) {
        self.records.insert(id, record);
        self.touched.insert(id);
    }

    fn remove(&mut self, id: uuid::Uuid) {
        self.records.remove(&id);
        self.touched.insert(id);
    }

    pub(crate) fn get(&self, id: TransitId) -> Option<&Record> {
        self.records.get(&id.0)
    }
}

/// `settlement info` row for one record touching a settlement.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TransitSummary {
    pub(crate) id: TransitId,
    pub(crate) source: SettlementId,
    pub(crate) destination: SettlementId,
    pub(crate) household: Option<String>,
    pub(crate) members: usize,
    pub(crate) departed: u64,
    pub(crate) arrives: u64,
    pub(crate) status: Status,
}

fn game_time(state: &TickState) -> u64 {
    state.tick_control.game_ticks(state.tick)
}

fn worlds(state: &TickState) -> impl Iterator<Item = &TickState> {
    std::iter::once(state).chain(state.other_dimensions.values().map(|world| &**world))
}

/// Bearing from `from` to `to` in this module's convention.
pub(crate) fn bearing(from: [i32; 3], to: [i32; 3]) -> f32 {
    let dx = f64::from(to[0] - from[0]);
    let dz = f64::from(to[2] - from[2]);
    mth::atan2(dx, -dz).to_degrees().rem_euclid(360.0) as f32
}

/// The farthest standable surface cell on `bearing_deg` from `anchor` whose
/// column is still entity-simulated, walking outward from the anchor and at
/// least [`MIN_EDGE_DISTANCE`] away from it. The civil radius plays no part:
/// with an ordinary simulation distance the point often lies inside it.
/// Reaching it is the outbound walk's job; an unreachable point fails there.
pub(crate) fn edge_point(
    state: &TickState,
    anchor: [i32; 3],
    bearing_deg: f32,
) -> Option<BlockPos> {
    let radians = f64::from(bearing_deg).to_radians();
    let (dx, dz) = (f64::from(mth::sin(radians)), -f64::from(mth::cos(radians)));
    let mut best = None;
    let mut previous = None;
    for distance in 0..=MAX_EDGE_DISTANCE {
        let column = (
            anchor[0] + (f64::from(distance) * dx).round() as i32,
            anchor[2] + (f64::from(distance) * dz).round() as i32,
        );
        if previous == Some(column) {
            continue;
        }
        previous = Some(column);
        if !state
            .world
            .is_active(column.0.div_euclid(16), column.1.div_euclid(16))
        {
            break;
        }
        let horizontal = mth::length2(
            f64::from(column.0 - anchor[0]),
            f64::from(column.1 - anchor[2]),
        );
        if horizontal < f64::from(MIN_EDGE_DISTANCE) {
            continue;
        }
        if let Some(cell) = surface(state, column.0, column.1) {
            best = Some(cell);
        }
    }
    best
}

/// Highest standable cell of one resident column.
fn surface(state: &TickState, x: i32, z: i32) -> Option<BlockPos> {
    let geometry = state.world.geometry();
    let air = state.registry.states.air;
    for y in (geometry.min_y() + 1..geometry.max_y() - 1).rev() {
        if state.world.block_at(x, y, z) != Some(air)
            || state
                .world
                .block_at(x, y - 1, z)
                .is_none_or(|below| below == air)
        {
            continue;
        }
        let cell = BlockPos { x, y, z };
        if super::allocator::standing(
            &state.world,
            &state.registry,
            cell,
            BODY_HALF_WIDTH,
            BODY_HEIGHT,
        )
        .is_some()
        {
            return Some(cell);
        }
    }
    None
}

/// A bounded nearest-foot-cell search when the return bearing has no shore.
fn anchor_entry(state: &TickState, anchor: [i32; 3]) -> Option<BlockPos> {
    let mut best = None;
    for dy in [0, 1, -1, 2, -2] {
        for dx in -4..=4 {
            for dz in -4..=4 {
                let distance = dx * dx + dy * dy + dz * dz;
                if best
                    .as_ref()
                    .is_some_and(|&(previous, _)| previous <= distance)
                {
                    continue;
                }
                let pos = BlockPos {
                    x: anchor[0] + dx,
                    y: anchor[1] + dy,
                    z: anchor[2] + dz,
                };
                if state
                    .world
                    .is_active(pos.x.div_euclid(16), pos.z.div_euclid(16))
                    && super::allocator::standing(
                        &state.world,
                        &state.registry,
                        pos,
                        BODY_HALF_WIDTH,
                        BODY_HEIGHT,
                    )
                    .is_some()
                {
                    if distance == 0 {
                        return Some(pos);
                    }
                    best = Some((distance, pos));
                }
            }
        }
    }
    best.map(|(_, pos)| pos)
}

fn defer_arrival(state: &mut TickState, id: uuid::Uuid, now: u64, reason: &str) {
    let record = state.settlements.transit.records.get_mut(&id).unwrap();
    if record.retry_at == 0 {
        tracing::warn!(transit = %id, destination = record.destination.0, reason,
            "settlement_transit_arrival_deferred");
    }
    record.retry_at = now.saturating_add(ARRIVAL_RETRY_TICKS);
}

fn horizontal(a: [i32; 3], b: [i32; 3]) -> f64 {
    mth::length2(f64::from(a[0] - b[0]), f64::from(a[2] - b[2]))
}

/// Everything a departure or emigration needs before it mutates: the unit's
/// living members, all in this dimension, all at `exit`, none holding
/// military, conversion, construction or economic custody, not a founding
/// household. No member is one `leave` would change (a Builder loses its
/// profession in `construction::detach`), so a transit image is the very body
/// that left.
fn admit(
    state: &TickState,
    source: SettlementId,
    unit: Unit<'_>,
    exit: BlockPos,
) -> Result<Vec<EntityId>, &'static str> {
    let members = households::unit_members(state, source, unit)?;
    let record = state.settlements.get(source).expect("validated source");
    if let Unit::Household(household) = unit {
        if record
            .founding
            .as_ref()
            .is_some_and(|founding| founding.household == household)
        {
            return Err("household leads a founding expedition");
        }
        if state.other_dimensions.values().any(|world| {
            world.entities.slots().any(|slot| {
                world.entities.body(slot).alive
                    && world
                        .entities
                        .row(slot)
                        .villager
                        .as_ref()
                        .is_some_and(|v| v.household.as_deref() == Some(household))
            })
        }) {
            return Err("a household member is in another dimension");
        }
    }
    let employments = &record.economy.commerce.employments;
    let centre = [f64::from(exit.x) + 0.5, f64::from(exit.z) + 0.5];
    for &entity in &members {
        let slot = state.entities.slot_of(entity).expect("living member");
        let row = state.entities.row(slot);
        let v = row.villager.as_ref().expect("villager member");
        if v.wandering || v.conversion_time.is_some() || v.smelting.is_some() {
            return Err("a member is converting, wandering or smelting");
        }
        if v.data.profession == crate::villager::VillagerProfession::Builder
            || state.settlements.records().any(|settlement| {
                settlement
                    .project
                    .as_ref()
                    .and_then(|project| project.borrowed_builder)
                    .is_some_and(|(builder, _)| builder == row.uuid)
            })
        {
            return Err("member has construction work; finish or release it first");
        }
        if super::economy::worker_reserved(state, Some(source), row.uuid)
            || employments.values().any(|employment| {
                employment.worker == row.uuid
                    && (employment.active || employment.paid < employment.earned)
            })
        {
            return Err("a member holds an economic reservation");
        }
        let pos = state.entities.pos(slot);
        if mth::length2(pos[0] - centre[0], pos[2] - centre[1]) > EXIT_REACH {
            return Err("a member is not at the exit point");
        }
    }
    Ok(members)
}

/// The member's complete native record in the WAL entity codec, exactly as a
/// full entity post-image of this tick would carry it.
fn capture(state: &TickState, slot: EntitySlot) -> ReplayEntity {
    let body = state.entities.body(slot);
    let row = state.entities.row(slot);
    let type_id = match body.kind {
        EntityKind::Mob { type_id }
        | EntityKind::Item { type_id }
        | EntityKind::ExperienceOrb { type_id } => type_id,
        EntityKind::Player { .. } => unreachable!("members are villagers"),
    };
    let [yaw, pitch, _] = state.entities.rot(slot);
    ReplayEntity {
        uuid: row.uuid.to_string(),
        type_id,
        pos: state.entities.pos(slot),
        vel: state.entities.vel(slot),
        yaw,
        pitch,
        ground: state.entities.ground(slot),
        health: row.health,
        alive: body.alive,
        age: row.age,
        breed_cooldown: row.breed_cooldown,
        pickup_delay: body.pickup_delay,
        item: row
            .item
            .as_deref()
            .map(crate::entities::StackRow::full_stack),
        appearance: crate::journal::EntityAppearanceImage::from(&*row.appearance),
        active_effects: row.active_effects.clone(),
        villager: row
            .villager
            .as_deref()
            .map(|villager| Box::new(crate::villager::persistence::journal_image(villager))),
        state: crate::journal::EntityStateImage::from_row(body, row),
        projectile: row.projectile.clone(),
        tick: state.tick,
    }
}

/// Departure tick of an abstract transit to `destination` (same dimension).
/// Validates first, then in this one tick: `leave` the source, capture and
/// remove each body without the death path and insert the durable record. The
/// record arrives [`TICKS_PER_BLOCK`] game ticks per block from `exit` to the
/// destination anchor after now.
pub(crate) fn depart(
    state: &mut TickState,
    changes: &mut ChangeSet,
    source: SettlementId,
    unit: Unit<'_>,
    destination: SettlementId,
    exit: BlockPos,
    bearing_deg: f32,
) -> Result<TransitId, &'static str> {
    if source == destination {
        return Err("migration needs another settlement");
    }
    if !bearing_deg.is_finite() {
        return Err("invalid bearing");
    }
    let destination_anchor = state
        .settlements
        .get(destination)
        .ok_or("destination settlement not found")?
        .anchor;
    let members = admit(state, source, unit, exit)?;
    let id = members
        .iter()
        .map(|&entity| {
            state
                .entities
                .row(state.entities.slot_of(entity).expect("living member"))
                .uuid
        })
        .min()
        .expect("a unit has members");
    if state.settlements.transit.records.contains_key(&id) {
        return Err("a member is already in transit");
    }
    let departed = households::leave(state, source, unit)?;
    let now = game_time(state);
    let exit = [exit.x, exit.y, exit.z];
    let distance = horizontal(exit, destination_anchor).ceil() as u64;
    // `leave` keeps the household key on each member for `join`; in transit
    // the record's `household` alone carries it (see [`validate`]).
    let mut images: Vec<(uuid::Uuid, ReplayEntity)> = departed
        .members
        .iter()
        .map(|&entity| {
            let slot = state.entities.slot_of(entity).expect("departed member");
            let mut image = capture(state, slot);
            if let Some(villager) = image.villager.as_mut() {
                villager.household = None;
            }
            (state.entities.row(slot).uuid, image)
        })
        .collect();
    images.sort_by_key(|(uuid, _)| *uuid);
    for &entity in &departed.members {
        let slot = state.entities.slot_of(entity).expect("departed member");
        state.remove_entity(slot);
        changes.entities_removed.push(entity);
    }
    let record = Record {
        source,
        destination,
        destination_anchor,
        departed: now,
        arrives: now.saturating_add(TICKS_PER_BLOCK.saturating_mul(distance)),
        bearing: bearing_deg,
        exit,
        status: Status::Travelling,
        household: departed.household,
        members: images.into_iter().map(|(_, image)| image).collect(),
        retry_at: 0,
    };
    tracing::info!(
        transit = %id,
        source = source.0,
        destination = destination.0,
        household = record.household.as_ref().map(|family| family.id.as_str()),
        members = record.members.len(),
        arrives = record.arrives,
        tick = state.tick,
        game_time = now,
        "settlement_transit_departure"
    );
    state.settlements.transit.put(id, record);
    Ok(TransitId(id))
}

/// Settlement → outside. Validates first (members at `exit`, every carried and
/// worn slot empty so no stock leaves the world), then in this one tick:
/// `leave` the source and remove the bodies for good. No record is kept; the
/// structured audit event names every UUID.
pub(crate) fn emigrate(
    state: &mut TickState,
    changes: &mut ChangeSet,
    source: SettlementId,
    unit: Unit<'_>,
    exit: BlockPos,
) -> Result<(), &'static str> {
    let members = admit(state, source, unit, exit)?;
    for &entity in &members {
        let row = state
            .entities
            .row(state.entities.slot_of(entity).expect("living member"));
        let villager = row.villager.as_ref().expect("villager member");
        if villager
            .inventory
            .iter()
            .flatten()
            .any(|stack| !stack.is_empty())
            || row
                .appearance
                .equipment
                .iter()
                .flatten()
                .any(|stack| !stack.is_empty())
        {
            return Err("an emigrant still carries items");
        }
    }
    let departed = households::leave(state, source, unit)?;
    let mut uuids = Vec::with_capacity(departed.members.len());
    for entity in departed.members {
        let slot = state.entities.slot_of(entity).expect("departed member");
        uuids.push(state.entities.row(slot).uuid);
        state.remove_entity(slot);
        changes.entities_removed.push(entity);
    }
    tracing::info!(
        settlement = source.0,
        household = departed.household.as_ref().map(|family| family.id.as_str()),
        members = ?uuids,
        tick = state.tick,
        game_time = game_time(state),
        "settlement_emigration"
    );
    Ok(())
}

/// Due records: retarget a deleted destination, mark `awaiting_entry` while
/// the destination anchor is not simulated, otherwise re-enter at its
/// simulated edge on the reverse bearing (at most [`MAX_ARRIVALS_PER_TICK`]).
/// Returns the arrivals that joined a destination in this call.
pub(crate) fn tick(state: &mut TickState, changes: &mut ChangeSet) -> Vec<Arrival> {
    let mut arrivals = Vec::new();
    if state.settlements.transit.records.is_empty() {
        return arrivals;
    }
    let now = game_time(state);
    let mut due: Vec<_> = state
        .settlements
        .transit
        .records
        .iter()
        .filter(|(_, record)| record.arrives <= now && record.retry_at <= now)
        .map(|(id, record)| (record.arrives.max(record.retry_at), *id))
        .collect();
    // Old untouched arrivals must not starve behind repeatedly refused UUIDs.
    due.sort_unstable();
    let mut admitted = 0;
    for (_, id) in due {
        retarget(state, id);
        let record = &state.settlements.transit.records[&id];
        let anchor = record.destination_anchor;
        if !state
            .world
            .is_active(anchor[0].div_euclid(16), anchor[2].div_euclid(16))
        {
            if record.status == Status::Travelling {
                let mut record = record.clone();
                record.status = Status::AwaitingEntry;
                state.settlements.transit.put(id, record);
            }
            continue;
        }
        if admitted == MAX_ARRIVALS_PER_TICK {
            continue;
        }
        admitted += 1;
        #[cfg(test)]
        {
            state.settlements.transit.entry_attempts += 1;
        }
        let back = (record.bearing + 180.0).rem_euclid(360.0);
        let Some(entry) = edge_point(state, anchor, back).or_else(|| anchor_entry(state, anchor))
        else {
            defer_arrival(
                state,
                id,
                now,
                "no standing entry on the return bearing or near the anchor",
            );
            continue;
        };
        match arrive(state, changes, id, entry) {
            Ok(arrival) => {
                arrivals.extend(arrival);
            }
            Err(reason) => {
                defer_arrival(state, id, now, &reason);
            }
        }
    }
    arrivals
}

/// A deleted destination: the source, else the nearest settlement, else none
/// (members then enter unbound at the old destination's edge).
fn retarget(state: &mut TickState, id: uuid::Uuid) {
    let record = &state.settlements.transit.records[&id];
    if state.settlements.get(record.destination).is_some() {
        return;
    }
    let old = record.destination_anchor;
    let target = state
        .settlements
        .get(record.source)
        .or_else(|| {
            state.settlements.records().min_by_key(|candidate| {
                let dx = i64::from(candidate.anchor[0] - old[0]);
                let dz = i64::from(candidate.anchor[2] - old[2]);
                (dx * dx + dz * dz, candidate.id)
            })
        })
        .map(|target| (target.id, target.anchor));
    let Some((destination, anchor)) = target else {
        return;
    };
    let mut record = record.clone();
    tracing::info!(
        transit = %id,
        from = record.destination.0,
        to = destination.0,
        "settlement_transit_retarget"
    );
    record.destination = destination;
    record.destination_anchor = anchor;
    record.retry_at = 0;
    state.settlements.transit.put(id, record);
}

/// Arrival tick: the same entities re-enter at `entry` and `join` the
/// destination (or stay unbound when none exists), the record is deleted, all
/// in this tick; returns the [`Arrival`] when the unit joined. Any refusal
/// leaves the world and the record unchanged.
fn arrive(
    state: &mut TickState,
    changes: &mut ChangeSet,
    id: uuid::Uuid,
    entry: BlockPos,
) -> Result<Option<Arrival>, std::borrow::Cow<'static, str>> {
    let record = state.settlements.transit.records[&id].clone();
    let mut uuids = Vec::with_capacity(record.members.len());
    for member in &record.members {
        let uuid: uuid::Uuid = member.uuid.parse().map_err(|_| "invalid member UUID")?;
        if worlds(state).any(|world| world.entities.slot_of_uuid(uuid).is_some()) {
            return Err("a member already exists in the world".into());
        }
        uuids.push(uuid);
    }
    let bound = state.settlements.get(record.destination).is_some_and(|to| {
        record
            .household
            .as_ref()
            .is_none_or(|family| !to.households.contains_key(&family.id))
    });
    if !bound && state.settlements.get(record.destination).is_some() {
        return Err("destination household identity conflict".into());
    }
    let centre = [
        f64::from(entry.x) + 0.5,
        f64::from(entry.y),
        f64::from(entry.z) + 0.5,
    ];
    let mut spawned = Vec::with_capacity(record.members.len());
    for member in &record.members {
        let mut image = member.clone();
        image.pos = centre;
        image.vel = [0.0; 3];
        image.ground = true;
        image.tick = state.tick;
        if let Some(villager) = image.villager.as_mut() {
            villager.household = record
                .household
                .as_ref()
                .filter(|_| bound)
                .map(|family| family.id.clone());
        }
        let entity = state.entities.alloc_id();
        if let Err(error) = crate::core::restore_entity(state, entity.0, &image) {
            discard(state, &spawned);
            return Err(format!("member {} could not restore: {error}", member.uuid).into());
        }
        spawned.push(entity);
    }
    if bound
        && let Err(reason) = households::join(
            state,
            record.destination,
            Departed {
                household: record.household.clone(),
                members: spawned.clone(),
            },
        )
    {
        discard(state, &spawned);
        return Err(reason.into());
    }
    state.settlements.transit.remove(id);
    let arrival = bound.then(|| Arrival {
        source: record.source,
        destination: record.destination,
        household: record.household.as_ref().map(|family| family.id.clone()),
        members: uuids,
    });
    tracing::info!(
        transit = %id,
        destination = record.destination.0,
        bound,
        members = spawned.len(),
        entry = ?[entry.x, entry.y, entry.z],
        tick = state.tick,
        game_time = game_time(state),
        "settlement_transit_arrival"
    );
    changes.entities_spawned.extend(spawned);
    Ok(arrival)
}

/// Undo this tick's partial re-entry; nothing of it was published yet.
fn discard(state: &mut TickState, spawned: &[EntityId]) {
    for &entity in spawned {
        if let Some(slot) = state.entities.slot_of(entity) {
            state.remove_entity(slot);
        }
    }
}

/// Members travelling to (or awaiting entry at) `destination`.
pub(crate) fn inbound(state: &TickState, destination: SettlementId) -> usize {
    state
        .settlements
        .transit
        .records
        .values()
        .filter(|record| record.destination == destination)
        .map(|record| record.members.len())
        .sum()
}

/// Records leaving or bound for `settlement`, for `settlement info`.
pub(crate) fn summary(state: &TickState, settlement: SettlementId) -> Vec<TransitSummary> {
    state
        .settlements
        .transit
        .records
        .iter()
        .filter(|(_, record)| record.source == settlement || record.destination == settlement)
        .map(|(id, record)| TransitSummary {
            id: TransitId(*id),
            source: record.source,
            destination: record.destination,
            household: record.household.as_ref().map(|family| family.id.clone()),
            members: record.members.len(),
            departed: record.departed,
            arrives: record.arrives,
            status: record.status,
        })
        .collect()
}

// ---- Operator commands (registered by the settlements command host) ----

/// `settlement transit depart <settlement> <household> <destination>`: a
/// transit departure from where the household stands. The exit is the block of
/// its lowest-UUID living member; [`depart`] validates everything else.
pub(crate) fn command_depart(
    state: &mut TickState,
    changes: &mut ChangeSet,
    source: SettlementId,
    household: &str,
    destination: SettlementId,
) -> Result<String, &'static str> {
    let unit = Unit::Household(household);
    let lead = households::unit_members(state, source, unit)?
        .into_iter()
        .map(|entity| {
            let slot = state.entities.slot_of(entity).expect("living member");
            (state.entities.row(slot).uuid, state.entities.pos(slot))
        })
        .min_by_key(|(uuid, _)| *uuid)
        .expect("a unit has members");
    let from = state
        .settlements
        .get(source)
        .expect("validated source")
        .anchor;
    let to = state
        .settlements
        .get(destination)
        .ok_or("destination settlement not found")?
        .anchor;
    let exit = super::allocator::at(lead.1);
    let id = depart(
        state,
        changes,
        source,
        unit,
        destination,
        exit,
        bearing(from, to),
    )?;
    let record = state.settlements.transit.get(id).expect("inserted record");
    Ok(format!(
        "Household {household} left in transit {} with {} members; earliest arrival at game time {}.",
        id.0,
        record.members.len(),
        record.arrives
    ))
}

/// `settlement transit list`: one line per record of this dimension.
pub(crate) fn command_list(state: &TickState) -> Vec<String> {
    state
        .settlements
        .transit
        .records
        .iter()
        .map(|(id, record)| {
            format!(
                "transit {id}: {} ({} members) {} -> {}, {}, departed {}, arrives {}",
                record
                    .household
                    .as_ref()
                    .map_or("lone resident", |family| family.id.as_str()),
                record.members.len(),
                record.source.0,
                record.destination.0,
                match record.status {
                    Status::Travelling => "travelling",
                    Status::AwaitingEntry => "awaiting entry",
                },
                record.departed,
                record.arrives
            )
        })
        .collect()
}

// ---- Row and WAL codec, recovery check ----

fn key(id: uuid::Uuid) -> String {
    format!("transit/{id}")
}

/// Canonical `transit/<uuid>` key (hyphenated lowercase).
fn parse_key(key: &str) -> Option<uuid::Uuid> {
    let suffix = key.strip_prefix("transit/")?;
    let id: uuid::Uuid = suffix.parse().ok()?;
    (id.to_string() == suffix).then_some(id)
}

/// A checkpoint row: the home dimension and the record.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Row<T> {
    dimension: Dimension,
    value: T,
}

fn storage(reason: impl std::fmt::Display) -> CoreError {
    CoreError::Storage(format!("native settlement transit: {reason}"))
}

/// Validate a record stored under `id`; its member UUIDs, strictly ascending,
/// the first being `id`. Members name no settlement and no household: the
/// record's `household` is the unit's only household identity in transit
/// (the villager codec refuses a household without a settlement).
fn validate(record: &Record, id: uuid::Uuid) -> Result<Vec<uuid::Uuid>, String> {
    if record.members.is_empty() {
        return Err(format!("record {id} has no members"));
    }
    if record.departed > record.arrives || !record.bearing.is_finite() {
        return Err(format!("record {id} has an invalid clock or bearing"));
    }
    if record
        .household
        .as_ref()
        .is_some_and(|family| family.id.is_empty() || family.revision == 0)
    {
        return Err(format!("record {id} has an invalid household"));
    }
    let mut uuids: Vec<uuid::Uuid> = Vec::with_capacity(record.members.len());
    for member in &record.members {
        let uuid: uuid::Uuid = member
            .uuid
            .parse()
            .map_err(|_| format!("record {id}: member UUID {} is invalid", member.uuid))?;
        if uuids.last().is_some_and(|last| *last >= uuid) {
            return Err(format!(
                "record {id}: member {uuid} is duplicated or out of order"
            ));
        }
        let villager = member
            .villager
            .as_ref()
            .filter(|_| member.alive)
            .ok_or_else(|| format!("record {id}: member {uuid} is not a living villager"))?;
        if villager.settlement.is_some() || villager.household.is_some() {
            return Err(format!(
                "record {id}: member {uuid} disagrees with its record"
            ));
        }
        uuids.push(uuid);
    }
    if uuids[0] != id {
        return Err(format!("record {id} is not keyed by its lowest member"));
    }
    Ok(uuids)
}

fn decode_row(
    key: &str,
    id: uuid::Uuid,
    row: &NativeDomainRecord,
) -> Result<Row<Record>, CoreError> {
    if row.version != ROW_VERSION {
        return Err(storage(format!("row {key} has unsupported version")));
    }
    let row: Row<Record> = serde_json::from_slice(&row.payload)
        .map_err(|error| storage(format!("row {key} unreadable: {error}")))?;
    validate(&row.value, id).map_err(storage)?;
    Ok(row)
}

/// Settlements persistence hookup: one `transit/<uuid>` row per record.
pub(super) fn checkpoint_rows(
    world: &TickState,
    tick: u64,
    rows: &mut BTreeMap<String, NativeDomainRecord>,
) -> Result<(), CoreError> {
    for (id, record) in &world.settlements.transit.records {
        let payload = serde_json::to_vec(&Row {
            dimension: world.dimension,
            value: record,
        })
        .map_err(storage)?;
        rows.insert(key(*id), crate::native_checkpoint::record(tick, payload));
    }
    Ok(())
}

/// Settlements persistence hookup: touched records as absolute post-images or
/// tombstones, in the same tick batch as the entity removals and re-entries.
pub(super) fn journal_entries(state: &mut TickState, tick: u64, entries: &mut Vec<JournalEntry>) {
    let ledger = &mut state.settlements.transit;
    for id in std::mem::take(&mut ledger.touched) {
        let record = ledger.records.get(&id).map(|record| {
            Arc::new(serde_json::to_string(record).expect("transit record serialization"))
        });
        entries.push(JournalEntry::Transit { id, record, tick });
    }
}

/// Settlements persistence hookup: this dimension's rows folded with its WAL
/// tail. Every row and image is decoded and validated; any failure refuses.
pub(super) fn hydrate(
    state: &mut TickState,
    checkpoint: &BTreeMap<String, NativeDomainRecord>,
    replay: &Replay,
) -> Result<(), CoreError> {
    for (key, row) in checkpoint {
        let Some(id) = parse_key(key) else {
            continue;
        };
        let row = decode_row(key, id, row)?;
        if row.dimension == state.dimension {
            state.settlements.transit.records.insert(id, row.value);
        }
    }
    for (id, image) in &replay.transits {
        match image {
            Some(payload) => {
                let record: Record = serde_json::from_str(payload)
                    .map_err(|error| storage(format!("WAL image {id} unreadable: {error}")))?;
                validate(&record, *id).map_err(storage)?;
                state.settlements.transit.records.insert(*id, record);
            }
            None => {
                state.settlements.transit.records.remove(id);
            }
        }
    }
    state.settlements.transit.touched.clear();
    Ok(())
}

/// Recovery check once every dimension hydrated: each member UUID is in one
/// transit record only and is no world entity in any dimension, every member
/// image passes the refusals of arrival's `restore_entity`, and each transit
/// household is in one record only and is no settlement's. A violating store
/// refuses to open.
pub(super) fn validate_custody(state: &TickState) -> Result<(), CoreError> {
    let households: BTreeSet<&str> = worlds(state)
        .flat_map(|world| world.settlements.records())
        .flat_map(|record| record.households.keys().map(String::as_str))
        .collect();
    let mut seen = BTreeSet::new();
    let mut travelling = BTreeMap::new();
    for world in worlds(state) {
        for (id, record) in &world.settlements.transit.records {
            for uuid in validate(record, *id).map_err(storage)? {
                if !seen.insert(uuid) {
                    return Err(storage(format!("member {uuid} is in two transit records")));
                }
                if worlds(state).any(|world| world.entities.slot_of_uuid(uuid).is_some()) {
                    return Err(storage(format!(
                        "member {uuid} is both a world entity and in transit record {id}"
                    )));
                }
            }
            restorable(world, *id, record)?;
            if let Some(family) = &record.household {
                if households.contains(family.id.as_str()) {
                    return Err(storage(format!(
                        "household {} is both in transit record {id} and a settlement's",
                        family.id
                    )));
                }
                if let Some(other) = travelling.insert(family.id.as_str(), *id) {
                    return Err(storage(format!(
                        "household {} is in transit records {other} and {id}",
                        family.id
                    )));
                }
            }
        }
    }
    Ok(())
}

/// The refusals `restore_entity` applies to each member image on arrival
/// (registered type, retained state, variants, finite motion, retained NBT),
/// so every record recovery serves can re-enter.
fn restorable(world: &TickState, id: uuid::Uuid, record: &Record) -> Result<(), CoreError> {
    for member in &record.members {
        crate::dimensions::recovery::validate_entity(world, 0, member)
            .map_err(|error| storage(format!("record {id}: member image refused: {error}")))?;
        crate::command_tag::normalize_opaque(member.state.opaque_entity_fields.clone()).map_err(
            |error| {
                storage(format!(
                    "record {id}: member {} retained data refused: {error}",
                    member.uuid
                ))
            },
        )?;
    }
    Ok(())
}

/// Offline census view of one persisted transit row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CensusRow {
    pub id: uuid::Uuid,
    pub dimension: Dimension,
    pub source: u64,
    pub destination: u64,
    pub household: Option<String>,
    pub members: Vec<uuid::Uuid>,
}

/// Every `transit/<uuid>` row of a checkpoint's globals, decoded and validated
/// exactly as recovery does.
pub fn census_rows(
    globals: &BTreeMap<String, NativeDomainRecord>,
) -> Result<Vec<CensusRow>, CoreError> {
    let mut rows = Vec::new();
    for (key, row) in globals {
        if !key.starts_with("transit/") {
            continue;
        }
        let id = parse_key(key).ok_or_else(|| storage(format!("non-canonical row key {key}")))?;
        let row = decode_row(key, id, row)?;
        let members = validate(&row.value, id).map_err(storage)?;
        rows.push(CensusRow {
            id,
            dimension: row.dimension,
            source: row.value.source.0,
            destination: row.value.destination.0,
            household: row.value.household.map(|family| family.id),
            members,
        });
    }
    Ok(rows)
}

#[cfg(test)]
#[path = "transit_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "transit_disk_tests.rs"]
mod disk_tests;
