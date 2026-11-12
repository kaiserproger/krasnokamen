//! Native contact damage, ignition and extinguishing through merge admission.
//!
//! Multi-cell scans borrow a single resident column when overlays are empty;
//! seam and overlay scans retain the exact overlay-first world reader.

use crate::boundary::EntityId;
use crate::intent::{Footprint, Intent, IntentKind};
use crate::registry::{
    HAZARD_BERRY, HAZARD_CACTUS, HAZARD_CAMPFIRE, HAZARD_FIRE, HAZARD_LAVA, HAZARD_MAGMA,
    HAZARD_SOUL_CAMPFIRE, HAZARD_WATER,
};
use crate::survival::DamageKind;
use crate::tick::TickState;
use mc_physics::mth::floor_i32;
use std::sync::{Arc, LazyLock};

// Test-only attribution of the whole contact pass, including the cauldron tail.
// Counts are collected on the serial owner, never on classifier worker threads.
#[cfg(test)]
std::thread_local! {
    static WORK: std::cell::Cell<[usize; 5]> = const { std::cell::Cell::new([0; 5]) };
}

#[cfg(test)]
fn count_work(index: usize, amount: usize) {
    WORK.with(|work| {
        let mut counts = work.get();
        counts[index] += amount;
        work.set(counts);
    });
}

/// Candidates, exact-scan rows, quiet rows, exact samples, cauldron samples.
#[cfg(test)]
pub(crate) fn take_work_counts() -> [usize; 5] {
    WORK.with(|work| work.replace([0; 5]))
}

/// Reused per-pass section-predicate caches for the contact run.
///
/// `hazard` gates body/foot-support samples on any nonzero hazard flag;
/// `suffocation` gates the eye-cell `block_at` on any suffocating state.
/// Both hold one fixed predicate's verdicts for a single read-only pass and
/// are `reset` once per `run`; `TickState` retains their capacity between runs.
///
/// `negative` memoizes proven raw geometric absence per dense entity slot as
/// `world::ContactStamp<2>` (`probes` are the foot-support center cell and the
/// exact eye cell from the actual height, which can exceed the clamped body
/// envelope). Slots are fenced by `EntityId` against swap-remove compaction;
/// the `Vec` retains capacity across ticks. Only true raw absence is stored:
/// any suffocating eye, any nonzero body hazard flag, or magma support blocks
/// the memo even when sneaking, frost-walker, or stationary-berry rules would
/// suppress the damage intent. A hit bypasses geometric reads only; retained
/// burn timers still decay below.
///
/// `cauldron` maps every block state to `[kind, level]` (kind 0 for non
/// cauldron states), built once from the registry on the first pass.
#[derive(Default)]
pub(crate) struct Scratch {
    pub(crate) hazard: crate::world::SectionMatchCache,
    pub(crate) suffocation: crate::world::SectionMatchCache,
    pub(crate) negative: Vec<Option<crate::world::ContactStamp<2>>>,
    cauldron: Vec<[u8; 2]>,
}

/// Set the retained timer and publish only the shared burning bit.
pub(crate) fn ignite(state: &mut TickState, entity: EntityId, ticks: u32) {
    let Some(slot) = state.entities.slot_of(entity) else {
        return;
    };
    let (body, row) = state.entities.parts_mut(slot);
    body.burn_ticks = body.burn_ticks.max(ticks);
    if ticks > 0 && row.appearance.shared_flags & 1 == 0 {
        Arc::make_mut(&mut row.appearance).shared_flags |= 1;
        state.appearance_changed.push(entity);
    }
}

/// One drowning damage per tick in water or rain for every blueprint with
/// [`Capabilities::hurt_by_water`](crate::blueprint::Capabilities::hurt_by_water)
/// (vanilla `isSensitiveToWater`): live rows in active chunks, registry then
/// dense-slot order. Runs before the enderman pass, which teleports out.
pub(crate) fn hurt_by_water(state: &mut TickState) {
    let sensitive: Vec<_> = state
        .registry
        .blueprints
        .iter()
        .filter(|(_, blueprint)| blueprint.capabilities.hurt_by_water)
        .flat_map(|(type_id, _)| state.entities.mob_slots(*type_id))
        .collect();
    for slot in sensitive {
        let body = state.entities.body(slot);
        let pos = state.entities.pos(slot);
        if !body.alive
            || !state.world.is_active(
                floor_i32(pos[0]).div_euclid(16),
                floor_i32(pos[2]).div_euclid(16),
            )
        {
            continue;
        }
        if crate::conduit::wet(state, pos)
            || crate::conduit::water(state, [pos[0], pos[1] + body.height * 0.5, pos[2]])
        {
            state.merge_damage.push((
                slot,
                crate::tick::DamagePiece {
                    physical: None,
                    custom_armor: false,
                    amount: 1.0,
                    kind: DamageKind::Drowning,
                    source: None,
                },
            ));
        }
    }
}

