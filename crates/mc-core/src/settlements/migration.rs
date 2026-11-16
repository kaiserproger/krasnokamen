//! Settlement migration policy and its derived, non-persisted observations.
//! Native membership and transit custody remain their existing owners.

use super::SettlementId;
use super::{allocator, households, transit};
use crate::boundary::{BlockPos, ChangeSet, EntityEvent, EntityId};
use crate::entities::EntitySlot;
use crate::natural_spawn::Rng;
use crate::tick::TickState;
use crate::tick_budget::Consumer;
use mc_physics::mth;
use std::collections::BTreeMap;
use std::collections::{BTreeSet, VecDeque};

const DAY_TICKS: u64 = 24_000;
const FOOD_PER_RESIDENT: f64 = 12.0;
const DEVELOPED_BUILDINGS: f64 = 12.0;
const OUTSIDE_ATTRACTIVENESS: f64 = 35.0;
const MIGRATION_THRESHOLD: f64 = 10.0;
const DISTANCE_COST_PER_BLOCK: f64 = 0.05;
const SCORE_SCALE: f64 = 100.0;
const HOUSING_WEIGHT: f64 = 0.30;
const FOOD_WEIGHT: f64 = 0.30;
const JOBS_WEIGHT: f64 = 0.15;
const SAFETY_WEIGHT: f64 = 0.15;
const DEVELOPMENT_WEIGHT: f64 = 0.10;
const SAFETY_DAYS: u64 = 7;
const EDGE_SURFACE_BAND: i32 = 32;
const ADULT_HEALTH: f32 = 20.0;
const ADULT_HALF_WIDTH: f64 = 0.3;
const ADULT_HEIGHT: f64 = 1.95;
const EXTRA_ARRIVAL_MAX_CHANCE: f64 = 0.5;
const WALK_TIMEOUT: u64 = DAY_TICKS;

/// Current physical inputs; these observations never become membership authority.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Facts {
    pub(crate) members: usize,
    pub(crate) beds: usize,
    pub(crate) inbound: usize,
    pub(crate) food_points: u64,
    pub(crate) open_jobs: usize,
    pub(crate) recent_deaths: usize,
    pub(crate) completed_buildings: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Attractiveness {
    pub(crate) housing: f64,
    pub(crate) food: f64,
    pub(crate) jobs: f64,
    pub(crate) safety: f64,
    pub(crate) development: f64,
}

impl Attractiveness {
    fn from_facts(facts: &Facts) -> Self {
        let population = facts.members.max(1) as f64;
        Self {
            housing: (facts
                .beds
                .saturating_sub(facts.members)
                .saturating_sub(facts.inbound) as f64
                / population)
                .clamp(0.0, 1.0),
            food: (facts.food_points as f64 / (FOOD_PER_RESIDENT * population)).clamp(0.0, 1.0),
            jobs: (facts.open_jobs as f64 / population).clamp(0.0, 1.0),
            safety: 1.0 - (facts.recent_deaths as f64 / population).clamp(0.0, 1.0),
            development: (facts.completed_buildings as f64 / DEVELOPED_BUILDINGS).clamp(0.0, 1.0),
        }
    }

