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
    /// The lot's own trees have no walking route (`forestry::cells`: a tree on
    /// the lot and `construction_access` returns none), so the plan cannot
    /// prepare it. Its own reason, not `NoPathDetour`: that one is the owner's
    /// relayout bridge (OWNER_DECISIONS.md:199-202) and the histogram has to
    /// name what actually refused (HANDOFF-51, review round 2 R4).
    LotTreesWithoutRoute,
}
const REASONS: [Reason; 27] = [
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
    Reason::LotTreesWithoutRoute,
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
#[derive(Debug, Default)]
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
    /// The settlement's own admission pass ended with a kind's walk unfinished
    /// and already holding a legal candidate: only such a settlement is stepped
    /// off the four-tick admission clock, and only its site scan (review round 3
    /// C3/C4). Runtime only, like the sweeps it describes. Cleared by a pass that
    /// finds no candidate, by `Empty`, by a restart after a failed re-proof, and
    /// by a step whose scan restarted a walk on a street change (the next
    /// admission pass arms it again, review round 2 N3).
    pub(super) stepping: bool,
    /// How often each kind's walk was dropped, by a street change or by its
    /// stored best losing its cost. Log-only, and kept OUTSIDE the sweep: a
    /// counter inside `Sweep` made `retain` create an entry (so the next walk
    /// started at frontage 0 instead of the cursor, and the persisted
    /// `construction_cursor` moved) and made completion re-insert a sweep
    /// instead of removing it (HANDOFF-51, review round 2 R3).
    restarts: BTreeMap<String, u32>,
    /// The funded verdict per kind, taken by the settlement's own admission
    /// pass: a step off the admission clock reuses it instead of re-proving
    /// every kind's recipe bill on every stepped tick (the funding work
    /// budget's cost, CIV-ADOPTED-CLOSURE-ROOT-TICK-COST). A step never
    /// materializes its winner, so the verdict is all it needs; the next
    /// admission pass re-proves and overwrites the entry.
    funding: BTreeMap<String, bool>,
}
impl Sweeps {
    /// Drop sweeps of a topology that no longer exists, counting the restart so
    /// the refusal log can say whether a village churns its streets under a
    /// walk. The count never changes what a walk returns: a stale sweep is gone
    /// exactly as before, and the next pass starts a fresh one.
    fn retain(&mut self, blueprint: &str, frontages: u32, topology: u64) {
        let dropped = self
            .live
            .iter()
            .any(|(id, sweep)| sweep_stale(id, sweep, blueprint, frontages, topology));
        self.live
            .retain(|id, sweep| !sweep_stale(id, sweep, blueprint, frontages, topology));
        if dropped {
            *self.restarts.entry(blueprint.to_owned()).or_default() += 1;
        }
    }

    /// This kind's walk exists and still has frontages to examine.
    pub(super) fn unfinished(&self, blueprint: &str) -> bool {
        self.live
            .get(blueprint)
            .is_some_and(|sweep| sweep.next < sweep.frontages)
    }

    /// This kind was funded when the settlement's own admission pass proved
    /// its bill: a step's funding decision, without a proof (see `funding`).
    fn funded(&self, blueprint: &str) -> bool {
        self.funding.get(blueprint).copied().unwrap_or(false)
    }

    /// How often walks were restarted in total (log-only counters): a scan that
    /// leaves this higher dropped a walk (a street change) or lost a stored
    /// best, so the next step must not keep stepping it (review round 2 N3).
    pub(super) fn restarts_total(&self) -> u32 {
        self.restarts.values().copied().sum()
    }

