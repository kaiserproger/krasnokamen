//! Runtime-only refusal evidence from the canonical site search. Complete
//! sweeps, not cursor batches, determine whether the reason has changed.
use std::collections::{BTreeMap, BTreeSet};

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub(super) enum Reason {
    NoFrontage,
    LotGeometry,
    OutpostClaim,
    BuildingBuffer,
    Territory,
    SurfaceUnavailable,
    ProtectedFloor,
    Unloaded,
    PlantingProvenance,
    ProtectedObstruction,
    AmbiguousFoliage,
    CutBeyondEnvelope,
    LiquidInWork,
    LavaShell,
    ExecutorReservation,
    Preparation,
    MaterialBill,
}
const REASONS: [Reason; 17] = [
    Reason::NoFrontage,
    Reason::LotGeometry,
    Reason::OutpostClaim,
    Reason::BuildingBuffer,
    Reason::Territory,
    Reason::SurfaceUnavailable,
    Reason::ProtectedFloor,
    Reason::Unloaded,
    Reason::PlantingProvenance,
    Reason::ProtectedObstruction,
    Reason::AmbiguousFoliage,
    Reason::CutBeyondEnvelope,
    Reason::LiquidInWork,
    Reason::LavaShell,
    Reason::ExecutorReservation,
    Reason::Preparation,
    Reason::MaterialBill,
];

#[derive(Default)]
pub(super) struct Report {
    pub(super) frontages: u32,
    pub(super) examined: u32,
    pub(super) counts: [u32; REASONS.len()],
    pub(super) examples: [Option<[i32; 3]>; REASONS.len()],
}
impl std::fmt::Debug for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut map = f.debug_map();
        for reason in REASONS {
            let index = reason as usize;
            if self.counts[index] != 0 {
                map.entry(&reason, &(self.counts[index], self.examples[index]));
            }
        }
        map.finish()
    }
}
impl Report {
    pub(super) fn reject(&mut self, reason: Reason, at: [i32; 3]) {
        let index = reason as usize;
        self.counts[index] += 1;
        self.examples[index].get_or_insert(at);
    }
    fn mask(&self) -> u32 {
        self.counts
            .iter()
            .enumerate()
            .fold(0, |mask, (index, &count)| {
                mask | (u32::from(count != 0) << index)
            })
    }
}

#[derive(Debug, Default)]
pub(crate) struct Watch {
    blueprint: String,
    sweep: Report,
    previous: Option<u32>,
}
impl Watch {
    fn record(&mut self, blueprint: &str, batch: Report) -> Option<Report> {
        if self.blueprint != blueprint {
            *self = Self {
                blueprint: blueprint.to_owned(),
                ..Self::default()
            };
        }
        if self.sweep.frontages != batch.frontages {
            self.sweep = Report::default();
        }
        self.sweep.frontages = batch.frontages;
        self.sweep.examined += batch.examined;
        for index in 0..REASONS.len() {
            self.sweep.counts[index] += batch.counts[index];
            if self.sweep.examples[index].is_none() {
                self.sweep.examples[index] = batch.examples[index];
            }
        }
        if self.sweep.examined < self.sweep.frontages {
            return None;
        }
        let report = std::mem::take(&mut self.sweep);
        // Retain the topology between sweeps so the first batch does not
        // invalidate an otherwise complete accumulation.
        self.sweep.frontages = report.frontages;
        let mask = report.mask();
        if self.previous == Some(mask) {
            return None;
        }
        self.previous = Some(mask);
        Some(report)
    }
}

pub(super) fn observe(state: &mut TickState, id: SettlementId, blueprint: &str, batch: Report) {
    if let Some(report) = state
        .settlements
        .site_watch
        .entry(id)
        .or_default()
        .record(blueprint, batch)
    {
        tracing::warn!(settlement = id.0, blueprint, level = 1,
            frontages = report.frontages, examined = report.examined,
            refusals = ?report, "settlement_construction_site_refused");
    }
}

/// Solid work the bounded terrain window would rewrite on this lot: cells the
/// plan fills below the authored floor plus cells it excavates inside the
/// authored volume. Lot ranking compares this sum; only the winner is planned.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct Cost {
    pub(super) cut: u32,
    pub(super) fill: u32,
}
impl Cost {
    pub(super) fn total(self) -> u32 {
        self.cut.saturating_add(self.fill)
    }
}

/// One blueprint sweep of the whole street cursor. Runtime-only scratch:
/// best metadata, progress and nothing the world can be rebuilt from.
#[derive(Debug)]
struct Sweep {
    frontages: u32,
    start: u32,
    next: u32,
    best: Option<([i32; 3], u8, u32)>,
}