    pub(crate) fn total(&self) -> f64 {
        SCORE_SCALE
            * (HOUSING_WEIGHT * self.housing
                + FOOD_WEIGHT * self.food
                + JOBS_WEIGHT * self.jobs
                + SAFETY_WEIGHT * self.safety
                + DEVELOPMENT_WEIGHT * self.development)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Observation {
    pub(crate) game_time: u64,
    pub(crate) facts: Facts,
    pub(crate) attractiveness: Attractiveness,
}

/// One host-owned runtime state. No serde, journal image, or saved sequence.
#[derive(Debug, Default)]
pub(crate) struct State {
    observations: BTreeMap<SettlementId, Observation>,
    pub(crate) returning: rustc_hash::FxHashMap<uuid::Uuid, crate::villager::return_home::Memory>,
    deaths: BTreeMap<SettlementId, VecDeque<u64>>,
    decisions: BTreeMap<SettlementId, String>,
    walking: rustc_hash::FxHashMap<SettlementId, Walk>,
    walking_sources: Vec<SettlementId>,
    targets: BTreeMap<uuid::Uuid, BlockPos>,
}

impl State {
    pub(crate) fn observe(&mut self, id: SettlementId, game_time: u64, facts: Facts) {
        self.observations.insert(
            id,
            Observation {
                game_time,
                facts,
                attractiveness: Attractiveness::from_facts(&facts),
            },
        );
    }

    pub(crate) fn observation(&self, id: SettlementId) -> Option<&Observation> {
        self.observations.get(&id)
    }

    pub(crate) fn remove(&mut self, id: SettlementId) {
        self.observations.remove(&id);
        self.deaths.remove(&id);
        self.decisions.remove(&id);
        self.walking.retain(|source, walk| {
            if *source != id && walk.destination != Some(id) {
                return true;
            }
            self.decisions.remove(source);
            for uuid in &walk.members {
                self.targets.remove(uuid);
            }
            false
        });
        self.walking_sources
            .retain(|source| self.walking.contains_key(source));
        self.returning.retain(|_, memory| memory.settlement() != id);
    }
}
pub(crate) fn return_memory(
    state: &mut crate::tick::TickState,
) -> &mut rustc_hash::FxHashMap<uuid::Uuid, crate::villager::return_home::Memory> {
    &mut state.settlements.migration.returning
}

fn phase(id: SettlementId) -> u64 {
    crate::natural_spawn::Rng(id.0).below(DAY_TICKS)
}

fn due(id: SettlementId, game_time: u64) -> bool {
    game_time % DAY_TICKS == phase(id)
}

fn gain(source: f64, destination: f64, distance: f64) -> f64 {
    destination - source - DISTANCE_COST_PER_BLOCK * distance
}

#[cfg(test)]
#[path = "migration_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "migration_return_tests.rs"]
pub(crate) mod return_tests;

#[cfg(test)]
#[path = "migration_flow_tests.rs"]
mod flow_tests;

#[derive(Debug, Clone)]
enum Unit {
    Household(String),
    Resident(EntityId),
}

impl Unit {
    fn borrowed(&self) -> households::Unit<'_> {
        match self {
            Self::Household(key) => households::Unit::Household(key),
            Self::Resident(id) => households::Unit::Resident(*id),
        }
    }
}

#[derive(Debug)]
struct Walk {
    unit: Unit,
    members: Vec<uuid::Uuid>,
    destination: Option<SettlementId>,
    exit: BlockPos,
    bearing: f32,
    deadline: u64,
}

pub(crate) fn target(state: &TickState, resident: uuid::Uuid) -> Option<BlockPos> {
    state.settlements.migration.targets.get(&resident).copied()
}

fn simulated(state: &TickState, anchor: [i32; 3]) -> bool {
    state
        .world
        .is_active(anchor[0].div_euclid(16), anchor[2].div_euclid(16))
}

fn horizontal(a: [i32; 3], b: [i32; 3]) -> f64 {
    mth::length2(
        f64::from(a[0]) - f64::from(b[0]),
        f64::from(a[2]) - f64::from(b[2]),
    )
}

fn in_bounds(bounds: mc_script::ScriptWorldBounds, pos: BlockPos) -> bool {
    [pos.x, pos.y, pos.z]
        .into_iter()
        .enumerate()
        .all(|(i, p)| p >= bounds.min[i] && p <= bounds.max[i])
}

fn physical_beds(state: &TickState, record: &super::SettlementRecord) -> usize {
    let mut beds = BTreeSet::new();
    for building in record
        .buildings
        .values()
        .filter(|b| b.kind == super::buildings::BuildingKind::House)
    {
        for x in building.bounds.min[0].div_euclid(16)..=building.bounds.max[0].div_euclid(16) {
            for z in building.bounds.min[2].div_euclid(16)..=building.bounds.max[2].div_euclid(16) {
                if let Some(chunk) = state
                    .villager_pois
                    .chunks
                    .get(&crate::boundary::ChunkPos { x, z })
                {
                    for poi in &chunk.pois {
                        if poi.kind == allocator::PoiKind::Home
                            && record.contains([poi.pos.x, poi.pos.y, poi.pos.z])
                            && in_bounds(building.bounds, poi.pos)
                            && allocator::bed_pair(&state.world, &state.registry, poi.pos).is_some()
                            && allocator::roofed(&state.world, &state.registry, poi.pos)
                        {
                            beds.insert((poi.pos.x, poi.pos.y, poi.pos.z));
                        }
                    }
                }
            }
        }
    }
    beds.len()
}

fn facts(state: &TickState, id: SettlementId, now: u64) -> Facts {
    let record = state.settlements.get(id).expect("observed settlement");
    let mut jobs = BTreeSet::new();
    for building in record.buildings.values() {
        for &p in building
            .work_points
            .iter()
            .take(super::buildings::job_slots(building.kind, building.level))
        {
            let pos = BlockPos {
                x: p[0],
                y: p[1],
                z: p[2],
            };
            if state
                .villager_pois
                .jobs
                .get(&pos)
                .is_none_or(|owners| owners.is_empty())
            {
                jobs.insert(p);
            }
        }
    }
    let food_points = super::inventory::town_storage(state, id)
        .into_iter()
        .flat_map(|(_, image)| image.slots)
        .map(|stack| {
            state
                .registry
                .items
                .name_of(stack.item_id)
                .map_or(0, |name| {
                    u64::from(crate::villager::brain::food_points_for_name(name.path()))
                        * stack.count.max(0) as u64
                })
        })
        .sum();
    let recent_deaths = state
        .settlements
        .migration
        .deaths
        .get(&id)
        .map_or(0, |deaths| {
            deaths
                .iter()
                .filter(|&&tick| now.saturating_sub(tick) < SAFETY_DAYS * DAY_TICKS)
                .count()
        });
    Facts {
        members: super::member_count(state, id),
        beds: physical_beds(state, record),
        inbound: transit::inbound(state, id),
        food_points,
        open_jobs: jobs.len(),
        recent_deaths,
        completed_buildings: record.buildings.len(),
    }
}

fn pending(state: &TickState, destination: SettlementId) -> usize {
    state
        .settlements
        .migration
        .walking
        .values()
        .filter(|walk| walk.destination == Some(destination))
        .map(|walk| walk.members.len())
        .sum()
}

fn capacity(state: &TickState, destination: SettlementId) -> usize {
    state
        .settlements
        .migration
        .observation(destination)
        .map_or(0, |observation| {
            let beds = state
                .settlements
                .get(destination)
                .filter(|record| simulated(state, record.anchor))
                .map_or(observation.facts.beds, |record| {
                    physical_beds(state, record)
                });
            beds.saturating_sub(super::member_count(state, destination))
                .saturating_sub(transit::inbound(state, destination))
                .saturating_sub(pending(state, destination))
        })
}

fn empty(state: &TickState, slot: EntitySlot) -> bool {
    let row = state.entities.row(slot);
    row.villager
        .as_ref()
        .is_some_and(|v| v.inventory.iter().flatten().all(|stack| stack.count <= 0))
        && row
            .appearance
            .equipment
            .iter()
            .flatten()
            .all(|stack| stack.count <= 0)
}

/// Everything which could refuse a move is checked before membership changes.
fn eligible(
    state: &TickState,
    source: SettlementId,
    unit: &Unit,
    outside: bool,
) -> Result<Vec<EntityId>, &'static str> {
    let record = state.settlements.get(source).ok_or("source deleted")?;
    if let Unit::Household(key) = unit
        && record
            .founding
            .as_ref()
            .is_some_and(|founding| founding.household == *key)
    {
        return Err("household leads a founding expedition");
    }
    let members = households::unit_members(state, source, unit.borrowed())?;
    for &member in &members {
        let slot = state.entities.slot_of(member).ok_or("member missing")?;
        let row = state.entities.row(slot);
        let v = row.villager.as_ref().ok_or("member is not a villager")?;
        if !super::construction::founding::member_unreserved(state, source, member)
            || v.wandering
            || v.smelting.is_some()
            || v.data.profession == crate::villager::VillagerProfession::Builder
            || state.settlements.records().any(|town| {
                town.project
                    .as_ref()
                    .and_then(|project| project.borrowed_builder)
                    .is_some_and(|(builder, _)| builder == row.uuid)
            })
            || record
                .economy
                .commerce
                .employments
                .values()
                .any(|employment| {
                    employment.worker == row.uuid
                        && (employment.active || employment.paid < employment.earned)
                })
        {
            return Err("member holds a military, conversion or economic/construction reservation");
        }
        if outside && !empty(state, slot) {
            return Err("outside emigration requires empty inventories");
        }
    }
    if let Unit::Household(key) = unit
        && state.other_dimensions.values().any(|world| {
            world.entities.slots().any(|slot| {
                world.entities.body(slot).alive
                    && world.entities.row(slot).villager.as_ref().is_some_and(|v| {
                        v.settlement == Some(source) && v.household.as_deref() == Some(key.as_str())
                    })
            })
        })
    {
        return Err("household spans dimensions");
    }
    Ok(members)
}