fn damage(
    state: &mut TickState,
    entity: EntityId,
    kind: DamageKind,
    amount: f32,
    intents: &mut Vec<Intent>,
) {
    let seq = state.next_seq(entity);
    intents.push(Intent {
        actor: entity,
        seq,
        kind: IntentKind::DamageEntity {
            target: entity,
            amount,
            kind,
            source: None,
        },
        footprint: Footprint {
            entity_reads: vec![entity],
            entity_writes: vec![entity],
            ..Default::default()
        },
    });
}

// The contract table is generated and immutable: resolve living once.
static LIVING_BY_TYPE: LazyLock<[bool; mc_data::entity_types::ENTITY_TYPE_COUNT]> =
    LazyLock::new(|| {
        let mut living = [false; mc_data::entity_types::ENTITY_TYPE_COUNT];
        for (index, slot) in living.iter_mut().enumerate() {
            *slot = mc_data::entity_types::entity_type_contract_26_1_2_by_protocol_id(index as u32)
                .is_some_and(|contract| {
                    contract.behavior.archetype == mc_data::entity_types::EntityArchetype::Living
                });
        }
        living
    });

type ContactCells = ((i32, i32, i32), (i32, i32, i32), [[i32; 3]; 2]);

#[inline]
fn contact_cells(pos: [f64; 3], shape: [f64; 2], foot_xz: [i32; 2]) -> ContactCells {
    let width = shape[0].min(2.0);
    let height = shape[1].min(6.0);
    let foot_y = floor_i32(pos[1]);
    (
        (floor_i32(pos[0] - width), foot_y, floor_i32(pos[2] - width)),
        (
            floor_i32(pos[0] + width),
            floor_i32(pos[1] + height - 1.0e-6),
            floor_i32(pos[2] + width),
        ),
        [
            [foot_xz[0], foot_y - 1, foot_xz[1]],
            [foot_xz[0], floor_i32(pos[1] + shape[1] * 0.9), foot_xz[1]],
        ],
    )
}

/// Geometry absence never suspends this retained clock or its damage cadence.
#[inline]
fn retained_burn(
    state: &mut TickState,
    slot: crate::entities::EntitySlot,
    water: bool,
    burning_contact: bool,
    intents: &mut Vec<Intent>,
) {
    let body = state.entities.body(slot);
    if body.burn_ticks == 0 && state.entities.row(slot).appearance.shared_flags & 1 == 0 {
        return;
    }
    let entity = body.id;
    let water = water || crate::conduit::rain_exposed(state, state.entities.pos(slot));
    let (body, row) = state.entities.parts_mut(slot);
    if water {
        body.burn_ticks = 0;
    } else {
        body.burn_ticks = body.burn_ticks.saturating_sub(1);
    }
    let burning = body.burn_ticks > 0;
    if (row.appearance.shared_flags & 1 != 0) != burning {
        let flags = &mut Arc::make_mut(&mut row.appearance).shared_flags;
        *flags = (*flags & !1) | u8::from(burning);
        state.appearance_changed.push(entity);
    }
    if burning && state.tick.is_multiple_of(20) && !burning_contact {
        damage(state, entity, DamageKind::OnFire, 1.0, intents);
    }
}

/// A bounded body scan; missing chunks never manufacture hazard or support.
/// Berry damage requires actual horizontal displacement, not packet cadence.
pub(crate) fn run(state: &mut TickState, intents: &mut Vec<Intent>) {
    run_with(state, intents, MIN_PARALLEL_ROWS);
}

/// Rows per rayon job; sized so a job amortizes its small result vector.
const CLASSIFY_JOB_ROWS: usize = 2048;
/// Below this row count the classifier runs inline: a rayon hand-off costs more.
pub(crate) const MIN_PARALLEL_ROWS: usize = 8192;

/// Run a pure per-row classifier over `rows` (jobs of [`CLASSIFY_JOB_ROWS`] on
/// the rayon pool once `rows.len() >= min_parallel`, otherwise inline) and
/// return ascending `(row index, work)` pairs for nonempty results, grouped
/// per job. `classify` may only mutate its own row and must not depend on any
/// other row, so the result is identical for every thread count.
pub(crate) fn classify_rows<S: Send, R: Send>(
    rows: &mut [S],
    min_parallel: usize,
    classify: impl Fn(usize, &mut S) -> Option<R> + Sync,
) -> Vec<Vec<(u32, R)>> {
    let job = |first: usize, rows: &mut [S]| {
        let mut flagged = Vec::new();
        for (offset, row) in rows.iter_mut().enumerate() {
            if let Some(work) = classify(first + offset, row) {
                flagged.push(((first + offset) as u32, work));
            }
        }
        flagged
    };
    if rows.len() < min_parallel {
        return vec![job(0, rows)];
    }
    use rayon::prelude::*;
    rows.par_chunks_mut(CLASSIFY_JOB_ROWS)
        .enumerate()
        .map(|(chunk, rows)| job(chunk * CLASSIFY_JOB_ROWS, rows))
        .collect()
}