/// Per-settlement live sweeps, keyed by blueprint id.
#[derive(Debug, Default)]
pub(crate) struct Sweeps {
    live: BTreeMap<String, Sweep>,
}
impl Sweeps {
    pub(super) fn clear(&mut self) {
        self.live.clear();
    }
    /// Drop sweeps of a topology that no longer exists.
    fn retain(&mut self, blueprint: &str, frontages: u32) {
        self.live
            .retain(|id, sweep| !sweep_stale(id, sweep, blueprint, frontages));
    }
}

fn sweep_stale(id: &str, sweep: &Sweep, blueprint: &str, frontages: u32) -> bool {
    // Another kind's finished topology stays cheap to keep; only this kind's
    // own frontage count invalidates it.
    id == blueprint && sweep.frontages != frontages
}

pub(super) enum Advance {
    /// The sweep is unfinished; nothing may be admitted from it yet.
    Incomplete(u32),
    /// A complete sweep found no legal lot.
    Empty,
    /// The complete sweep's cheapest legal lot, in cursor order on ties.
    Cheapest([i32; 3], u8),
}

/// A completed sweep's winner and the funding variant chosen for its kind.
pub(super) struct Winner<'a> {
    pub(super) blueprint: &'a blueprints::Blueprint,
    pub(super) origin: [i32; 3],
    pub(super) rotation: u8,
    pub(super) species: &'a str,
    pub(super) color: u8,
}

/// Revalidate the winner once against the current world, then materialize it
/// exactly once. A winner that no longer passes is reported and dropped.
pub(super) fn materialize(
    state: &TickState,
    record: &SettlementRecord,
    winner: Winner<'_>,
    floors: &BTreeSet<[i32; 2]>,
    clearance: &mut super::super::forestry::Clearance,
    report: &mut Report,
) -> Option<super::FundedSite> {
    let size = winner
        .blueprint
        .footprint(Rotation::from_quarter_turns(winner.rotation));
    if let Err(reason) = check(
        state,
        record,
        winner.origin,
        size,
        record.founding.as_ref(),
        clearance,
        Some(floors),
    ) {
        report.reject(reason, winner.origin);
        return None;
    }
    let project = Project {
        settlement: record.id,
        blueprint: winner.blueprint.id.clone(),
        level: 1,
        origin: winner.origin,
        rotation: winner.rotation,
        building: None,
        structure: String::new(),
        ordered: false,
        borrowed_builder: None,
    };
    let Ok((bounds, mut cells)) = planned_cells(state, &project, winner.species, clearance) else {
        report.reject(Reason::Preparation, winner.origin);
        return None;
    };
    if apply_bed_color(state, &mut cells, winner.color).is_none() {
        report.reject(Reason::MaterialBill, winner.origin);
        return None;
    }
    Some(super::FundedSite {
        project,
        bounds,
        cells,
    })
}

/// Columns whose own authored level-one cell already sits on the floor: the
/// plan pays those cells once, as a placed block, never as an extra fill.
pub(super) fn floor_columns(blueprint: &blueprints::Blueprint, rotation: u8) -> BTreeSet<[i32; 2]> {
    let Some(level) = blueprint.level(1) else {
        return BTreeSet::new();
    };
    level
        .rotated(Rotation::from_quarter_turns(rotation))
        .blocks
        .iter()
        .filter(|(at, _)| at[1] == 0)
        .map(|(at, _)| [i32::from(at[0]), i32::from(at[2])])
        .collect()
}

/// Examine up to `budget` further candidates of this blueprint's sweep. An
/// incomplete sweep admits nothing; a completed one returns its cheapest lot
/// and forgets its scratch. Equal costs keep the earlier cursor position.
#[allow(clippy::too_many_arguments)]
pub(super) fn advance(
    state: &TickState,
    record: &SettlementRecord,
    network: &mut layout::Network<'_>,
    blueprint: &blueprints::Blueprint,
    floors: &[BTreeSet<[i32; 2]>; 4],
    clearance: &mut super::super::forestry::Clearance,
    sweeps: &mut Sweeps,
    start: u32,
    budget: &mut u32,
    report: &mut Report,
) -> Advance {
    let fronts = network.frontage();
    let count = u32::try_from(fronts.len()).unwrap_or(u32::MAX);
    if count == 0 {
        report.reject(Reason::NoFrontage, record.anchor);
        return Advance::Empty;
    }
    sweeps.retain(&blueprint.id, count);
    let start = start % count;
    let sweep = sweeps.live.entry(blueprint.id.clone()).or_insert(Sweep {
        frontages: count,
        start,
        next: 0,
        best: None,
    });
    while sweep.next < sweep.frontages && *budget > 0 {
        let index = (sweep.start + sweep.next) % sweep.frontages;
        sweep.next += 1;
        *budget -= 1;
        report.examined += 1;
        let (front, normal) = fronts[index as usize];
        let Some(lot) = network.lot(blueprint, front, normal) else {
            report.reject(Reason::LotGeometry, front);
            continue;
        };
        let rotation = lot.rotation;
        let size = blueprint.footprint(Rotation::from_quarter_turns(rotation));
        match check(
            state,
            record,
            lot.origin,
            size,
            record.founding.as_ref(),
            clearance,
            Some(&floors[usize::from(rotation) % 4]),
        ) {
            Err(reason) => report.reject(reason, lot.origin),
            Ok(cost) => {
                let cost = cost.total();
                if sweep.best.is_none_or(|(_, _, known)| cost < known) {
                    sweep.best = Some((lot.origin, rotation, cost));
                }
            }
        }
    }
    if sweep.next < sweep.frontages {
        let next = (sweep.start + sweep.next) % sweep.frontages;
        return Advance::Incomplete(next);
    }
    let best = sweeps
        .live
        .remove(&blueprint.id)
        .and_then(|sweep| sweep.best);
    match best {
        Some((origin, rotation, _)) => Advance::Cheapest(origin, rotation),
        None => Advance::Empty,
    }
}