#[derive(Debug, PartialEq)]
enum Proof {
    Reachable,
    Blocked,
    Deferred,
}

fn route(
    state: &mut TickState,
    from: [f64; 3],
    target: BlockPos,
    width: f64,
    height: f64,
) -> Proof {
    let mut cache = std::mem::take(&mut state.villager_pois);
    let mut path = std::mem::take(&mut cache.candidate_path);
    let start = allocator::at(from);
    let horizontal = start
        .x
        .abs_diff(target.x)
        .max(start.z.abs_diff(target.z))
        .saturating_add(8)
        .max(48);
    let vertical = start.y.abs_diff(target.y).saturating_add(4).max(16);
    let mut ctx = allocator::RouteCtx::tick(state, Consumer::Brain);
    let result = if !ctx.admit() {
        Proof::Deferred
    } else {
        let found = allocator::find_path_ctx(
            &mut ctx, &mut cache, from, target, width, height, &mut path, true, horizontal,
            vertical, true,
        );
        if found {
            Proof::Reachable
        } else if ctx.budget.allowance(
            ctx.dimension,
            ctx.consumer,
            crate::tick_budget::Kind::PathNode,
        ) == 0
        {
            Proof::Deferred
        } else {
            Proof::Blocked
        }
    };
    path.clear();
    cache.candidate_path = path;
    state.villager_pois = cache;
    result
}