    /// Some unfinished sweep of these kinds already holds a legal candidate.
    /// A retained sweep of a kind this pass did not advance (its funding was
    /// withdrawn) must not hold the slot for other work.
    pub(super) fn holding<'a>(&self, kinds: impl IntoIterator<Item = &'a str>) -> bool {
        kinds.into_iter().any(|kind| {
            self.live
                .get(kind)
                .is_some_and(|sweep| sweep.best.is_some())
        })
    }

    /// Candidates this blueprint's live sweep has examined (test observer).
    #[cfg(test)]
    pub(super) fn examined(&self, blueprint: &str) -> Option<u32> {
        self.live.get(blueprint).map(|sweep| sweep.next)
    }

    /// One line per live sweep for the planner's refusal log: which kind, how far
    /// its walk got, how many frontages it must cover, and whether it already
    /// holds a candidate. A settlement that never admits has to say whether its
    /// sweeps finish, restart on a street change, or finish without a lot.
    pub(super) fn summary(&self) -> Vec<(String, u32, u32, bool, u32)> {
        self.live
            .iter()
            .map(|(id, sweep)| {
                (
                    id.clone(),
                    sweep.next,
                    sweep.frontages,
                    sweep.best.is_some(),
                    self.restarts.get(id).copied().unwrap_or(0),
                )
            })
            .collect()
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
) -> Option<FundedSite> {
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
    Some(FundedSite {
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
/// `park` keeps a proven winner's finished sweep instead of forgetting it: a
/// caller that may not admit this tick (an off-cadence step) leaves the lot in
/// the hold, and the settlement's next admission pass re-proves it rather than
/// walking the street again.
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
    park: bool,
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
        ..Sweep::default()
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
            // `check` cannot model the tree plan, and `forestry::cells` refuses a
            // lot whose own trees have no walking route or whose tree work it
            // cannot prove: agree here, with the pass's own network and the very
            // predicate `cells` runs, so the lot never wins the cheapest ranking
            // and cannot be re-found every walk (HANDOFF-51: seed-24 House lot
            // [246,69,-176]).
            Ok(_)
                if super::super::forestry::lot_trees_without_route(
                    state,
                    network,
                    &super::Project {
                        settlement: record.id,
                        blueprint: blueprint.id.clone(),
                        level: 1,
                        origin: lot.origin,
                        rotation,
                        building: None,
                        structure: String::new(),
                        ordered: false,
                        builder: None,
                        order_seq: None,
                    },
                    size,
                    &mut survey.clearance,
                ) =>
            {
                report.reject(Reason::LotTreesWithoutRoute, lot.origin);
            }
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
    let finished = sweeps.live.remove(&blueprint.id);
    let Some((origin, rotation, stored)) = finished.as_ref().and_then(|sweep| sweep.best) else {
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
        Ok(cost) if cost == stored => {
            if park {
                // The caller may not admit this tick: keep the proven winner so
                // its next admission pass re-proves it, not the whole street.
                if let Some(sweep) = finished {
                    sweeps.live.insert(blueprint.id.clone(), sweep);
                }
            }
            Advance::Cheapest(origin, rotation)
        }
        // The world moved under the stored candidate: it is gone (the sweep was
        // removed above), so this restart holds nothing. The count is log-only.
        Ok(_) | Err(_) => {
            *sweeps.restarts.entry(blueprint.id.clone()).or_default() += 1;
            Advance::Incomplete(0)
        }
    }
}

/// The funded site search and its off-cadence stepping, moved here from
/// `construction.rs` beside the sweeps they drive (code-health ratchet,
/// 2026-11-26): one bounded pass of the funded kinds (`site_project`), the
/// shared street walk over the runtime sweeps (`scan_site_kinds`), the
/// priority-House-then-funded scan an admission turn or a step runs
/// (`scan_sites`), and the off-the-clock step itself (`step_sites`).
pub(super) struct FundedSite {
    pub(super) project: Project,
    pub(super) bounds: ScriptWorldBounds,
    pub(super) cells: Vec<executor::PlannedCell>,
}

/// One bounded pass of the funded site search.
pub(super) struct SitePass {
    pub(super) site: Option<FundedSite>,
    pub(super) cursor: u32,
    pub(super) report: Report,
    /// At least one kind's whole sweep still has candidates to examine.
    pub(super) running: bool,
    /// A kind this pass advanced ran unfinished and already holds a legal
    /// candidate: its cheapest lot is worth the slot. Another kind's sweep,
    /// even a now-unfunded one, never blocks other work.
    pub(super) waiting: bool,
    /// This pass really ran the catalog House's sweep (`advance` was called for
    /// a House kind, whatever it returned). The same catalog House can be
    /// fetched by the priority scan (physical beds) and then by the ordinary
    /// funded scan (certificate levels); the later visit owns its final state.
    pub(super) house_advanced: bool,
    /// A walk finished in a pass that may not admit (an off-cadence step): its
    /// proven winner is parked in the sweep and waits for the settlement's next
    /// `PLAN_CADENCE` admission pass (up to 20 x settlements ticks), so the
    /// settlement stops stepping until that pass reports again.
    pub(super) complete: bool,
}

/// Search the funded kinds in priority order over one shared street walk, for an
/// admission: the winner is materialized and returned.
pub(super) fn site_project<'a>(
    state: &TickState,
    record: &SettlementRecord,
    kinds: impl ExactSizeIterator<Item = &'a blueprints::Blueprint>,
    sweeps: &mut Sweeps,
    start: u32,
    attempts: u32,
    images: &[super::super::inventory::Image],
) -> SitePass {
    scan_site_kinds(
        state,
        record,
        kinds,
        sweeps,
        start,
        SiteSlice {
            lots: attempts,
            admit: true,
        },
        images,
    )
}

/// One scan's slice: the lots it may examine, and whether a completed walk's
/// winner may be materialized (an off-cadence step parks it instead).
#[derive(Clone, Copy)]
struct SiteSlice {
    lots: u32,
    admit: bool,
}

/// Search the funded kinds in priority order over one shared street walk.
/// A kind may admit only after its whole cursor sweep has been examined, and
/// then only its cheapest legal lot; an unfinished sweep admits nothing. Each
/// kind advances its own resumable sweep every pass, so a completed empty kind
/// cannot starve a later one, and the first kind in priority order still wins
/// when several are ready. Admission is decided on the authored bill only, so
/// foundation material is a project shortfall, not a site refusal. Only the
/// winner is materialized, and only when `slice.admit`: a step off the
/// admission clock parks a completed walk's winner in its sweep instead.
fn scan_site_kinds<'a>(
    state: &TickState,
    record: &SettlementRecord,
    kinds: impl ExactSizeIterator<Item = &'a blueprints::Blueprint>,
    sweeps: &mut Sweeps,
    start: u32,
    slice: SiteSlice,
    images: &[super::super::inventory::Image],
) -> SitePass {
    let mut network = layout::Network::for_construction(state, record);
    let count = u32::try_from(network.frontage().len()).unwrap_or(u32::MAX);
    let mut report = Report {
        frontages: count,
        ..Default::default()
    };
    if count == 0 {
        report.reject(Reason::NoFrontage, record.anchor);
        return SitePass {
            site: None,
            cursor: 0,
            report,
            running: false,
            waiting: false,
            house_advanced: false,
            complete: false,
        };
    }
    let mut cursor = start % count;
    let kinds_count = kinds.len().max(1) as u32;
    // Every funded kind gets an equal share of the pass's lots, and the
    // remainder (at most `kinds - 1` lots) now goes to the first kinds instead
    // of being dropped. This is the only thing the hand-out changes: a kind was
    // never starved by the kinds before it (each one already had its own share;
    // a sweep standing still in the gate log was simply not funded that pass).
    let share = slice.lots / kinds_count;
    let mut leftover = slice.lots % kinds_count;
    let mut running = false;
    let mut advanced: Vec<&'a str> = Vec::with_capacity(kinds.len());
    let mut house_advanced = false;
    let mut survey = Survey::default();
    for blueprint in kinds {
        let Some(level) = blueprint.level(1) else {
            continue;
        };
        let floors = std::array::from_fn(|turns| {
            floor_columns(blueprint, u8::try_from(turns).expect("quarter turns"))
        });
        // An admission pass proves the kind's bill and records the verdict on
        // the sweeps; a step off the admission clock reuses that verdict, so a
        // stepped tick never re-derives funding (its recipe scans were the work
        // budget's cost, CIV-ADOPTED-CLOSURE-ROOT-TICK-COST). A step never
        // materializes its winner, so the verdict is all it needs; the next
        // admission pass re-proves and overwrites the entry.
        let species = if slice.admit {
            let proven =
                storage::funded_variant(state, record, blueprint, &level.materials, images);
            sweeps
                .funding
                .insert(blueprint.id.clone(), proven.is_some());
            proven
        } else {
            None
        };
        if slice.admit && species.is_none() {
            report.reject(Reason::MaterialBill, record.anchor);
            continue;
        }
        if !slice.admit && !sweeps.funded(blueprint.id.as_str()) {
            report.reject(Reason::MaterialBill, record.anchor);
            continue;
        }
        survey.storage = storage::Capacity::for_blueprint(state, record, blueprint, images);
        if blueprint.kind == BuildingKind::Storage {
            tracing::debug!(
                game_tick = state.tick,
                settlement = record.id.0,
                pressured = survey.storage.is_some(),
                attempts = share,
                "settlement_storehouse_search"
            );
        }
        let mut budget = share;
        if leftover > 0 {
            budget += 1;
            leftover -= 1;
        }
        // Any outcome below means this pass really walked the kind's sweep; the
        // flag is about the visit, not about what the walk found.
        if blueprint.kind == BuildingKind::House {
            house_advanced = true;
        }
        advanced.push(blueprint.id.as_str());
        match advance(
            state,
            record,
            &mut network,
            blueprint,
            &floors,
            &mut survey,
            sweeps,
            cursor,
            &mut budget,
            &mut report,
            !slice.admit,
        ) {
            Advance::Incomplete(next) => {
                cursor = next;
                running = true;
            }
            Advance::Empty => {
                cursor = 0;
            }
            Advance::Cheapest(origin, rotation) => {
                // A step that may not admit leaves nothing to materialize: the
                // completed walk is parked in its sweep (see `advance`) and
                // waits for the settlement's next `PLAN_CADENCE` admission pass.
                let site = slice
                    .admit
                    .then(|| {
                        let (species, color) = species.expect("an admission proved its species");
                        materialize(
                            state,
                            record,
                            Winner {
                                blueprint,
                                origin,
                                rotation,
                                species,
                                color,
                            },
                            &floors[usize::from(rotation) % 4],
                            &mut survey,
                            images,
                            &mut report,
                        )
                    })
                    .flatten();
                // A refused materialization is not an admission: an earlier
                // kind's unfinished sweep that holds a lot keeps the slot.
                return SitePass {
                    site,
                    cursor: 0,
                    report,
                    running,
                    waiting: sweeps.holding(advanced.iter().copied()),
                    house_advanced,
                    complete: true,
                };
            }
        }
    }
    SitePass {
        site: None,
        cursor,
        report,
        running,
        // Only the kinds this pass advanced own the hold.
        waiting: sweeps.holding(advanced.iter().copied()),
        house_advanced,
        complete: false,
    }
}