/// Reuse a geometric-absence proof, refreshing raw drift only while the same
/// cells and world revision still prove quiet. Return computed coverage for a
/// stale proof so the serial scan never repeats the cell math.
fn quiet_proof(
    world: &crate::world::ChunkWorld,
    hazard: &crate::world::SectionMatchCache,
    stamp: &mut crate::world::ContactStamp<2>,
    pos: [f64; 3],
    shape: [f64; 2],
    foot_xz: [i32; 2],
) -> (bool, Option<ContactCells>) {
    let raw_same = stamp.position == pos && stamp.shape == shape;
    let cells = if raw_same {
        (stamp.min, stamp.max, stamp.probes)
    } else {
        contact_cells(pos, shape, foot_xz)
    };
    let quiet = (raw_same || cells == (stamp.min, stamp.max, stamp.probes))
        && world.contact_unchanged_since(stamp.min, stamp.max, stamp.revision, hazard);
    if quiet && !raw_same {
        stamp.position = pos;
        stamp.shape = shape;
    }
    (quiet, (!quiet).then_some(cells))
}

/// What the pure classifier leaves for the serial owner on one row.
#[derive(Default)]
struct ContactWork {
    /// The exact serial scan ([`scan_row`]) is required.
    scan: bool,
    /// Quiet row with a live burn timer or burning bit: only `retained_burn`.
    burn: bool,
    /// The feet sit in a cauldron's interior and the cell holds a cauldron.
    cauldron: bool,
    /// Attribution for [`take_work_counts`]; rows with no work are never listed otherwise.
    #[cfg(test)]
    quiet: bool,
    #[cfg(test)]
    sampled: bool,
}

/// Geometric verdict of the cold-free classifier for a living row.
enum Verdict {
    Scan,
    Quiet,
}

/// Cauldron kind codes in [`Scratch::cauldron`].
const CAULDRON_WATER: u8 = 1;
const CAULDRON_POWDER_SNOW: u8 = 2;
const CAULDRON_LAVA: u8 = 3;

/// `[kind, level]` for every block state; kind 0 is not a cauldron.
fn cauldron_table(state: &TickState) -> Vec<[u8; 2]> {
    (0..state.registry.hazard_flags.len() as u32)
        .map(|id| {
            let Some(block) = state.registry.blocks.by_id(mc_world::BlockStateId(id)) else {
                return [0, 0];
            };
            let kind = match block.block.id.path() {
                "water_cauldron" => CAULDRON_WATER,
                "powder_snow_cauldron" => CAULDRON_POWDER_SNOW,
                "lava_cauldron" => CAULDRON_LAVA,
                _ => return [0, 0],
            };
            let level = block
                .properties
                .iter()
                .find(|(key, _)| key == "level")
                .and_then(|(_, value)| value.parse::<u8>().ok())
                .unwrap_or(0);
            [kind, level]
        })
        .collect()
}

/// The feet cell of a living, alive mob or player whose feet are inside a
/// cauldron's interior box (not the rim or wall); the block is read later.
#[inline]
fn cauldron_cell(body: &crate::entities::Body, pos: [f64; 3]) -> Option<(i32, i32, i32)> {
    if !body.alive
        || !matches!(
            body.kind,
            crate::entities::EntityKind::Mob { .. } | crate::entities::EntityKind::Player { .. }
        )
    {
        return None;
    }
    let fx = pos[0] - pos[0].floor();
    let fz = pos[2] - pos[2].floor();
    if fx < 0.125 || fx > 0.875 || fz < 0.125 || fz > 0.875 {
        return None;
    }
    let cell = (
        pos[0].floor() as i32,
        pos[1].floor() as i32,
        pos[2].floor() as i32,
    );
    // Feet must be within the cauldron's height, not standing on top.
    if pos[1] - (cell.1 as f64) >= 1.0 || pos[1] - (cell.1 as f64) < 0.0 {
        return None;
    }
    Some(cell)
}

/// Pure per-row classifier. Quiet survivors only need `retained_burn`, and
/// only while a burn timer or the burning bit is live: both are read here, on
/// the pool, so the serial owner never touches the cold row of a cold mob. A
/// cauldron under the feet is found here too; no block write of this pass
/// turns a non-cauldron cell into one, so the serial pass need only revisit
/// the flagged rows and re-read their live state.
fn classify_row(
    pass: &Pass,
    index: usize,
    stamp: &mut Option<crate::world::ContactStamp<2>>,
) -> Option<ContactWork> {
    let entities = pass.entities;
    let body = &entities.bodies()[index];
    let mut work = ContactWork::default();
    if let Some(cell) = cauldron_cell(body, entities.positions()[index]) {
        #[cfg(test)]
        {
            work.sampled = true;
        }
        work.cauldron = pass
            .world
            .block_at(cell.0, cell.1, cell.2)
            .is_some_and(|id| pass.cauldrons.get(id as usize).is_some_and(|c| c[0] != 0));
    }
    match classify_contact(pass, index, stamp) {
        None => {}
        Some(Verdict::Scan) => work.scan = true,
        Some(Verdict::Quiet) => {
            #[cfg(test)]
            {
                work.quiet = true;
            }
            work.burn = body.burn_ticks != 0
                || entities
                    .row(crate::entities::EntitySlot(index as u32))
                    .appearance
                    .shared_flags
                    & 1
                    != 0;
        }
    }
    let any = work.scan || work.burn || work.cauldron;
    #[cfg(test)]
    let any = any || work.quiet || work.sampled;
    any.then_some(work)
}