fn approach(state: &TickState, anchor: [i32; 3]) -> Option<BlockPos> {
    for (dx, dz) in [
        (0, 0),
        (1, 0),
        (-1, 0),
        (0, 1),
        (0, -1),
        (2, 0),
        (-2, 0),
        (0, 2),
        (0, -2),
    ] {
        for dy in [0, 1, -1, 2, -2] {
            let pos = BlockPos {
                x: anchor[0] + dx,
                y: anchor[1] + dy,
                z: anchor[2] + dz,
            };
            if simulated(state, [pos.x, pos.y, pos.z])
                && allocator::standing(
                    &state.world,
                    &state.registry,
                    pos,
                    ADULT_HALF_WIDTH,
                    ADULT_HEIGHT,
                )
                .is_some()
            {
                return Some(pos);
            }
        }
    }
    None
}

fn unit_route(state: &mut TickState, members: &[EntityId], target: BlockPos) -> Proof {
    for &member in members {
        let Some(slot) = state.entities.slot_of(member) else {
            return Proof::Blocked;
        };
        let pos = state.entities.pos(slot);
        let body = state.entities.body(slot);
        let (width, height) = (body.half_width, body.height);
        let proof = route(state, pos, target, width, height);
        if proof != Proof::Reachable {
            return proof;
        }
    }
    Proof::Reachable
}

enum EdgeTravel<'a> {
    Leaving(&'a [EntityId]),
    Entering(BlockPos),
}

