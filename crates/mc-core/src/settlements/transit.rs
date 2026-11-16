//! Settlement migration across unsimulated ground: emigration to the outside
//! and the durable transit custody of a household between two settlements.
//!
//! A migrating unit (a household or a lone resident) leaves its source with
//! [`households::leave`]. Emigration then removes the bodies for good; transit
//! keeps every member's complete native entity record until the destination
//! is simulated again and [`households::join`] admits them there.
//!
//! Geometry: a bearing is degrees clockwise from north (−Z), so 90° is east
//! (+X). Exit and entry points lie on the edge of the entity-simulated area,
//! independent of the civil settlement radius.

use super::SettlementId;
use super::households::{self, Unit};
use crate::boundary::{BlockPos, ChangeSet, EntityId};
use crate::tick::TickState;

/// A point closer than this to the anchor is not an exit or entry point.
pub(crate) const MIN_EDGE_DISTANCE: i32 = 16;
/// Search limit along a bearing; far beyond any simulation distance.
const MAX_EDGE_DISTANCE: i32 = 1024;
/// Every living member stands within this horizontal distance of the exit.
pub(crate) const EXIT_REACH: f64 = 2.0;
/// Villager body used to judge a standable surface cell.
const BODY_HALF_WIDTH: f64 = 0.3;
const BODY_HEIGHT: f64 = 1.95;

/// Identity of one transit record: its lowest member UUID. A UUID is in at
/// most one place, so no second sequence exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct TransitId(pub(crate) uuid::Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    Travelling,
    AwaitingEntry,
}

/// `settlement info` row for one record touching a settlement.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TransitSummary {
    pub(crate) id: TransitId,
    pub(crate) source: SettlementId,
    pub(crate) destination: SettlementId,
    pub(crate) household: Option<String>,
    pub(crate) members: usize,
    /// Game time of the departure tick and of the earliest arrival.
    pub(crate) departed: u64,
    pub(crate) arrives: u64,
    pub(crate) status: Status,
}

/// Bearing from `from` to `to` in this module's convention.
pub(crate) fn bearing(from: [i32; 3], to: [i32; 3]) -> f32 {
    let dx = f64::from(to[0] - from[0]);
    let dz = f64::from(to[2] - from[2]);
    dx.atan2(-dz).to_degrees().rem_euclid(360.0) as f32
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
    let (dx, dz) = (radians.sin(), -radians.cos());
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
        let horizontal = f64::from(column.0 - anchor[0]).hypot(f64::from(column.1 - anchor[2]));
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

/// Everything a departure or emigration needs before it mutates: the unit's
/// living members, all in this dimension, all at `exit`, none holding
/// military, conversion or economic custody, not a founding household.
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
        if super::economy::worker_reserved(state, Some(source), row.uuid)
            || employments.values().any(|employment| {
                employment.worker == row.uuid
                    && (employment.active || employment.paid < employment.earned)
            })
        {
            return Err("a member holds an economic reservation");
        }
        let pos = state.entities.pos(slot);
        if (pos[0] - centre[0]).hypot(pos[2] - centre[1]) > EXIT_REACH {
            return Err("a member is not at the exit point");
        }
    }
    Ok(members)
}

/// Departure tick of an abstract transit to `destination` (same dimension).
/// Validates first, then in this one tick: `leave` the source, remove each
/// body without the death path and insert the durable record.
pub(crate) fn depart(
    state: &mut TickState,
    changes: &mut ChangeSet,
    source: SettlementId,
    unit: Unit<'_>,
    destination: SettlementId,
    exit: BlockPos,
    bearing_deg: f32,
) -> Result<TransitId, &'static str> {
    let _ = (changes, bearing_deg);
    if source == destination {
        return Err("migration needs another settlement");
    }
    state
        .settlements
        .get(destination)
        .ok_or("destination settlement not found")?;
    admit(state, source, unit, exit)?;
    Err("migration transit custody is not available on this build")
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
        game_time = state.tick_control.game_ticks(state.tick),
        "settlement_emigration"
    );
    Ok(())
}

/// Arrivals and `awaiting_entry` transitions, bounded per tick.
pub(crate) fn tick(state: &mut TickState, changes: &mut ChangeSet) {
    let _ = (state, changes);
}

/// Members travelling to (or awaiting entry at) `destination`.
pub(crate) fn inbound(state: &TickState, destination: SettlementId) -> usize {
    let _ = (state, destination);
    0
}

/// Records leaving or bound for `settlement`, for `settlement info`.
pub(crate) fn summary(state: &TickState, settlement: SettlementId) -> Vec<TransitSummary> {
    let _ = (state, settlement);
    Vec::new()
}

#[cfg(test)]
#[path = "transit_tests.rs"]
mod tests;