fn tree_refusal(state: &TickState, at: [i32; 3]) -> Reason {
    let Some(block) = state.world.physics_block_at(at[0], at[1], at[2]) else {
        return Reason::Unloaded;
    };
    if state.block_log.player_touched(at) || super::super::forestry::persistent(state, block) {
        return Reason::ProtectedObstruction;
    }
    if super::super::forestry::planted_crown_overlaps(
        state,
        ScriptWorldBounds::new(at, at).expect("cell"),
    ) {
        if state
            .registry
            .blocks
            .by_id(mc_world::BlockStateId(block))
            .is_some_and(|b| b.block.id.path().ends_with("_leaves"))
        {
            Reason::AmbiguousFoliage
        } else {
            Reason::PlantingProvenance
        }
    } else {
        Reason::ProtectedObstruction
    }
}

fn definition(state: &TickState, block: u32) -> Option<&mc_world::BlockState> {
    state.registry.blocks.by_id(mc_world::BlockStateId(block))
}

fn fluid(state: &TickState, block: u32) -> bool {
    definition(state, block).is_some_and(|definition| definition.has_fluid())
}

fn lava(state: &TickState, at: [i32; 3]) -> bool {
    state
        .world
        .physics_block_at(at[0], at[1], at[2])
        .is_some_and(|block| state.registry.states.lava_levels.contains(&block))
}

/// One-cell shell around the work volume: lava there would flow into a cut.
fn lava_shell(state: &TickState, volume: ScriptWorldBounds) -> bool {
    let [x0, y0, z0] = volume.min.map(|value| value - 1);
    let [x1, y1, z1] = volume.max.map(|value| value + 1);
    for x in x0..=x1 {
        for z in z0..=z1 {
            let edge = x == x0 || x == x1 || z == z0 || z == z1;
            if !edge {
                if lava(state, [x, y0, z]) || lava(state, [x, y1, z]) {
                    return true;
                }
                continue;
            }
            if (y0..=y1).any(|y| lava(state, [x, y, z])) {
                return true;
            }
        }
    }
    false
}