/// The geometry helper finds the outermost surface, not a route. Prove it
/// first, then consider inward surface columns in descending distance order.
/// Shared-budget denial defers the whole decision, never invents a route.
fn proven_edge(
    state: &mut TickState,
    anchor: [i32; 3],
    bearing: f32,
    travel: EdgeTravel<'_>,
) -> Result<BlockPos, Proof> {
    let farthest = transit::edge_point(state, anchor, bearing).ok_or(Proof::Blocked)?;
    let radius = horizontal(anchor, [farthest.x, farthest.y, farthest.z]).ceil() as i32;
    let radians = f64::from(bearing).to_radians();
    for distance in (transit::MIN_EDGE_DISTANCE..=radius).rev() {
        let point = if distance == radius {
            Some(farthest)
        } else {
            let x =
                (f64::from(anchor[0]) + 0.5 + f64::from(mth::sin(radians)) * f64::from(distance))
                    .floor() as i32;
            let z = (f64::from(anchor[2]) + 0.5
                - f64::from(mth::cos(radians)) * f64::from(distance))
            .floor() as i32;
            if !simulated(state, [x, anchor[1], z]) {
                continue;
            }
            (anchor[1] - EDGE_SURFACE_BAND..=anchor[1] + EDGE_SURFACE_BAND)
                .rev()
                .find_map(|y| {
                    let cell = BlockPos { x, y, z };
                    allocator::standing(
                        &state.world,
                        &state.registry,
                        cell,
                        ADULT_HALF_WIDTH,
                        ADULT_HEIGHT,
                    )
                    .map(|_| cell)
                })
        };
        let Some(point) = point else {
            continue;
        };
        if horizontal(anchor, [point.x, point.y, point.z]) < f64::from(transit::MIN_EDGE_DISTANCE) {
            continue;
        }
        let proof = match travel {
            EdgeTravel::Leaving(members) => unit_route(state, members, point),
            EdgeTravel::Entering(goal) => route(
                state,
                allocator::centre(point),
                goal,
                ADULT_HALF_WIDTH,
                ADULT_HEIGHT,
            ),
        };
        match proof {
            Proof::Reachable => return Ok(point),
            Proof::Deferred => return Err(Proof::Deferred),
            Proof::Blocked => {}
        }
    }
    Err(Proof::Blocked)
}

fn notice(
    state: &mut TickState,
    settlement: SettlementId,
    event: &'static str,
    unit: uuid::Uuid,
    reason: &str,
) {
    tracing::info!(
        settlement = settlement.0,
        event,
        unit = %unit,
        tick = state.tick,
        reason,
        game_time = state.tick_control.game_ticks(state.tick),
        "settlement_migration"
    );
    let owner = state
        .settlements
        .get(settlement)
        .and_then(|r| r.owner.as_deref())
        .and_then(|s| s.parse::<uuid::Uuid>().ok());
    let connection = owner.and_then(|owner| {
        state
            .connections
            .all_connections()
            .find(|connection| state.connections.uuid_of(*connection) == Some(owner))
    });
    if let Some(connection) = connection {
        state.push_player(
            connection,
            crate::boundary::PlayerChange::Chat {
                message: format!("Migration {event}: {reason}"),
            },
        );
    }
    state
        .settlements
        .migration
        .decisions
        .insert(settlement, format!("{event}: {reason}"));
}

fn units(state: &TickState, source: SettlementId) -> Vec<Unit> {
    let mut units: Vec<_> = state
        .settlements
        .get(source)
        .into_iter()
        .flat_map(|r| r.households.keys())
        .cloned()
        .map(Unit::Household)
        .collect();
    let mut singletons: Vec<_> = state
        .entities
        .slots()
        .filter(|&slot| {
            state.entities.body(slot).alive
                && state
                    .entities
                    .row(slot)
                    .villager
                    .as_ref()
                    .is_some_and(|v| v.settlement == Some(source) && v.household.is_none())
        })
        .map(|slot| (state.entities.row(slot).uuid, state.entities.body(slot).id))
        .collect();
    singletons.sort_unstable_by_key(|(uuid, _)| *uuid);
    units.extend(singletons.into_iter().map(|(_, id)| Unit::Resident(id)));
    units
}

fn relocate(
    state: &mut TickState,
    source: SettlementId,
    destination: SettlementId,
    unit: &Unit,
) -> Result<(), &'static str> {
    let to = state
        .settlements
        .get(destination)
        .ok_or("destination deleted")?;
    to.revision
        .checked_add(1)
        .ok_or("destination revision exhausted")?;
    match unit {
        Unit::Household(key) => households::relocate(state, source, key, destination),
        Unit::Resident(_) => {
            let departed = households::leave(state, source, unit.borrowed())?;
            households::join(state, destination, departed)
        }
    }
}