/// Read-only inputs shared by every row of one classification pass.
struct Pass<'a> {
    entities: &'a crate::entities::EntityStore,
    world: &'a crate::world::ChunkWorld,
    registry: &'a crate::registry::CoreRegistry,
    hazard: &'a crate::world::SectionMatchCache,
    cauldrons: &'a [[u8; 2]],
    border: crate::command_border::Border,
}

/// Whether the exact cells of one body hold neither a suffocating eye block
/// nor a hazard (body cells or magma support): the verdict the serial scan
/// would reach for a row with no hazard, as pure reads.
///
/// The section-palette gates alone rule out most rows. A gate that fires only
/// says some state in the 16-high section matches (a roof, a wall or a pond
/// elsewhere in it), so the exact cells are read here, on the pool, instead of
/// handing every moved mob in such a section to the serial scan. Missing
/// chunks read as `None`, which neither suffocates nor holds a hazard, as in
/// [`scan_row`]; overlay edits are seen through `block_at`.
fn exactly_clear(
    pass: &Pass,
    min: (i32, i32, i32),
    max: (i32, i32, i32),
    [support, eye]: [[i32; 3]; 2],
) -> bool {
    let (world, hazard, registry) = (pass.world, pass.hazard, pass.registry);
    let suffocates = |id: u32| registry.light.suffocating(id) == Some(true);
    if world.may_hold_matching_state_shared(
        (eye[0], eye[1], eye[2]),
        (eye[0], eye[1], eye[2]),
        hazard,
        |seen| suffocates(seen.0),
    ) && world
        .block_at(eye[0], eye[1], eye[2])
        .is_some_and(suffocates)
    {
        return false;
    }
    let flags = &registry.hazard_flags;
    if !world.may_hold_matching_state_shared((min.0, support[1], min.2), max, hazard, |seen| {
        flags.get(seen.0 as usize).copied().unwrap_or(0) != 0
    }) {
        return true;
    }
    let flag_at = |x, y, z| {
        world
            .block_at(x, y, z)
            .and_then(|id| flags.get(id as usize).copied())
            .unwrap_or(0)
    };
    flag_at(support[0], support[1], support[2]) & HAZARD_MAGMA == 0
        && (min.0..=max.0)
            .all(|x| (min.1..=max.1).all(|y| (min.2..=max.2).all(|z| flag_at(x, y, z) == 0)))
}

/// Body/position part of the classifier: scan, quiet, or neither.
///
/// A mob without a valid proof (new, moved to other cells, or its chunk
/// changed) is proven quiet here when its exact eye cell, body cells and
/// foot-support cell hold no hazard or suffocating state ([`exactly_clear`]):
/// the serial scan would then find nothing, bank this exact stamp and only run
/// `retained_burn`. Rows with a real hazard or suffocation keep the exact
/// serial scan.
fn classify_contact(
    pass: &Pass,
    index: usize,
    stamp: &mut Option<crate::world::ContactStamp<2>>,
) -> Option<Verdict> {
    let (entities, world, hazard) = (pass.entities, pass.world, pass.hazard);
    let row = &entities.bodies()[index];
    let type_id = match row.kind {
        crate::entities::EntityKind::Mob { type_id } => type_id,
        crate::entities::EntityKind::Player { .. } | crate::entities::EntityKind::Item { .. } => {
            return Some(Verdict::Scan);
        }
        _ => {
            *stamp = None;
            return None;
        }
    };
    // Data-defined blueprints are living through their presentation type.
    let living = LIVING_BY_TYPE
        .get(crate::blueprint::wire_type_id(type_id) as usize)
        .copied()
        .unwrap_or(false);
    if !row.alive || !living {
        *stamp = None;
        return None;
    }
    let pos = entities.positions()[index];
    let shape = [row.half_width, row.height];
    let previous = stamp.as_ref().filter(|stamp| stamp.entity == row.id);
    let foot_xz = if let Some(stamp) = previous
        && stamp.position[0] == pos[0]
        && stamp.position[2] == pos[2]
    {
        [stamp.probes[0][0], stamp.probes[0][2]]
    } else {
        [floor_i32(pos[0]), floor_i32(pos[2])]
    };
    if !world.is_active(foot_xz[0].div_euclid(16), foot_xz[1].div_euclid(16)) {
        *stamp = None;
        return None;
    }
    let border = pass.border;
    if border.damage_per_block > 0.0
        && (pos[0] - border.center_x)
            .abs()
            .max((pos[2] - border.center_z).abs())
            > border.size * 0.5 + border.safe_zone
    {
        return Some(Verdict::Scan);
    }
    let (quiet, computed) = stamp
        .as_mut()
        .filter(|stamp| stamp.entity == row.id)
        .map_or((false, None), |stamp| {
            quiet_proof(world, hazard, stamp, pos, shape, foot_xz)
        });
    if quiet {
        return Some(Verdict::Quiet);
    }
    let (min, max, probes) = computed.unwrap_or_else(|| contact_cells(pos, shape, foot_xz));
    if !exactly_clear(pass, min, max, probes) {
        return Some(Verdict::Scan);
    }
    *stamp = Some(crate::world::ContactStamp {
        entity: row.id,
        position: pos,
        shape,
        min,
        max,
        probes,
        revision: world.revision_now(),
    });
    Some(Verdict::Quiet)
}