/// `floors` are the footprint columns of this candidate's own authored floor.
pub(super) fn check(
    state: &TickState,
    record: &SettlementRecord,
    origin: [i32; 3],
    size: [u8; 3],
    expedition: Option<&FoundingExpedition>,
    clearance: &mut super::super::forestry::Clearance,
    floors: Option<&BTreeSet<[i32; 2]>>,
) -> Result<Cost, Reason> {
    if founding::site_reserved(state, record.id, origin, size) {
        return Err(Reason::OutpostClaim);
    }
    let max: [i32; 3] = std::array::from_fn(|axis| origin[axis] + i32::from(size[axis]) - 1);
    let lot = ScriptWorldBounds::new(origin, max).expect("blueprint bounds");
    // Support discovery is centered on the candidate floor, and the whole
    // terrain window is bounded: fill at most SITE_BAND below that floor and
    // cut only inside the authored level. Anything else is a named refusal.
    let floor = origin[1];
    let mut lowest = floor;
    let mut cost = Cost::default();
    if record.buildings.values().any(|building| {
        !building.natural_zone
            && [0, 2].into_iter().all(|axis| {
                origin[axis] <= building.bounds.max[axis] + 1
                    && building.bounds.min[axis] <= max[axis] + 1
            })
    }) {
        return Err(Reason::BuildingBuffer);
    }
    if super::super::forestry::off_lot_sapling_overlaps(state, lot) {
        return Err(Reason::PlantingProvenance);
    }
    for x in origin[0]..=max[0] {
        for z in origin[2]..=max[2] {
            if [x, max[1], z]
                .into_iter()
                .zip(record.anchor)
                .map(|(value, anchor)| (i64::from(value) - i64::from(anchor)).pow(2))
                .sum::<i64>()
                > i64::from(record.radius).pow(2)
                && !expedition.is_some_and(|site| site.contains([x, max[1], z]))
            {
                return Err(Reason::Territory);
            }
            let height =
                surface_y_for_lot(state, floor, x, z, Some(lot), clearance).ok_or_else(|| {
                    (floor - SITE_BAND + 1..=floor + SITE_BAND + 1)
                        .find(|&y| {
                            state.world.physics_block_at(x, y, z).is_some_and(|block| {
                                super::super::forestry::tree_or_sapling(state, block)
                                    && !clearance.clearable(state, [x, y, z], Some(lot))
                            })
                        })
                        .map_or(Reason::SurfaceUnavailable, |y| {
                            tree_refusal(state, [x, y, z])
                        })
                })?;
            lowest = lowest.min(height);
            let ground = state
                .world
                .physics_block_at(x, height, z)
                .expect("loaded surface");
            if !floor_replaceable(state, ground) {
                return Err(Reason::ProtectedFloor);
            }
            let authored_floor =
                floors.is_some_and(|floors| floors.contains(&[x - origin[0], z - origin[2]]));
            // The floor cell is the building's own level: an authored block
            // pays for it, otherwise the terrain there must be replaceable. A
            // fluid, a path, farmland or a paid crop is not.
            if !authored_floor {
                let floor_block = state
                    .world
                    .physics_block_at(x, floor, z)
                    .ok_or(Reason::Unloaded)?;
                if !floor_replaceable(state, floor_block) {
                    return Err(Reason::ProtectedFloor);
                }
            }
            // Fill: real blocks in air, liquid-free cells between the support
            // and the floor. A solid there is a cut this window does not own,
            // and a fluid would be overwritten by a written block.
            for y in height + 1..floor {
                let block = state
                    .world
                    .physics_block_at(x, y, z)
                    .ok_or(Reason::Unloaded)?;
                if fluid(state, block) {
                    return Err(Reason::LiquidInWork);
                }
                if !clearable(state, block) {
                    return Err(Reason::CutBeyondEnvelope);
                }
                if block != state.registry.states.air && state.block_log.player_touched([x, y, z]) {
                    return Err(Reason::ProtectedObstruction);
                }
                cost.fill += 1;
            }
            if height < floor && !authored_floor {
                // The floor cell itself is one more real dirt; floor_replaceable
                // already allowed solid ground there.
                cost.fill += 1;
            }
            // Authored volume: the only place this slice may cut terrain.
            for y in floor + 1..=max[1] {
                let block = state
                    .world
                    .physics_block_at(x, y, z)
                    .ok_or(Reason::Unloaded)?;
                if fluid(state, block) {
                    return Err(Reason::LiquidInWork);
                }
                let tree = super::super::forestry::tree_or_sapling(state, block);
                let tree_clearable = tree && clearance.clearable(state, [x, y, z], Some(lot));
                if state.block_entities.contains_key(&(x, y, z))
                    || (tree && !tree_clearable)
                    || (!clearable(state, block) && !excavatable(state, block) && !tree_clearable)
                {
                    return Err(tree_refusal(state, [x, y, z]));
                }
                if block != state.registry.states.air && !tree {
                    if state.block_log.player_touched([x, y, z]) {
                        return Err(Reason::ProtectedObstruction);
                    }
                    cost.cut += 1;
                }
            }
            // A hillside above the authored roof is beyond this window too:
            // the plan would have to cut it to expose the building.
            for y in max[1] + 1..=height {
                let block = state
                    .world
                    .physics_block_at(x, y, z)
                    .ok_or(Reason::Unloaded)?;
                let tree = super::super::forestry::tree_or_sapling(state, block);
                if tree {
                    if !clearance.clearable(state, [x, y, z], Some(lot)) {
                        return Err(tree_refusal(state, [x, y, z]));
                    }
                    continue;
                }
                if block != state.registry.states.air && !clearable(state, block) {
                    return Err(Reason::CutBeyondEnvelope);
                }
            }
        }
    }
    let volume = ScriptWorldBounds::new([origin[0], lowest, origin[2]], max).expect("work volume");
    if lava_shell(state, volume) {
        return Err(Reason::LavaShell);
    }
    if executor::area_reserved(state, volume) {
        return Err(Reason::ExecutorReservation);
    }
    Ok(cost)
}

#[cfg(test)]
#[path = "construction_site_refusal_tests.rs"]
mod tests;