fn depart_decision(state: &mut TickState, source: SettlementId, rng: &mut Rng) {
    if state.settlements.migration.walking.contains_key(&source) {
        return;
    }
    if rng.unit() >= state.test_acceleration.migration_rate.min(1.0) {
        return;
    }
    let observation = *state
        .settlements
        .migration
        .observation(source)
        .expect("evaluated source");
    let source_anchor = state.settlements.get(source).unwrap().anchor;
    let mut reason = "no destination exceeds the migration threshold";
    for unit in units(state, source) {
        let members = match eligible(state, source, &unit, false) {
            Ok(members) => members,
            Err(refusal) => {
                reason = refusal;
                continue;
            }
        };
        let can_emigrate = members
            .iter()
            .all(|id| empty(state, state.entities.slot_of(*id).expect("eligible member")));
        let mut best = (
            if can_emigrate {
                OUTSIDE_ATTRACTIVENESS - observation.attractiveness.total()
            } else {
                f64::NEG_INFINITY
            },
            None,
        );
        for destination in state.settlements.records().filter(|r| r.id != source) {
            if capacity(state, destination.id) < members.len() {
                continue;
            }
            let Some(candidate) = state.settlements.migration.observation(destination.id) else {
                continue;
            };
            let score = gain(
                observation.attractiveness.total(),
                candidate.attractiveness.total(),
                horizontal(source_anchor, destination.anchor),
            );
            if score > best.0 {
                best = (score, Some(destination.id));
            }
        }
        if best.0 <= MIGRATION_THRESHOLD {
            continue;
        }
        let unit_id = members
            .iter()
            .map(|id| {
                state
                    .entities
                    .row(state.entities.slot_of(*id).expect("eligible member"))
                    .uuid
            })
            .min()
            .expect("a migrating unit has members");
        if let Some(destination) = best.1 {
            let anchor = state.settlements.get(destination).unwrap().anchor;
            if let Some(target) = approach(state, anchor) {
                match unit_route(state, &members, target) {
                    Proof::Reachable => {
                        match relocate(state, source, destination, &unit) {
                            Ok(()) => {
                                notice(
                                    state,
                                    source,
                                    "departure",
                                    unit_id,
                                    &format!(
                                        "{} residents relocated to settlement {}",
                                        members.len(),
                                        destination.0
                                    ),
                                );
                                notice(
                                    state,
                                    destination,
                                    "arrival",
                                    unit_id,
                                    &format!(
                                        "{} residents joined from settlement {} and are walking home",
                                        members.len(),
                                        source.0
                                    ),
                                );
                            }
                            Err(reason) => {
                                notice(state, source, "cancel", unit_id, reason);
                            }
                        }
                        return;
                    }
                    Proof::Deferred => {
                        reason = "route budget deferred";
                        break;
                    }
                    Proof::Blocked => {}
                }
            }
        }
        let bearing = best
            .1
            .map(|destination| {
                transit::bearing(
                    source_anchor,
                    state.settlements.get(destination).unwrap().anchor,
                )
            })
            .unwrap_or_else(|| (rng.unit() * 360.0) as f32);
        let exit = match proven_edge(state, source_anchor, bearing, EdgeTravel::Leaving(&members)) {
            Ok(exit) => exit,
            Err(Proof::Deferred) => {
                reason = "exit route budget deferred";
                break;
            }
            Err(_) => {
                reason = "no reachable simulated exit";
                continue;
            }
        };
        let uuids: Vec<_> = members
            .iter()
            .map(|id| {
                state
                    .entities
                    .row(state.entities.slot_of(*id).unwrap())
                    .uuid
            })
            .collect();
        for &uuid in &uuids {
            state.settlements.migration.targets.insert(uuid, exit);
        }
        state.settlements.migration.walking.insert(
            source,
            Walk {
                unit,
                members: uuids,
                destination: best.1,
                exit,
                bearing,
                deadline: state.tick + WALK_TIMEOUT,
            },
        );
        state.settlements.migration.decisions.insert(
            source,
            format!(
                "outbound walk to {:?}, destination {:?}, gain {:.1}",
                exit, best.1, best.0
            ),
        );
        return;
    }
    state
        .settlements
        .migration
        .decisions
        .insert(source, reason.into());
}