/// Classify every row on the pool (pure reads), then run the exact serial
/// scan, in row order, only for rows that need it, so intent, fire and
/// appearance sequences match the all-serial pass. Cauldron contact follows
/// all scans, as before, over the rows the classifier flagged.
pub(crate) fn run_with(state: &mut TickState, intents: &mut Vec<Intent>, min_parallel: usize) {
    // Take the reused predicate caches out of tick state (restored at the end
    // so capacity survives across ticks); reset once for this read-only pass.
    // `negative` retains capacity; stale slots beyond the live length are ignored.
    let mut scratch = std::mem::take(&mut state.contact_hazards_scratch);
    scratch.hazard.reset(&state.world);
    scratch.suffocation.reset(&state.world);
    if scratch.cauldron.is_empty() {
        scratch.cauldron = cauldron_table(state);
    }
    let rows = state.entities.len();
    let phase_start = std::env::var_os("MC_CORE_PHASE_LOG")
        .is_some()
        .then(std::time::Instant::now);
    #[cfg(test)]
    count_work(0, rows);
    if scratch.negative.len() < rows {
        scratch.negative.resize(rows, None);
    }
    let pending = {
        let pass = Pass {
            entities: &state.entities,
            world: &state.world,
            registry: &state.registry,
            hazard: &scratch.hazard,
            cauldrons: &scratch.cauldron,
            border: state.border,
        };
        classify_rows(
            &mut scratch.negative[..rows],
            min_parallel,
            |index, stamp| classify_row(&pass, index, stamp),
        )
    };
    let classified = std::time::Instant::now();
    // This pass cannot spawn/remove rows; indexing avoids an ID snapshot.
    for &(index, ref work) in pending.iter().flatten() {
        #[cfg(test)]
        if work.quiet {
            count_work(2, 1);
        }
        if work.scan {
            #[cfg(test)]
            count_work(1, 1);
            scan_row(state, &mut scratch, intents, index as usize);
        } else if work.burn {
            retained_burn(
                state,
                crate::entities::EntitySlot(index),
                false,
                false,
                intents,
            );
        }
    }
    for &(index, ref work) in pending.iter().flatten() {
        #[cfg(test)]
        if work.sampled {
            count_work(4, 1);
        }
        if work.cauldron {
            cauldron_row(
                state,
                &scratch.cauldron,
                crate::entities::EntitySlot(index),
                intents,
            );
        }
    }
    state.contact_hazards_scratch = scratch;
    if let Some(started) = phase_start
        && rows > 1000
    {
        let (scans, burns, cauldrons) = pending.iter().flatten().fold((0, 0, 0), |sum, (_, w)| {
            (
                sum.0 + usize::from(w.scan),
                sum.1 + usize::from(!w.scan && w.burn),
                sum.2 + usize::from(w.cauldron),
            )
        });
        eprintln!(
            "  contact detail ms: rows={rows} classify={:.2} serial={:.2} total={:.2} scans={scans} burns={burns} cauldrons={cauldrons}",
            (classified - started).as_secs_f64() * 1000.0,
            classified.elapsed().as_secs_f64() * 1000.0,
            started.elapsed().as_secs_f64() * 1000.0,
        );
    }
}

/// Vanilla cauldron contents contact for one flagged row: a burning living
/// entity inside a water cauldron is extinguished (one level debited); powder
/// snow melts one level into water; a living entity inside lava is burned and
/// ignited. Only the interior counts, not touching the exterior wall. The
/// block is re-read live: an earlier row of this pass may have debited it.
fn cauldron_row(
    state: &mut TickState,
    cauldrons: &[[u8; 2]],
    slot: crate::entities::EntitySlot,
    intents: &mut Vec<Intent>,
) {
    let body = state.entities.body(slot);
    let Some(cell) = cauldron_cell(body, state.entities.pos(slot)) else {
        return;
    };
    let Some(current) = state.world.block_at(cell.0, cell.1, cell.2) else {
        return;
    };
    let [kind, level] = cauldrons.get(current as usize).copied().unwrap_or([0, 0]);
    let id = body.id;
    match kind {
        CAULDRON_WATER | CAULDRON_POWDER_SNOW if level > 0 && body.burn_ticks > 0 => {
            let (body, row) = state.entities.parts_mut(slot);
            body.burn_ticks = 0;
            Arc::make_mut(&mut row.appearance).shared_flags &= !1;
            state.appearance_changed.push(id);
            lower_cauldron(state, cell, current, level, kind == CAULDRON_POWDER_SNOW);
        }
        CAULDRON_LAVA => {
            damage(state, id, DamageKind::Lava, 4.0, intents);
            ignite(state, id, 300);
        }
        _ => {}
    }
}

