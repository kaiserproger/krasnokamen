//! Runtime-only refusal evidence from the canonical site search. Complete
//! sweeps, not cursor batches, determine whether the reason has changed.
use std::collections::{BTreeMap, BTreeSet};

use super::preparation::liquid;
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub(super) enum Reason {
    NoFrontage,
    LotGeometry,
    OutpostClaim,
    BuildingBuffer,
    Territory,
    HeightVariance,
    SurfaceUnavailable,
    ProtectedFloor,
    Unloaded,
    PlantingProvenance,
    ProtectedObstruction,
    AmbiguousFoliage,
    CutBeyondEnvelope,
    LiquidInWork,
    LavaShell,
    LiquidOpenShell,
    MixedLiquids,
    CombustibleShell,
    LiquidBeyondBound,
    LiquidAccess,
    NoPathDetour,
    ExecutorReservation,
    Preparation,
    MaterialBill,
    StorageFull,
    /// The upgrade's placement cell holds neither air nor the previous build's
    /// own last committed block: the world there is someone else's.
    PreviousBuildDiverged,
}
const REASONS: [Reason; 26] = [
    Reason::NoFrontage,
    Reason::LotGeometry,
    Reason::OutpostClaim,
    Reason::BuildingBuffer,
    Reason::Territory,
    Reason::HeightVariance,
    Reason::SurfaceUnavailable,
    Reason::ProtectedFloor,
    Reason::Unloaded,
    Reason::PlantingProvenance,
    Reason::ProtectedObstruction,
    Reason::AmbiguousFoliage,
    Reason::CutBeyondEnvelope,
    Reason::LiquidInWork,
    Reason::LavaShell,
    Reason::LiquidOpenShell,
    Reason::MixedLiquids,
    Reason::CombustibleShell,
    Reason::LiquidBeyondBound,
    Reason::LiquidAccess,
    Reason::NoPathDetour,
    Reason::ExecutorReservation,
    Reason::Preparation,
    Reason::MaterialBill,
    Reason::StorageFull,
    Reason::PreviousBuildDiverged,
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

/// Queue reason of a funded new-building order whose bounded site sweep is
/// still running: later planning passes continue it.
pub(super) const WAITING_FOR_SITE: &str = "waiting for a site";

/// Admission refusal of a pass that found no site: a sweep that is still
/// running waits for a site, a finished one has no free lot.
pub(super) fn miss(running: bool) -> String {
    if running {
        WAITING_FOR_SITE.into()
    } else {
        "No free loaded construction site is available within the material budget.".into()
    }
}

/// Solid work the bounded terrain window would rewrite on this lot: cells the
/// plan fills below the authored floor plus cells it excavates inside the
/// authored volume. A fluid fill cell is a fill cell and belongs to exactly one
/// of `liquid`/`water`/`lava`, never also to `fill`: the material is counted
/// once and the liquid figures only rank equal work. `lava` first, then `water`,
/// break the tie, so the queue head is not parked on a pool it must drain.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct Cost {
    pub(super) cut: u32,
    pub(super) fill: u32,
    pub(super) liquid: u32,
    pub(super) water: u32,
    pub(super) lava: u32,
}
impl Cost {
    /// Cells of work; a fluid fill is one cell, exactly like a dry fill.
    pub(super) fn total(self) -> u32 {
        self.cut
            .saturating_add(self.fill)
            .saturating_add(self.liquid)
    }
    /// Ranking key inside one completed sweep: least work, then the least lava,
    /// then the least water.
    pub(super) fn rank(self) -> (u32, u32, u32) {
        (self.total(), self.lava, self.water)
    }
}

/// One blueprint sweep of the whole street cursor. Runtime-only scratch:
/// best metadata, progress and nothing the world can be rebuilt from.
#[derive(Debug)]
struct Sweep {
    frontages: u32,
    /// Identity of the ordered frontage input this sweep walked; a different
    /// list of the same length invalidates the stored ranking too.
    topology: u64,
    start: u32,
    next: u32,
    best: Option<([i32; 3], u8, Cost)>,
}

/// Per-settlement live sweeps, keyed by blueprint id.
#[derive(Debug, Default)]
pub(crate) struct Sweeps {
    live: BTreeMap<String, Sweep>,
}
impl Sweeps {
    /// Drop sweeps of a topology that no longer exists.
    fn retain(&mut self, blueprint: &str, frontages: u32, topology: u64) {
        self.live
            .retain(|id, sweep| !sweep_stale(id, sweep, blueprint, frontages, topology));
    }