fn outside_arrivals(
    state: &mut TickState,
    changes: &mut ChangeSet,
    destination: SettlementId,
    rng: &mut Rng,
) {
    let score = state
        .settlements
        .migration
        .observation(destination)
        .unwrap()
        .attractiveness
        .total();
    let advantage = score - OUTSIDE_ATTRACTIVENESS;
    if advantage <= MIGRATION_THRESHOLD {
        return;
    }
    let extra = ((advantage - MIGRATION_THRESHOLD) / SCORE_SCALE).min(EXTRA_ARRIVAL_MAX_CHANCE);
    let expected = (1.0 + extra) * state.test_acceleration.migration_rate;
    let whole = expected.floor();
    let count = (whole as usize)
        .saturating_add(usize::from(rng.unit() < expected - whole))
        .min(capacity(state, destination));
    let anchor = state.settlements.get(destination).unwrap().anchor;
    let Some(goal) = approach(state, anchor) else {
        return;
    };
    let Some(type_id) = state.registry.entity_types.id_of(
        &mc_data::Identifier::parse(mc_data::entity_types::VILLAGER)
            .expect("constant villager identifier"),
    ) else {
        return;
    };
    for _ in 0..count {
        let bearing = (rng.unit() * 360.0) as f32;
        let Ok(entry) = proven_edge(state, anchor, bearing, EdgeTravel::Entering(goal)) else {
            break;
        };
        let position = allocator::centre(entry);
        let entity = state.spawn_mob(type_id, position, ADULT_HEALTH);
        super::admit_arrival(state, destination, entity)
            .expect("fresh adult, existing route-proven settlement");
        changes.entities_spawned.push(entity);
        let slot = state.entities.slot_of(entity).unwrap();
        let uuid = state.entities.row(slot).uuid;
        tracing::info!(resident=%state.entities.row(slot).uuid, destination=destination.0, entry=?entry, "settlement_outside_arrival");
        notice(
            state,
            destination,
            "arrival",
            uuid,
            "an adult immigrant joined from the outside world",
        );
    }
}

fn advance_walks(state: &mut TickState, changes: &mut ChangeSet) {
    let mut sources = std::mem::take(&mut state.settlements.migration.walking_sources);
    sources.clear();
    sources.extend(state.settlements.migration.walking.keys().copied());
    sources.sort_unstable();
    for &source in &sources {
        let walk = state.settlements.migration.walking.remove(&source).unwrap();
        let all_near = walk.members.iter().all(|uuid| {
            state.entities.slot_of_uuid(*uuid).is_some_and(|slot| {
                let pos = state.entities.pos(slot);
                mth::length2(
                    pos[0] - (f64::from(walk.exit.x) + 0.5),
                    pos[2] - (f64::from(walk.exit.z) + 0.5),
                ) <= transit::EXIT_REACH
            })
        });
        let missing = walk.members.iter().any(|uuid| {
            state.entities.slot_of_uuid(*uuid).is_none_or(|slot| {
                !state.entities.body(slot).alive
                    || state
                        .entities
                        .row(slot)
                        .villager
                        .as_ref()
                        .is_none_or(|v| v.settlement != Some(source))
            })
        });
        if !all_near && !missing && state.tick < walk.deadline {
            state.settlements.migration.walking.insert(source, walk);
            continue;
        }
        for uuid in &walk.members {
            state.settlements.migration.targets.remove(uuid);
        }
        let result = if missing {
            Err("member died, left or changed dimension")
        } else if !all_near {
            Err("outbound walk timed out")
        } else {
            eligible(state, source, &walk.unit, walk.destination.is_none()).and_then(|members| {
                if members.len() != walk.members.len()
                    || members.iter().any(|id| {
                        !walk.members.contains(
                            &state
                                .entities
                                .row(state.entities.slot_of(*id).unwrap())
                                .uuid,
                        )
                    })
                {
                    return Err("household changed during outbound walk");
                }
                if let Some(destination) = walk.destination {
                    if capacity(state, destination) < members.len() {
                        return Err("destination has no free beds");
                    }
                    transit::depart(
                        state,
                        changes,
                        source,
                        walk.unit.borrowed(),
                        destination,
                        walk.exit,
                        walk.bearing,
                    )
                    .map(|_| ())
                } else {
                    transit::emigrate(state, changes, source, walk.unit.borrowed(), walk.exit)
                }
            })
        };
        let unit_id = *walk
            .members
            .iter()
            .min()
            .expect("a walking unit has members");
        match result {
            Ok(()) => {
                tracing::info!(source=source.0, destination=?walk.destination, members=?walk.members, tick=state.tick, game_time=state.tick_control.game_ticks(state.tick), "settlement_migration_departed");
                notice(
                    state,
                    source,
                    "departure",
                    unit_id,
                    &format!(
                        "{} residents left for {}",
                        walk.members.len(),
                        walk.destination.map_or_else(
                            || "the outside world".into(),
                            |id| format!("settlement {}", id.0)
                        )
                    ),
                );
            }
            Err(reason) => notice(state, source, "cancel", unit_id, reason),
        }
    }
    sources.clear();
    state.settlements.migration.walking_sources = sources;
}