/// One bounded site scan of a settlement: the priority House's walk first, then
/// the funded kinds over the same runtime sweeps. Its two passes are the ones
/// the settlement's admission turns run; a step off the admission clock runs at
/// most one of them with one slice and may not admit.
pub(super) struct SiteScan {
    pub(super) house: Option<FundedSite>,
    pub(super) cursor: u32,
    pub(super) funded: Option<SitePass>,
    /// A kind's whole sweep still has candidates to examine.
    pub(super) running: bool,
    /// A kind this scan advanced is unfinished and already holds a legal lot.
    pub(super) waiting: bool,
    /// A walk finished in a step that may not admit; its lot is parked in the
    /// sweep and waits for the settlement's next `PLAN_CADENCE` admission pass.
    pub(super) complete: bool,
}

/// The funded autonomous kinds this scan may consider, in priority order. A
/// kind whose level-one bill no stock can fund is skipped, so a funded kind
/// later in the order is never starved by an unfunded one before it.
pub(super) fn funded_kinds(
    state: &TickState,
    record: &SettlementRecord,
    id: SettlementId,
    images: &[super::super::inventory::Image],
) -> (
    [Option<&'static blueprints::Blueprint>; AUTONOMOUS_KINDS],
    usize,
) {
    let mut funded = [None; AUTONOMOUS_KINDS];
    let mut count = 0;
    let catalog = catalog(state, record);
    for kind in candidate_kinds(state, record, images) {
        let Some(blueprint) = catalog.clone().find(|blueprint| blueprint.kind == kind) else {
            continue;
        };
        if blueprint
            .level(1)
            .is_some_and(|level| funded_variant(state, id, &level.materials, images).is_some())
        {
            funded[count] = Some(blueprint);
            count += 1;
        }
    }
    (funded, count)
}

/// The kinds an off-cadence step may advance: the funded verdict its own
/// admission pass recorded, re-read with no recipe proof at all (a step never
/// adds funding work; the next admission pass decides again).
fn funded_memo(
    state: &TickState,
    record: &SettlementRecord,
    images: &[super::super::inventory::Image],
    sweeps: &Sweeps,
) -> (
    [Option<&'static blueprints::Blueprint>; AUTONOMOUS_KINDS],
    usize,
) {
    let mut funded = [None; AUTONOMOUS_KINDS];
    let mut count = 0;
    let catalog = catalog(state, record);
    for kind in candidate_kinds(state, record, images) {
        let Some(blueprint) = catalog.clone().find(|blueprint| blueprint.kind == kind) else {
            continue;
        };
        if sweeps.funded(blueprint.id.as_str()) {
            funded[count] = Some(blueprint);
            count += 1;
        }
    }
    (funded, count)
}

/// The settlement's site scan. `step` is an off-cadence step's already-taken
/// single slice: the whole scan spends it, only one of the two passes runs (the
/// priority House's walk while it is unfinished, else the funded kinds), and
/// nothing is materialized. `None` is the admission pass, which takes one slice
/// per pass (the D42 planner budget) and materializes its winner.
pub(super) fn scan_sites(
    state: &TickState,
    id: SettlementId,
    record: &SettlementRecord,
    images: &[super::super::inventory::Image],
    sweeps: &mut Sweeps,
    budget: &mut site_turn::SiteBudget,
    step: Option<u32>,
) -> SiteScan {
    let admit = step.is_none();
    // In a step the priority House is scanned only while its own walk is
    // unfinished; a finished (parked) one waits for the admission pass and
    // leaves the slice to the funded kinds.
    let house = match priority_house(state, record, images) {
        Some(house) if admit || sweeps.unfinished(&house.id) => Some(scan_site_kinds(
            state,
            record,
            std::iter::once(house),
            sweeps,
            record.construction_cursor,
            SiteSlice {
                lots: step.unwrap_or_else(|| budget.take()),
                admit,
            },
            images,
        )),
        _ => None,
    };
    let house_ran = house.is_some();
    let house = house.unwrap_or_else(|| SitePass {
        site: None,
        cursor: record.construction_cursor,
        report: Report::default(),
        running: false,
        waiting: false,
        house_advanced: false,
        complete: false,
    });
    let mut running = house.running;
    let mut waiting = house.waiting;
    let mut complete = house.complete;
    let mut funded = None;
    // An admission pass scans the funded kinds whenever the priority House has
    // no site; a step scans them only when it did not already spend its slice.
    if house.site.is_none() && (admit || !house_ran) {
        // An admission pass proves funding; a step reads the verdict that pass
        // recorded (see `scan_site_kinds`).
        let (kinds, count) = if admit {
            funded_kinds(state, record, id, images)
        } else {
            funded_memo(state, record, images, sweeps)
        };
        if count != 0 {
            let pass = scan_site_kinds(
                state,
                record,
                kinds
                    .into_iter()
                    .take(count)
                    .map(|blueprint| blueprint.expect("funded prefix")),
                sweeps,
                house.cursor,
                SiteSlice {
                    lots: step.unwrap_or_else(|| budget.take()),
                    admit,
                },
                images,
            );
            running |= pass.running;
            // The priority scan (physical beds) and the ordinary scan
            // (certificate levels) can fetch the same catalog House in one
            // pass. When the ordinary pass really walked that sweep again,
            // its final state owns the House's hold: a completed sweep whose
            // recheck moved the world carries no hold, and the stale
            // priority hold must not survive it.
            if pass.house_advanced {
                waiting = pass.waiting;
            } else {
                waiting |= pass.waiting;
            }
            complete |= pass.complete;
            // Why a funded kind still has no site is the question the gates
            // could not answer: autonomous refusals were never logged, so
            // "no House in the run" had no cause on disk (ReviewD123Civ13).
            if pass.site.is_none() {
                // Log-only diagnosis (HANDOFF-49): a settlement that never admits
                // has to say whether its sweeps finish, restart on a street change,
                // or finish without a lot. `sweeps` is
                // (kind, next, frontages, holds a lot, restarts).
                tracing::debug!(
                    target: "settlements::construction",
                    settlement = id.0,
                    cursor = pass.cursor,
                    frontages = pass.report.frontages,
                    examined = pass.report.examined,
                    sweeps = ?sweeps.summary(),
                    refusals = ?pass.report,
                    "funded kinds found no legal site"
                );
            }
            funded = Some(pass);
        }
    }
    SiteScan {
        house: house.site,
        cursor: house.cursor,
        funded,
        running,
        waiting,
        complete,
    }
}

/// Off the four-tick construction clock, step the settlement whose last
/// admission pass left an unfinished walk holding a legal lot: only its site
/// scan, ONE slice for the whole tick, and never an admission, a construction
/// cell or a duty. The slice taken is one of the tick's two, so the order turn
/// and the replanting proofs keep one; the lowest settlement id with a holding
/// walk steps first, and the next follows once that walk finishes or a street
/// change restarts it (a restart clears the flag until the next admission pass).
/// `drain_carry_duties`, the gatherer staffing and the builder assignment stay
/// on `PLAN_CADENCE`; `villager::building::tick` stays on the four-tick clock,
/// so construction's pace is unchanged (review round 3 C1/C2/C5, round 2 N3).
pub(super) fn step_sites(state: &mut TickState, budget: &mut site_turn::SiteBudget) {
    let stepping: Vec<SettlementId> = state
        .settlements
        .site_sweep
        .iter()
        .filter(|(_, sweeps)| sweeps.stepping)
        .map(|(id, _)| *id)
        .collect();
    for id in stepping {
        // One slice per tick, and a whole slice always stays behind.
        if budget.slices() < 2 {
            break;
        }
        let active = state.settlements.get(id).is_some_and(|record| {
            state.world.is_active(
                record.anchor[0].div_euclid(16),
                record.anchor[2].div_euclid(16),
            )
        });
        if !active {
            continue;
        }
        let slice = budget.take();
        let images = executor::source_images(state, id);
        let mut sweeps = state.settlements.site_sweep.remove(&id).unwrap_or_default();
        let restarts = sweeps.restarts_total();
        let scan = {
            let record = state.settlements.get(id).expect("stepped settlement");
            scan_sites(state, id, record, &images, &mut sweeps, budget, Some(slice))
        };
        // A finished walk waits for the settlement's next admission pass. An
        // unfinished walk that still holds a lot keeps stepping; one that holds
        // nothing clears the flag, so a village whose walk finds no legal lot
        // returns to the normal cadence instead of paying a scan every tick
        // (C4). A step whose scan restarted a walk (a street change dropped it,
        // or its stored best lost its cost) clears it too: the new walk starts
        // holding nothing and the next admission pass re-arms it, so continuous
        // churn cannot become steady-state stepping, and the lowest settlement
        // id cannot keep the tick to itself (review round 2 N3).
        sweeps.stepping = scan.waiting && !scan.complete && sweeps.restarts_total() == restarts;
        state.settlements.site_sweep.insert(id, sweeps);
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

/// The restart counter is log-only bookkeeping: a street change that drops a
/// half-walked sweep must be visible in the refusal log, and must not change
/// what the walk returns (the sweep is gone exactly as before, and `Empty`
/// stays reachable for an owner order).
#[cfg(test)]
mod sweep_restart_tests {
    use super::*;

    fn sweep(frontages: u32, topology: u64, next: u32, best: bool) -> Sweep {
        Sweep {
            frontages,
            topology,
            start: 0,
            next,
            best: best.then_some((
                [3, 201, 4],
                0,
                Cost {
                    cut: 1,
                    ..Cost::default()
                },
            )),
        }
    }

    #[test]
    fn a_street_change_drops_the_walk_and_counts_the_restart() {
        let mut sweeps = Sweeps::default();
        sweeps.live.insert("house".into(), sweep(120, 7, 55, true));
        // The streets changed: same kind, new frontage list.
        sweeps.retain("house", 130, 8);
        assert!(
            !sweeps.live.contains_key("house"),
            "the stale walk is dropped, and the counter does not keep it alive"
        );
        assert_eq!(sweeps.restarts.get("house").copied(), Some(1));
    }

    #[test]
    fn a_stable_village_keeps_its_sweep_and_counts_no_restart() {
        let mut sweeps = Sweeps::default();
        sweeps.live.insert("house".into(), sweep(120, 7, 55, true));
        sweeps.retain("house", 120, 7);
        let kept = sweeps.live.get("house").expect("the sweep survives");
        assert_eq!(kept.next, 55, "the walk is not restarted");
        assert!(kept.best.is_some());
        assert!(!sweeps.restarts.contains_key("house"));
    }

    #[test]
    fn the_refusal_log_names_the_restart_count() {
        let mut sweeps = Sweeps::default();
        sweeps.live.insert("house".into(), sweep(120, 7, 55, true));
        sweeps.retain("house", 130, 8);
        // The next pass starts a fresh walk, as before the counter existed.
        sweeps.live.insert("house".into(), sweep(130, 8, 0, false));
        let summary = sweeps.summary();
        assert_eq!(summary.len(), 1);
        let (kind, next, frontages, holds, restarts) = &summary[0];
        assert_eq!((kind.as_str(), *next, *frontages), ("house", 0, 130));
        assert!(!*holds, "the fresh walk holds nothing yet");
        assert_eq!(*restarts, 1);
    }
}

#[cfg(test)]
#[path = "construction_site_refusal_tests.rs"]
mod tests;