/// The pre-split cauldron pass over every row, by block name. Differential
/// tests compare it against the flagged-row pass.
#[cfg(test)]
pub(crate) fn cauldron_contact(state: &mut TickState, intents: &mut Vec<Intent>) {
    let count = state.entities.len();
    for index in 0..count {
        let slot = crate::entities::EntitySlot(index as u32);
        let body = state.entities.body(slot);
        let pos = state.entities.pos(slot);
        if !body.alive {
            continue;
        }
        let living = matches!(
            body.kind,
            crate::entities::EntityKind::Mob { .. } | crate::entities::EntityKind::Player { .. }
        );
        if !living {
            continue;
        }
        // Interior: feet inside the cell's inset box, not on the rim or wall.
        let fx = pos[0] - pos[0].floor();
        let fz = pos[2] - pos[2].floor();
        if fx < 0.125 || fx > 0.875 || fz < 0.125 || fz > 0.875 {
            continue;
        }
        let cell = (
            pos[0].floor() as i32,
            pos[1].floor() as i32,
            pos[2].floor() as i32,
        );
        // Feet must be within the cauldron's height, not standing on top.
        if pos[1] - (cell.1 as f64) >= 1.0 || pos[1] - (cell.1 as f64) < 0.0 {
            continue;
        }
        #[cfg(test)]
        count_work(4, 1);
        let Some(current) = state.world.block_at(cell.0, cell.1, cell.2) else {
            continue;
        };
        let Some(block) = state.registry.blocks.by_id(mc_world::BlockStateId(current)) else {
            continue;
        };
        let name = block.block.id.path().to_owned();
        let level = block
            .properties
            .iter()
            .find(|(key, _)| key == "level")
            .and_then(|(_, value)| value.parse::<u8>().ok())
            .unwrap_or(0);
        let id = body.id;
        match name.as_str() {
            "water_cauldron" if level > 0 && body.burn_ticks > 0 => {
                let (body, row) = state.entities.parts_mut(slot);
                body.burn_ticks = 0;
                Arc::make_mut(&mut row.appearance).shared_flags &= !1;
                state.appearance_changed.push(id);
                lower_cauldron(state, (cell.0, cell.1, cell.2), current, level, false);
            }
            "powder_snow_cauldron" if level > 0 && body.burn_ticks > 0 => {
                let (body, row) = state.entities.parts_mut(slot);
                body.burn_ticks = 0;
                Arc::make_mut(&mut row.appearance).shared_flags &= !1;
                state.appearance_changed.push(id);
                lower_cauldron(state, (cell.0, cell.1, cell.2), current, level, true);
            }
            "lava_cauldron" => {
                damage(state, id, DamageKind::Lava, 4.0, intents);
                ignite(state, id, 300);
            }
            _ => {}
        }
    }
}

/// Debit one cauldron level serially: water drops one level (1 empties);
/// powder snow melts into water one level lower.
fn lower_cauldron(
    state: &mut TickState,
    cell: (i32, i32, i32),
    current: u32,
    level: u8,
    snow: bool,
) {
    use mc_data::Identifier;
    let next = if snow {
        if level <= 1 {
            state
                .registry
                .blocks
                .block(&Identifier::parse("minecraft:cauldron").expect("vanilla block"))
                .expect("cauldron")
                .default
                .0
        } else {
            state
                .registry
                .blocks
                .by_name_and_props(
                    &Identifier::parse("minecraft:water_cauldron").expect("vanilla block"),
                    &[("level".into(), (level - 1).to_string())],
                )
                .expect("water cauldron level")
                .0
        }
    } else if level <= 1 {
        state
            .registry
            .blocks
            .block(&Identifier::parse("minecraft:cauldron").expect("vanilla block"))
            .expect("cauldron")
            .default
            .0
    } else {
        state
            .registry
            .blocks
            .by_name_and_props(
                &Identifier::parse("minecraft:water_cauldron").expect("vanilla block"),
                &[("level".into(), (level - 1).to_string())],
            )
            .expect("water cauldron level")
            .0
    };
    let at = crate::boundary::BlockPos {
        x: cell.0,
        y: cell.1,
        z: cell.2,
    };
    let _ = current;
    state.world.set_block(at.x, at.y, at.z, next);
    if let Some((_, value)) = state
        .merge_block_writes
        .iter_mut()
        .find(|(written, _)| *written == at)
    {
        *value = next;
    } else {
        state.merge_block_writes.push((at, next));
    }
}

/// The pre-split behaviour: the exact serial scan over every row. Differential
/// tests compare it against [`run_with`] on any thread count.
#[cfg(test)]
pub(crate) fn run_all_serial(state: &mut TickState, intents: &mut Vec<Intent>) {
    let mut scratch = std::mem::take(&mut state.contact_hazards_scratch);
    scratch.hazard.reset(&state.world);
    scratch.suffocation.reset(&state.world);
    if scratch.negative.len() < state.entities.len() {
        scratch.negative.resize(state.entities.len(), None);
    }
    for index in 0..state.entities.len() {
        scan_row(state, &mut scratch, intents, index);
    }
    state.contact_hazards_scratch = scratch;
    cauldron_contact(state, intents);
}