pub(crate) fn tick(state: &mut TickState, changes: &mut ChangeSet, arrivals: &[transit::Arrival]) {
    let now = state.tick_control.game_ticks(state.tick);
    for event in &changes.entity_events {
        if let EntityEvent::Death { entity } = *event
            && let Some(id) = state
                .entities
                .slot_of(entity)
                .and_then(|slot| state.entities.row(slot).villager.as_ref())
                .and_then(|v| v.settlement)
        {
            state
                .settlements
                .migration
                .deaths
                .entry(id)
                .or_default()
                .push_back(now);
        }
    }
    for arrival in arrivals {
        let unit_id = *arrival
            .members
            .first()
            .expect("a successful arrival has members");
        notice(
            state,
            arrival.destination,
            "arrival",
            unit_id,
            &format!(
                "{} residents arrived from settlement {}",
                arrival.members.len(),
                arrival.source.0
            ),
        );
    }
    advance_walks(state, changes);
    if now.is_multiple_of(DAY_TICKS) {
        let entities = &state.entities;
        state.settlements.migration.returning.retain(|uuid, _| {
            entities.slot_of_uuid(*uuid).is_some_and(|slot| {
                entities.body(slot).alive
                    && entities
                        .row(slot)
                        .villager
                        .as_ref()
                        .is_some_and(|v| v.settlement.is_some() && v.home.is_none())
            })
        });
    }
    let due_ids: Vec<_> = state
        .settlements
        .records()
        .filter(|r| due(r.id, now) && simulated(state, r.anchor))
        .map(|r| r.id)
        .collect();
    for id in due_ids {
        if let Some(deaths) = state.settlements.migration.deaths.get_mut(&id) {
            while deaths
                .front()
                .is_some_and(|tick| now.saturating_sub(*tick) >= SAFETY_DAYS * DAY_TICKS)
            {
                deaths.pop_front();
            }
        }
        let facts = facts(state, id, now);
        state.settlements.migration.observe(id, now, facts);
        let mut rng = Rng(id.0 ^ now.rotate_left(17));
        depart_decision(state, id, &mut rng);
        outside_arrivals(state, changes, id, &mut rng);
    }
}

pub(crate) fn info(state: &TickState, id: SettlementId) -> String {
    let mut message = match state.settlements.migration.observation(id) {
        Some(o) => format!(
            "Migration A={:.1} (housing={:.2} food={:.2} jobs={:.2} safety={:.2} development={:.2}); members={} beds={} inbound={} food_points={} open_jobs={} recent_deaths={} buildings={} observed_at={}",
            o.attractiveness.total(),
            o.attractiveness.housing,
            o.attractiveness.food,
            o.attractiveness.jobs,
            o.attractiveness.safety,
            o.attractiveness.development,
            o.facts.members,
            o.facts.beds,
            transit::inbound(state, id),
            o.facts.food_points,
            o.facts.open_jobs,
            o.facts.recent_deaths,
            o.facts.completed_buildings,
            o.game_time
        ),
        None => "Migration A=unknown (not evaluated since start)".into(),
    };
    if let Some(reason) = state.settlements.migration.decisions.get(&id) {
        message.push_str(&format!("\nLast decision: {reason}"));
    }
    for record in transit::summary(state, id) {
        message.push_str(&format!(
            "\nTransit {}: {} -> {}, household={}, members={}, departed={}, arrives={}, {:?}",
            record.id.0,
            record.source.0,
            record.destination.0,
            record.household.as_deref().unwrap_or("none"),
            record.members,
            record.departed,
            record.arrives,
            record.status
        ));
    }
    message
}