    /// Some unfinished sweep already holds a legal candidate.
    pub(super) fn waiting(&self) -> bool {
        self.live.values().any(|sweep| sweep.best.is_some())
    }

    /// Candidates this blueprint's live sweep has examined (test observer).
    #[cfg(test)]
    pub(super) fn examined(&self, blueprint: &str) -> Option<u32> {
        self.live.get(blueprint).map(|sweep| sweep.next)
    }
}

fn sweep_stale(id: &str, sweep: &Sweep, blueprint: &str, frontages: u32, topology: u64) -> bool {
    // Another kind's finished topology stays cheap to keep; only this kind's
    // own ordered frontage input invalidates it.
    id == blueprint && (sweep.frontages != frontages || sweep.topology != topology)
}

/// Identity of the ordered frontage list a sweep walked.
pub(super) fn topology(fronts: &[([i32; 3], [i32; 2])]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for (at, normal) in fronts {
        for byte in at
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .chain(normal.iter().flat_map(|value| value.to_le_bytes()))
        {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

/// Squared distance between two block positions.
fn distance_sq(a: [i32; 3], b: [i32; 3]) -> i64 {
    a.into_iter()
        .zip(b)
        .map(|(a, b)| (i64::from(a) - i64::from(b)).pow(2))
        .sum()
}

#[derive(Debug)]
pub(super) enum Advance {
    /// The sweep is unfinished; nothing may be admitted from it yet.
    Incomplete(u32),
    /// A complete sweep found no legal lot.
    Empty,
    /// The complete sweep's cheapest legal lot; equal costs go to the lot
    /// nearest the anchor, then to the lower block position and rotation.
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
    survey: &mut Survey,
    images: &[super::super::inventory::Image],
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
        survey,
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
        builder: None,
        order_seq: None,
    };
    let Ok((bounds, mut cells)) =
        planned_cells(state, &project, winner.species, &mut survey.clearance)
    else {
        report.reject(Reason::Preparation, winner.origin);
        return None;
    };
    // The filler identity is chosen before the bill is fixed: after the authored
    // bill and its crafting chain are reserved (item 2).
    super::filler::apply_filler(state, &project, &mut cells, images);
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

#[cfg(test)]
thread_local! {
    /// Candidate lots examined on this thread (cost observer).
    pub(super) static EXAMINED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Count one examined candidate lot on this thread (tests' cost observer).
pub(super) fn examined() {
    #[cfg(test)]
    EXAMINED.with(|count| count.set(count.get() + 1));
}

/// Examine up to `budget` further candidates of this blueprint's sweep. An
/// incomplete sweep admits nothing; a completed one returns its cheapest lot
/// and forgets its scratch. Equal costs go to the lot nearest the anchor.
#[allow(clippy::too_many_arguments)]
pub(super) fn advance(
    state: &TickState,
    record: &SettlementRecord,
    network: &mut layout::Network<'_>,
    blueprint: &blueprints::Blueprint,
    floors: &[BTreeSet<[i32; 2]>; 4],
    survey: &mut Survey,
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
    let topology = topology(&fronts);
    sweeps.retain(&blueprint.id, count, topology);
    let start = start % count;
    let sweep = sweeps.live.entry(blueprint.id.clone()).or_insert(Sweep {
        frontages: count,
        topology,
        start,
        next: 0,
        best: None,
    });
    while sweep.next < sweep.frontages && *budget > 0 {
        let index = (sweep.start + sweep.next) % sweep.frontages;
        sweep.next += 1;
        *budget -= 1;
        report.examined += 1;
        examined();
        let (front, normal) = fronts[index as usize];
        let Some(lot) = network.lot(blueprint, front, normal) else {
            // The lot's own door or verge column holds fluid: the builder
            // cannot reach it. Named separately from a plain geometry miss so
            // the backlog item is visible in the refusal report (owner D21).
            if liquid::door_fluid(state, front, normal, record.anchor[1]) {
                report.reject(Reason::LiquidAccess, front);
            } else {
                report.reject(Reason::LotGeometry, front);
            }
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
            survey,
            Some(&floors[usize::from(rotation) % 4]),
        ) {
            Err(reason) => report.reject(reason, lot.origin),
            Ok(cost) => {
                // Ties go to the lot nearest the settlement's anchor, then to
                // its own block position: both are functions of the lot alone,
                // so a restart cannot change the winner, and flat ground grows
                // the village from its bell rather than from a plan edge.
                let key = (
                    cost.rank(),
                    distance_sq(lot.origin, record.anchor),
                    lot.origin,
                    rotation,
                );
                if sweep.best.is_none_or(|(origin, rotation, known)| {
                    key < (
                        known.rank(),
                        distance_sq(origin, record.anchor),
                        origin,
                        rotation,
                    )
                }) {
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
    let Some((origin, rotation, stored)) = best else {
        return Advance::Empty;
    };
    // The world may have moved since that candidate was ranked: re-check it
    // now and restart this sweep when its cost changed. The pass's survey
    // already describes this world, so a registered root is proved once.
    let size = blueprint.footprint(Rotation::from_quarter_turns(rotation));
    match check(
        state,
        record,
        origin,
        size,
        record.founding.as_ref(),
        survey,
        Some(&floors[usize::from(rotation) % 4]),
    ) {
        Ok(cost) if cost == stored => Advance::Cheapest(origin, rotation),
        Ok(_) | Err(_) => Advance::Incomplete(0),
    }
}

/// `property`: the cell is inside the working settlement's territory (owner
/// D27.4); there player history and the persistent-leaf mark name nothing.
fn tree_refusal(state: &TickState, at: [i32; 3], property: bool) -> Reason {
    let Some(block) = state.world.physics_block_at(at[0], at[1], at[2]) else {
        return Reason::Unloaded;
    };
    if !property
        && (state.block_log.player_touched(at) || super::super::forestry::persistent(state, block))
    {
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

/// Deepest foundation a column pays for unless every cell of its fill lies
/// inside the territory; a deeper column would be a retaining wall (committee
/// ruling).
const MAX_FILL_DEPTH: i32 = 4;

/// One-cell shell around the work volume: a fluid there would flow into a cut.
fn shell_fluid(state: &TickState, volume: ScriptWorldBounds) -> bool {
    let [x0, y0, z0] = volume.min.map(|value| value - 1);
    let [x1, y1, z1] = volume.max.map(|value| value + 1);
    for x in x0..=x1 {
        for z in z0..=z1 {
            let edge = x == x0 || x == x1 || z == z0 || z == z1;
            let wet = |y: i32| {
                state
                    .world
                    .physics_block_at(x, y, z)
                    .is_some_and(|block| liquid::has_fluid(state, block))
            };
            if !edge {
                if wet(y0) || wet(y1) {
                    return true;
                }
                continue;
            }
            if (y0..=y1).any(wet) {
                return true;
            }
        }
    }
    false
}

/// One site pass's scratch, shared by every candidate it checks: what the
/// pass derived from the world and needs again for its next lot. A pass runs
/// inside one tick for one settlement; the next pass starts a new survey,
/// since the world and the territory move between passes.
#[derive(Default)]
pub(super) struct Survey<'a> {
    /// Planted-tree proofs: a registered root is proved once per pass.
    pub(super) clearance: super::super::forestry::Clearance,
    /// The village walk every lot that cuts a path must keep connected.
    walk: super::relayout::Walk,
    /// The deposit path's capacity, only while checking a pressured storehouse.
    pub(super) storage: Option<storage::Capacity<'a>>,
}

/// `floors` are the footprint columns of this candidate's own authored floor.
pub(super) fn check(
    state: &TickState,
    record: &SettlementRecord,
    origin: [i32; 3],
    size: [u8; 3],
    expedition: Option<&FoundingExpedition>,
    survey: &mut Survey,
    floors: Option<&BTreeSet<[i32; 2]>>,
) -> Result<Cost, Reason> {
    if founding::site_reserved(state, record.id, origin, size) {
        return Err(Reason::OutpostClaim);
    }
    // Owner D27.4: inside this settlement's own territory, decided per modified
    // cell with `record.contains`, its planner reads no player history or
    // attribution. A founding site lies outside the parent's territory and
    // keeps every protection.
    let Survey {
        clearance, walk, ..
    } = survey;
    clearance.within(record);
    let max: [i32; 3] = std::array::from_fn(|axis| origin[axis] + i32::from(size[axis]) - 1);
    let lot = ScriptWorldBounds::new(origin, max).expect("blueprint bounds");
    // Support discovery is centered on the candidate floor, and the whole
    // terrain window is bounded: fill at most SITE_BAND below that floor and
    // cut only inside the authored level. Anything else is a named refusal.
    let floor = origin[1];
    let mut lowest = floor;
    let mut cost = Cost::default();
    let mut liquids = liquid::Liquids::default();
    // Per-column discovered supports: the bowl also proves the interior below a
    // higher neighbour's support is solid ground (no cave pocket to descend in).
    let mut columns: Vec<([i32; 2], i32)> = Vec::new();
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
    // Village paths and farmland the lot covers: a path that carries the road
    // network is re-laid around the lot, so those cells are not refusals.
    let relayout = super::relayout::plan(state, record, lot, walk)?;
    for x in origin[0]..=max[0] {
        for z in origin[2]..=max[2] {
            if distance_sq([x, max[1], z], record.anchor) > i64::from(record.radius).pow(2)
                && !expedition.is_some_and(|site| site.contains([x, max[1], z]))
            {
                return Err(Reason::Territory);
            }
            let height = liquid::ground_y_for_lot(state, floor, x, z, Some(lot), clearance)
                .or_else(|| relayout.as_ref().and_then(|plan| plan.height(x, z)))
                .ok_or_else(|| {
                    (floor - SITE_BAND + 1..=floor + SITE_BAND + 1)
                        .find(|&y| {
                            state.world.physics_block_at(x, y, z).is_some_and(|block| {
                                super::super::forestry::tree_or_sapling(state, block)
                                    && !clearance.clearable(state, [x, y, z], Some(lot))
                            })
                        })
                        .map_or(Reason::SurfaceUnavailable, |y| {
                            tree_refusal(state, [x, y, z], record.contains([x, y, z]))
                        })
                })?;
            lowest = lowest.min(height);
            columns.push(([x, z], height));
            let ground = state
                .world
                .physics_block_at(x, height, z)
                .expect("loaded surface");
            // Only a retired path is the re-layout's own work at the support: a
            // crop there is still a paid obstacle for a generic site.
            let relayout_support = relayout
                .as_ref()
                .is_some_and(|plan| plan.retires([x, height, z]));
            if !floor_replaceable(state, ground) && !relayout_support {
                return Err(Reason::ProtectedFloor);
            }
            let authored_floor =
                floors.is_some_and(|floors| floors.contains(&[x - origin[0], z - origin[2]]));
            // The column's own floor cell is rewritten either way, so every
            // veto applies to it: the authored exemption waives only its fill
            // price, never permission, fluid, container or player checks.
            let floor_block = state
                .world
                .physics_block_at(x, floor, z)
                .ok_or(Reason::Unloaded)?;
            let floor_property = record.contains([x, floor, z]);
            // A fluid on the authored floor row is above the fill work: it is a
            // named liquid refusal, not a generic protected floor. On an
            // unfloored row the fluid block *is* the last fill cell and the plan
            // writes one paid block into it, so it is admitted here and counted
            // by `scan_column`; a waterlogged solid there is a supply no fill can
            // cut off and refuses by name.
            let floor_fluid = liquid::fluid_block(state, floor_block);
            let floor_contained = liquid::contained(state, floor_block).is_some();
            if floor_contained && (authored_floor || !floor_fluid) {
                return Err(Reason::LiquidInWork);
            }
            // Only a candidate whose authored plan is known may treat its own
            // unfloored floor row as the last fill cell. A generic footprint
            // test (`floors: None`, e.g. founding) keeps refusing any fluid
            // there, as does an authored floor row.
            let floor_fluid_fill = floor_fluid && floors.is_some() && !authored_floor;
            if state.block_entities.contains_key(&(x, floor, z)) {
                return Err(Reason::ProtectedObstruction);
            }
            let relayout_floor = relayout
                .as_ref()
                .is_some_and(|plan| plan.retires([x, floor, z]));
            if !floor_replaceable(state, floor_block) && !floor_fluid_fill && !relayout_floor {
                return Err(Reason::ProtectedFloor);
            }
            if floor_block != state.registry.states.air
                && !floor_property
                && state.block_log.player_touched([x, floor, z])
            {
                return Err(Reason::ProtectedObstruction);
            }
            // A single fill column deeper than this is a retaining wall, not a
            // foundation, unless every one of its cells is inside the territory
            // (the sphere is convex, so its lowest fill cell and the floor decide):
            // there the builder fills it (owner 2026-11-17). The support was found
            // within `SITE_BAND` of the floor, which bounds the depth to what a
            // builder at the lot's perimeter reaches.
            if floor - height > MAX_FILL_DEPTH
                && !(record.contains([x, height + 1, z]) && record.contains([x, floor, z]))
            {
                return Err(Reason::HeightVariance);
            }
            // Fill: real blocks between the support and the floor. A fluid cell
            // there is a fill target (its supply is proved closed by
            // `liquid::finish`); an authored floor row may not hold fluid.
            liquid::scan_column(
                state,
                record,
                &mut liquids,
                &mut cost,
                [x, z],
                height,
                floor,
                authored_floor,
            )?;
            // Authored volume: the only place this slice may cut terrain.
            for y in floor + 1..=max[1] {
                let block = state
                    .world
                    .physics_block_at(x, y, z)
                    .ok_or(Reason::Unloaded)?;
                if liquid::has_fluid(state, block) {
                    return Err(Reason::LiquidInWork);
                }
                let property = record.contains([x, y, z]);
                let tree = super::super::forestry::tree_or_sapling(state, block);
                let tree_clearable = tree && clearance.clearable(state, [x, y, z], Some(lot));
                // A lot crop and a retired village path cell are the
                // re-layout's own removal work, never a refusal.
                // A retired village path is always the re-layout's work. A lot
                // crop is its work only when the candidate's authored plan is
                // known (`floors: Some`, the real admission path): a generic
                // footprint probe still refuses a paid crop.
                let relayout_work = relayout
                    .as_ref()
                    .is_some_and(|plan| plan.retires([x, y, z]) || plan.clears([x, y, z]));
                if relayout_work {
                    continue;
                }
                if state.block_entities.contains_key(&(x, y, z))
                    || (tree && !tree_clearable)
                    || (!clearable(state, block) && !excavatable(state, block) && !tree_clearable)
                {
                    return Err(tree_refusal(state, [x, y, z], property));
                }
                if block != state.registry.states.air && !tree {
                    if !property && state.block_log.player_touched([x, y, z]) {
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
                        return Err(tree_refusal(state, [x, y, z], record.contains([x, y, z])));
                    }
                    continue;
                }
                if block != state.registry.states.air && !clearable(state, block) {
                    return Err(Reason::CutBeyondEnvelope);
                }
            }
        }
    }
    // A certified natural zone is terrain, not a building: it keeps only its own
    // columns and is tested against the whole preparation volume — the authored
    // footprint and the fill down to the lowest discovered ground. A zone below
    // the floor that the fill would cross still refuses (owner D10, review 1.1).
    let prepared_min = lowest;
    if record.buildings.values().any(|building| {
        building.natural_zone
            && [0, 2].into_iter().all(|axis| {
                origin[axis] <= building.bounds.max[axis] && building.bounds.min[axis] <= max[axis]
            })
            && building.bounds.min[1] <= max[1]
            && prepared_min <= building.bounds.max[1]
    }) {
        return Err(Reason::BuildingBuffer);
    }
    let volume = ScriptWorldBounds::new([origin[0], lowest, origin[2]], max).expect("work volume");
    // A fluid in the one-cell shell refuses before the bowl name: its supply is
    // outside this footprint, which is a different failure from an open ring.
    if shell_fluid(state, volume) {
        return Err(Reason::LavaShell);
    }
    // Fluid inside the work volume is a fill target only while its supply ends
    // inside this footprint (closed bowl) and it is a single fluid kind.
    liquid::finish(state, &liquids, origin, max, lowest, &columns)?;
    cost.liquid = liquids.total();
    cost.water = liquids.water;
    cost.lava = liquids.lava;
    // The re-laid village detour is executor work too: its own reservation
    // refuses the lot exactly like the prepared volume.
    if executor::area_reserved(state, volume)
        || relayout
            .as_ref()
            .and_then(|plan| plan.area())
            .is_some_and(|area| executor::area_reserved(state, area))
    {
        return Err(Reason::ExecutorReservation);
    }
    if let Some(capacity) = &mut survey.storage {
        storage::candidate_fits(
            state,
            origin,
            size,
            &columns,
            cost.cut,
            relayout.as_ref(),
            capacity,
        )?;
    }
    Ok(cost)
}

#[cfg(test)]
#[path = "construction_site_refusal_tests.rs"]
mod tests;