fn scan_row(state: &mut TickState, scratch: &mut Scratch, intents: &mut Vec<Intent>, index: usize) {
    let living_by_type = &*LIVING_BY_TYPE;
    let slot = crate::entities::EntitySlot(index as u32);
    let body = state.entities.body(slot);
    let kind = body.kind;
    let is_player = matches!(kind, crate::entities::EntityKind::Player { .. });
    let living = if is_player {
        true
    } else if let crate::entities::EntityKind::Mob { type_id } = kind {
        living_by_type
            .get(crate::blueprint::wire_type_id(type_id) as usize)
            .copied()
            .unwrap_or(false)
    } else {
        false
    };
    let item = matches!(kind, crate::entities::EntityKind::Item { .. });
    let entity = body.id;
    if !body.alive || (!living && !item) {
        scratch.negative[index] = None;
        return;
    }
    let pos = state.entities.pos(slot);
    let shape = [body.half_width, body.height];
    let previous = scratch.negative[index]
        .as_ref()
        .filter(|stamp| !item && stamp.entity == entity);
    // Reuse exact foot coordinates as well as body cells on a resting row.
    // Activity still gets checked every pass, even when content is unchanged.
    let foot_xz = if let Some(stamp) = previous
        && stamp.position[0] == pos[0]
        && stamp.position[2] == pos[2]
    {
        [stamp.probes[0][0], stamp.probes[0][2]]
    } else {
        [floor_i32(pos[0]), floor_i32(pos[2])]
    };
    let mut sneaking = state.entities.row(slot).appearance.shared_flags & 2 != 0;
    let mut moving = true;
    let mut immune = false;
    // Non-players keep `moving = true`; only players have player-map rows.
    if is_player && let Some(player) = state.players.get_mut(&entity) {
        immune = player.creative() || player.spectator() || player.vitals.is_dead();
        moving = player.last_contact_position.is_some_and(|old| {
            (old[0] - pos[0]).abs() > 1.0e-9 || (old[2] - pos[2]).abs() > 1.0e-9
        });
        player.last_contact_position = Some(pos);
        sneaking |= state.entities.row(slot).appearance.pose == 5;
    }
    if immune {
        // Contact hazards do not apply, but the fire clock still runs and water
        // still douses it (vanilla Entity.baseTick), so a creative player does
        // not leave creative burning. The damage pass is discarded.
        let water = crate::conduit::water(state, [pos[0], pos[1] + shape[1] * 0.5, pos[2]]);
        retained_burn(state, slot, water, false, &mut Vec::new());
        scratch.negative[index] = None;
        return;
    }
    if living && state.border.damage_per_block > 0.0 {
        let border = state.border;
        let outside = (pos[0] - border.center_x)
            .abs()
            .max((pos[2] - border.center_z).abs())
            - border.size * 0.5
            - border.safe_zone;
        if outside > 0.0 {
            damage(
                state,
                entity,
                DamageKind::OutsideBorder,
                (outside * border.damage_per_block) as f32,
                intents,
            );
        }
    }
    if !state
        .world
        .is_active(foot_xz[0].div_euclid(16), foot_xz[1].div_euclid(16))
    {
        scratch.negative[index] = None;
        return;
    }
    let row = state.entities.row(slot);
    let (quiet, computed) = scratch.negative[index]
        .as_mut()
        .filter(|stamp| !item && stamp.entity == entity)
        .map(|stamp| quiet_proof(&state.world, &scratch.hazard, stamp, pos, shape, foot_xz))
        .unwrap_or((false, None));
    if quiet {
        retained_burn(state, slot, false, false, intents);
        return;
    }
    let (body_min, body_max, [support, eye]) =
        computed.unwrap_or_else(|| contact_cells(pos, shape, foot_xz));
    let eye_y = eye[1];
    // Gate the eye-cell read on resident sections holding any
    // suffocating state; overlay/missing chunks fail open to the exact
    // read so a fresh wall still suffocates.
    let eye_gate = state.world.may_hold_matching_state_cached(
        (eye[0], eye_y, eye[2]),
        (eye[0], eye_y, eye[2]),
        &mut scratch.suffocation,
        |seen| state.registry.light.suffocating(seen.0) == Some(true),
    );
    // Player-only: after the suffocation probe so mobs skip the lookup.
    let frost_walker = is_player
        && state.players.get(&entity).is_some_and(|p| {
            p.inventory.slots[36]
                .enchantments
                .iter()
                .any(|enchantment| {
                    enchantment.id.as_str() == "minecraft:frost_walker" && enchantment.level > 0
                })
        });
    // Gate body hazard samples (plus the magma support read) on section
    // palettes matching any nonzero hazard flag. The box conservatively
    // covers every scanned body cell plus the foot-center support cell
    // below; overlay/missing chunks fail open to the exact scan.
    let gate_min = (body_min.0, support[1], body_min.2);
    let gate_max = body_max;
    // A raw hit blocks memoization even when sneaking, frost-walker or
    // berry-motion rules suppress its current damage.
    let mut raw_body_hazard = false;
    let gated = {
        let flags = &state.registry.hazard_flags;
        state.world.may_hold_matching_state_cached(
            gate_min,
            gate_max,
            &mut scratch.hazard,
            |seen| flags.get(seen.0 as usize).copied().unwrap_or(0) != 0,
        )
    };
    // Borrow a resident column once for multi-cell hazard scans. Otherwise
    // retain the exact overlay-first reader; an eye-only pass needs just
    // one lookup and does not benefit from the column checks.
    let borrowed_chunk = if gated && state.world.overlay_is_empty() {
        let column = crate::boundary::ChunkPos {
            x: body_min.0.div_euclid(16),
            z: body_min.2.div_euclid(16),
        };
        (body_max.0.div_euclid(16) == column.x
            && body_max.2.div_euclid(16) == column.z
            && support[0].div_euclid(16) == column.x
            && support[2].div_euclid(16) == column.z
            && eye[0].div_euclid(16) == column.x
            && eye[2].div_euclid(16) == column.z)
            .then(|| state.world.chunk(column))
            .flatten()
    } else {
        None
    };
    let block_at = |x: i32, y: i32, z: i32| {
        #[cfg(test)]
        count_work(3, 1);
        match borrowed_chunk {
            Some(chunk) => chunk
                .get_block(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
                .map(|id| id.0),
            None => state.world.block_at(x, y, z),
        }
    };
    let flag_at = |x, y, z| {
        block_at(x, y, z)
            .and_then(|id| state.registry.hazard_flags.get(id as usize).copied())
            .unwrap_or(0)
    };
    let suffocates = eye_gate
        && block_at(eye[0], eye_y, eye[2])
            .is_some_and(|id| state.registry.light.suffocating(id) == Some(true));
    let raw_magma_support =
        gated && flag_at(support[0], support[1], support[2]) & HAZARD_MAGMA != 0;
    let hot = gated && !sneaking && !frost_walker && raw_magma_support;
    let (mut water, mut lava, mut fire, mut cactus, mut berry, mut campfire, mut soul_campfire) =
        (false, false, false, false, false, false, false);
    if gated {
        for x in body_min.0..=body_max.0 {
            for y in body_min.1..=body_max.1 {
                for z in body_min.2..=body_max.2 {
                    let flags = flag_at(x, y, z);
                    if flags == 0 {
                        continue;
                    }
                    raw_body_hazard = true;
                    // State flags bake in age/lit properties; movement
                    // and posture still gate only the damage intent.
                    if flags & HAZARD_WATER != 0 {
                        water = true;
                    } else if flags & HAZARD_LAVA != 0 {
                        lava = true;
                    } else if flags & HAZARD_FIRE != 0 {
                        fire = true;
                    } else if flags & HAZARD_CACTUS != 0 {
                        cactus = true;
                    } else if flags & HAZARD_BERRY != 0 {
                        berry |= moving;
                    } else if flags & HAZARD_CAMPFIRE != 0 {
                        campfire = true;
                    } else if flags & HAZARD_SOUL_CAMPFIRE != 0 {
                        soul_campfire = true;
                    }
                }
            }
        }
    }
    if item {
        let kind = if cactus {
            Some(DamageKind::Cactus)
        } else if lava {
            Some(DamageKind::Lava)
        } else if fire {
            Some(DamageKind::InFire)
        } else {
            None
        };
        if let Some(kind) = kind
            && !row
                .item
                .as_ref()
                .is_some_and(|item| crate::item_hazards::resistant(state, item, kind))
        {
            damage(state, entity, kind, 1.0, intents);
        }
        scratch.negative[index] = None;
        return;
    }
    if suffocates {
        damage(state, entity, DamageKind::Suffocation, 1.0, intents)
    }
    if cactus {
        damage(state, entity, DamageKind::Cactus, 1.0, intents)
    }
    if berry {
        damage(state, entity, DamageKind::SweetBerryBush, 1.0, intents)
    }
    if hot {
        damage(state, entity, DamageKind::HotFloor, 1.0, intents)
    }
    // Vanilla campfires hurt sneaking players; Frost Walker boots (which
    // freeze the fire's footing) prevent it. Soul fires deal 2 HP per hit.
    if (campfire || soul_campfire) && !frost_walker {
        damage(
            state,
            entity,
            DamageKind::Campfire,
            if soul_campfire { 2.0 } else { 1.0 },
            intents,
        )
    }
    if lava {
        damage(state, entity, DamageKind::Lava, 4.0, intents)
    } else if fire {
        damage(state, entity, DamageKind::InFire, 1.0, intents)
    }
    // Evaluated only for burning contact so the common case skips the
    // player-map and gamerule probes; identical truth table to before.
    if (lava || fire)
        && !water
        && (!state.players.contains_key(&entity)
            || crate::command_rules::boolean(state, crate::command_rules::BoolRule::FIRE_DAMAGE))
    {
        ignite(state, entity, if lava { 300 } else { 160 });
    }
    // Bank raw absence independently of the live burn clock. A newly
    // ignited body can remain geometrically quiet throughout its burn.
    scratch.negative[index] = if !suffocates && !raw_body_hazard && !raw_magma_support {
        Some(crate::world::ContactStamp {
            entity,
            position: pos,
            shape,
            min: body_min,
            max: body_max,
            probes: [support, eye],
            revision: state.world.revision_now(),
        })
    } else {
        None
    };
    retained_burn(state, slot, water, lava || fire, intents);
}
