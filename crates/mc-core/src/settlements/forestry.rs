//! Funded site preparation: immutable worldgen evidence, never admission writes.
use std::collections::VecDeque;
use std::sync::mpsc;

use mc_world::{BlockStateId, Chunk, ChunkPos};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

use crate::tick::TickState;

// Covers the largest legal settlement, with room for boundary chunks. Old
// towns may evict immutable evidence; they can request it again when evaluated.
const CACHE_CHUNKS: usize = 1_156;
const CACHE_BYTES: usize = 64 * 1024 * 1024;
const QUEUED_CHUNKS: usize = 32;

pub(super) const PLANTED_CROWN_RADIUS: u32 = 4;

#[path = "forestry_tree_proof.rs"]
mod tree_proof;

/// Selection owns one bounded proof per root. The executor instead rechecks
/// intrinsic vetoes on cells already certified in its canonical work manifest;
/// it must not repeat a volume scan, or depend on a root retired while felling.
pub(crate) struct Clearance {
    proofs: Option<HashMap<[i32; 3], tree_proof::Proof>>,
    /// Roots whose whole tree is felled for this scan. Deterministic order and
    /// no closure: only cells certified on a lot, corridor or upgrade diff mark.
    needed: std::collections::BTreeSet<[i32; 3]>,
    grove: tree_proof::Grove,
    /// The manifest may carry cells outside the authored lot (whole trees).
    lot_free: bool,
    /// Executor mode: a log must prove its own identity against worldgen or a
    /// registered root column instead of trusting the manifest.
    verify: bool,
    /// The last `clearable` lost forest evidence that is still being generated.
    missed: bool,
}

impl Default for Clearance {
    fn default() -> Self {
        Self {
            proofs: Some(HashMap::default()),
            needed: std::collections::BTreeSet::new(),
            grove: tree_proof::Grove::default(),
            lot_free: false,
            verify: false,
            missed: false,
        }
    }
}

impl Clearance {
    pub(crate) fn prepared() -> Self {
        Self {
            proofs: None,
            lot_free: true,
            ..Self::default()
        }
    }

    /// A per-cell identity recheck for the canonical executor. No volume proof:
    /// a log is either worldgen evidence or a log inside a registered root's
    /// own column, and leaves need none because a placed leaf is persistent.
    pub(crate) fn verify() -> Self {
        Self {
            proofs: None,
            verify: true,
            ..Self::prepared()
        }
    }

    pub(crate) fn lost_evidence(&self) -> bool {
        self.missed
    }

    fn owned(&mut self, state: &TickState, at: [i32; 3], leaf: bool) -> bool {
        let Some(_) = self.proofs else {
            return true;
        };
        let roots: Vec<[i32; 3]> = state
            .settlements
            .records()
            .flat_map(|record| record.planted_trees.iter().copied())
            .filter(|&root| in_halo(root, at))
            .collect();
        if roots.is_empty() {
            return false;
        }
        {
            let grove = &mut self.grove;
            let proofs = self.proofs.as_mut().expect("selection proofs");
            for &root in &roots {
                if !proofs.contains_key(&root) {
                    proofs.insert(root, tree_proof::Proof::new(state, root, grove));
                }
            }
        }
        let mut certified = false;
        let mut contributors: Vec<[i32; 3]> = Vec::new();
        {
            let proofs = self.proofs.as_ref().expect("selection proofs");
            for root in &roots {
                let Some(proof) = proofs.get(root) else {
                    continue;
                };
                let owns = if leaf {
                    proof.permits(at)
                } else {
                    proof.owns_log(at)
                };
                if owns {
                    certified = true;
                    contributors.extend(proof.contributors.iter().copied());
                }
            }
        }
        if !certified {
            return false;
        }
        // Every tree whose own log stands within RANGE of a certified cell is
        // felled whole; the walk stays one hop wide, so no grove chain grows.
        for other in contributors {
            if self.needed.contains(&other) {
                continue;
            }
            let within = self.grove.component(state, other).is_some_and(|logs| {
                logs.iter().any(|log| {
                    (log[0] - at[0]).abs() + (log[1] - at[1]).abs() + (log[2] - at[2]).abs()
                        <= tree_proof::RANGE
                })
            });
            if within {
                self.needed.insert(other);
            }
        }
        true
    }

    pub(crate) fn append_whole_trees(
        &mut self,
        state: &TickState,
        cells: &mut Vec<crate::plugin_structures::settlement_builds::PlannedCell>,
    ) {
        // The proofs are parked while the grove memo is used, so a crown check
        // can walk a neighbour's component without borrowing this root's proof.
        let Some(mut proofs) = self.proofs.take() else {
            return;
        };
        if self.needed.is_empty() {
            self.proofs = Some(proofs);
            return;
        }
        // A root may be marked needed from a neighbour's proof, so build its
        // own proof on demand from the same scan-wide component memo.
        let needed: Vec<[i32; 3]> = self.needed.iter().copied().collect();
        for &root in &needed {
            if !proofs.contains_key(&root) {
                proofs.insert(root, tree_proof::Proof::new(state, root, &mut self.grove));
            }
        }
        let mut seen: HashSet<_> = cells.iter().map(|cell| cell.at).collect();
        for root in needed {
            let Some(proof) = proofs.get(&root) else {
                continue;
            };
            for at in proof.positions(root) {
                let Some(block) = state.world.physics_block_at(at[0], at[1], at[2]) else {
                    continue;
                };
                if !tree_or_sapling(state, block) {
                    continue;
                }
                let leaf = state
                    .registry
                    .blocks
                    .by_id(BlockStateId(block))
                    .is_some_and(|b| b.block.id.path().ends_with("_leaves"));
                if leaf && !self.crown_is_this_trees(root, &proof.contributors, state, at) {
                    continue;
                }
                if !(proof.owns_log(at) || (leaf && proof.permits(at)))
                    || vetoed(state, at)
                    || (leaf && persistent(state, block))
                    || !seen.insert(at)
                {
                    continue;
                }
                cells.push(crate::plugin_structures::settlement_builds::PlannedCell {
                    at,
                    state: state.registry.states.air,
                    resource: None,
                    quantity: 0,
                });
            }
        }
        self.proofs = Some(proofs);
    }

    /// A crown cell belongs to `root` only when no other tree of its own box
    /// reaches it: felling a leaf another (unneeded) registered tree supports
    /// would mutilate that tree instead of clearing a grove.
    fn crown_is_this_trees(
        &mut self,
        root: [i32; 3],
        contributors: &[[i32; 3]],
        state: &TickState,
        at: [i32; 3],
    ) -> bool {
        let others: Vec<[i32; 3]> = contributors
            .iter()
            .copied()
            .filter(|other| *other != root && !self.needed.contains(other))
            .collect();
        for other in others {
            let reach = self.grove.component(state, other).is_some_and(|logs| {
                logs.iter().any(|log| {
                    (log[0] - at[0]).abs() + (log[1] - at[1]).abs() + (log[2] - at[2]).abs()
                        <= tree_proof::RANGE
                })
            });
            if reach {
                return false;
            }
        }
        true
    }
}

pub(crate) fn vetoed(state: &TickState, at: [i32; 3]) -> bool {
    state.block_log.player_touched(at) || near_unlogged_player(state, at)
}

pub(crate) fn persistent(state: &TickState, block: u32) -> bool {
    state
        .registry
        .blocks
        .by_id(BlockStateId(block))
        .is_some_and(|definition| {
            definition
                .properties
                .iter()
                .any(|(key, value)| key == "persistent" && value == "true")
        })
}

type Generated = (ChunkPos, Option<Evidence>, u128);

fn tree_name(name: &str) -> bool {
    name.ends_with("_leaves") || (name.ends_with("_log") && !name.starts_with("stripped_"))
}

fn tree_states(blocks: &mc_world::BlockRegistry) -> Vec<bool> {
    blocks
        .states()
        .map(|definition| tree_name(definition.block.id.path()))
        .collect()
}

/// Only tree cells can authorize clearing; all other generated states are
/// represented by air. Positions are ordered by y, x, z for binary search.
struct Evidence {
    min_y: i32,
    max_y: i32,
    cells: Vec<(u32, u32)>,
}

impl Evidence {
    fn from_chunk(chunk: &Chunk, tree_states: &[bool]) -> Self {
        let geometry = chunk.geometry();
        let mut cells = Vec::new();
        let is_tree = |id: BlockStateId| tree_states.get(id.0 as usize).copied().unwrap_or(false);
        for (section_index, section) in chunk.sections.iter().enumerate() {
            if !section.palette().map_or_else(
                || is_tree(section.get(0, 0, 0)),
                |palette| palette.iter().copied().any(is_tree),
            ) {
                continue;
            }
            for y in 0..16 {
                for x in 0..16 {
                    for z in 0..16 {
                        let id = section.get(x, y, z);
                        if is_tree(id) {
                            let index = section_index as u32 * 4096
                                + u32::from(y) * 256
                                + u32::from(x) * 16
                                + u32::from(z);
                            cells.push((index, id.0));
                        }
                    }
                }
            }
        }
        cells.shrink_to_fit();
        Self {
            min_y: geometry.min_y(),
            max_y: geometry.max_y(),
            cells,
        }
    }

    fn bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.cells.capacity() * std::mem::size_of::<(u32, u32)>()
    }

    fn block(&self, at: [i32; 3], air: u32) -> Option<u32> {
        if !(self.min_y..self.max_y).contains(&at[1]) {
            return None;
        }
        let index = ((at[1] - self.min_y) as u32 * 256)
            + (at[0].rem_euclid(16) as u32 * 16)
            + at[2].rem_euclid(16) as u32;
        Some(
            self.cells
                .binary_search_by_key(&index, |&(index, _)| index)
                .map_or(air, |position| self.cells[position].1),
        )
    }
}

#[derive(Default)]
pub(crate) struct Cache {
    ready: HashMap<ChunkPos, Evidence>,
    bytes: usize,
    order: VecDeque<ChunkPos>,
    pending: HashSet<ChunkPos>,
    misses: u64,
    denied: HashSet<ChunkPos>,
    worker: Option<(mpsc::SyncSender<ChunkPos>, mpsc::Receiver<Generated>)>,
    /// The worker owns a clone of the world's chunk source and with it the
    /// native store, so the cache joins it on drop rather than detaching it.
    thread: Option<std::thread::JoinHandle<()>>,
    trusted: Option<bool>,
}

impl std::fmt::Debug for Cache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForestCache")
            .field("ready", &self.ready.len())
            .field("pending", &self.pending.len())
            .field("trusted", &self.trusted)
            .finish()
    }
}

impl Drop for Cache {
    fn drop(&mut self) {
        // Closing both channels stops the worker at its next receive or send,
        // after at most the chunk it is generating; only then is the store
        // released, so a reopen right after a stop finds it closed.
        self.worker = None;
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::error!("settlement forest evidence worker panicked");
        }
    }
}

impl Cache {
    pub(super) fn misses(&self) -> u64 {
        self.misses
    }

    fn block(&mut self, state: &TickState, at: [i32; 3]) -> Option<u32> {
        if !*self.trusted.get_or_insert_with(|| generator_known(state)) {
            return None;
        }
        if self.worker.is_none() {
            let source = state.world.chunk_source();
            let (send, requests) = mpsc::sync_channel::<ChunkPos>(QUEUED_CHUNKS);
            let (results, receive) = mpsc::sync_channel(QUEUED_CHUNKS);
            let spawn = std::thread::Builder::new()
                .name("settlement-forest".into())
                .spawn(move || {
                    let tree_states = tree_states(&source.registry);
                    while let Ok(pos) = requests.recv() {
                        let started = std::time::Instant::now();
                        let trusted = source.native.as_ref().is_none_or(|store| {
                            store.generator_owns_baseline(source.dimension, pos).unwrap_or_else(|error| {
                                tracing::error!(x = pos.x, z = pos.z, %error, "forest baseline provenance read failed");
                                false
                            })
                        });
                        let chunk = trusted.then(|| {
                            Evidence::from_chunk(&source.generator.generate(pos), &tree_states)
                        });
                        if results
                            .send((pos, chunk, started.elapsed().as_micros()))
                            .is_err()
                        {
                            break;
                        }
                    }
                });
            let thread = match spawn {
                Ok(thread) => thread,
                Err(error) => {
                    tracing::error!(%error, "settlement forest evidence worker could not start");
                    self.trusted = Some(false);
                    return None;
                }
            };
            self.worker = Some((send, receive));
            self.thread = Some(thread);
        }
        let (send, receive) = self.worker.as_ref().expect("forest worker");
        while let Ok((pos, chunk, micros)) = receive.try_recv() {
            self.pending.remove(&pos);
            let Some(chunk) = chunk else {
                if self.denied.len() >= CACHE_CHUNKS {
                    self.denied.clear();
                }
                self.denied.insert(pos);
                continue;
            };
            while (self.ready.len() >= CACHE_CHUNKS || self.bytes + chunk.bytes() > CACHE_BYTES)
                && let Some(oldest) = self.order.pop_front()
            {
                if let Some(old) = self.ready.remove(&oldest) {
                    self.bytes -= old.bytes();
                }
            }
            self.bytes += chunk.bytes();
            tracing::debug!(
                x = pos.x,
                z = pos.z,
                micros,
                "settlement forest regeneration cached"
            );
            self.ready.insert(pos, chunk);
            self.order.push_back(pos);
        }
        let pos = ChunkPos {
            x: at[0].div_euclid(16),
            z: at[2].div_euclid(16),
        };
        if let Some(chunk) = self.ready.get(&pos) {
            return chunk.block(at, state.registry.states.air);
        }
        if self.denied.contains(&pos) {
            return None;
        }
        self.misses = self.misses.saturating_add(1);
        if !self.pending.contains(&pos) {
            match send.try_send(pos) {
                Ok(()) => {
                    self.pending.insert(pos);
                }
                Err(mpsc::TrySendError::Full(_)) => {}
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    tracing::error!("settlement forest evidence worker disconnected");
                    self.trusted = Some(false);
                }
            }
        }
        None
    }
}

/// Native startup validates the full contract. Imported and older baselines
/// also require per-chunk generator provenance before they can be cleared.
fn generator_known(state: &TickState) -> bool {
    if state.world.chunk_source().native.is_none() {
        return state
            .native_globals
            .get("level_metadata")
            .is_none_or(|record| {
                matches!(crate::native_checkpoint::decode_tag(record, "level_metadata"),
                Ok(mc_nbt::Tag::Compound(fields)) if fields.is_empty())
            });
    }
    let Some(record) = state.native_globals.get("generation") else {
        return false;
    };
    let Ok(contract) = serde_json::from_slice::<serde_json::Value>(&record.payload) else {
        return false;
    };
    contract["base"]["seed"].as_i64() == Some(state.world.seed())
        && contract["base"]["revision"].as_u64() == Some(u64::from(mc_worldgen::WORLDGEN_REVISION))
}

pub(crate) fn tree(state: &TickState, block: u32) -> bool {
    state
        .registry
        .blocks
        .by_id(BlockStateId(block))
        .is_some_and(|definition| tree_name(definition.block.id.path()))
}

/// A planted root protects its trunk and a bounded crown, not the whole
/// settlement column. Unknown/unloaded cells remain protected.
fn planted(state: &TickState, at: [i32; 3]) -> bool {
    state.settlements.records().any(|record| {
        record.planted_trees.iter().any(|root| {
            let dx = at[0].abs_diff(root[0]);
            let dz = at[2].abs_diff(root[2]);
            if dx > PLANTED_CROWN_RADIUS
                || dz > PLANTED_CROWN_RADIUS
                || at[1] < root[1]
                || at[1] - root[1] > 64
            {
                return false;
            }
            let low = root[1].max(at[1] - 8);
            let high = (root[1] + 64)
                .min(at[1] + 8)
                .min(state.world.geometry().max_y() - 1);
            for y in low..=high {
                let Some(id) = state.world.physics_block_at(root[0], y, root[2]) else {
                    return true;
                };
                if state
                    .registry
                    .blocks
                    .by_id(BlockStateId(id))
                    .is_some_and(|block| {
                        let name = block.block.id.path();
                        name.ends_with("_log") || name.ends_with("_sapling")
                    })
                {
                    return true;
                }
            }
            false
        })
    })
}

/// A registered planting reserves the same possible crown before it grows.
/// Root retirement remains the sole authority for removing vanished plantings.
pub(super) fn planted_crown_overlaps(
    state: &TickState,
    bounds: mc_script::ScriptWorldBounds,
) -> bool {
    let crown = i64::from(PLANTED_CROWN_RADIUS);
    state.settlements.records().any(|record| {
        record.planted_trees.iter().any(|root| {
            [0, 2].into_iter().all(|axis| {
                let coordinate = i64::from(root[axis]);
                coordinate + crown >= i64::from(bounds.min[axis])
                    && coordinate - crown <= i64::from(bounds.max[axis])
            }) && i64::from(root[1]) <= i64::from(bounds.max[1])
                && i64::from(root[1]) + 64 >= i64::from(bounds.min[1])
        })
    })
}

/// A future off-lot crown cannot be put in the work manifest before it grows.
/// Keep its one bounded halo refusal while the registered root is a sapling.
pub(super) fn off_lot_sapling_overlaps(
    state: &TickState,
    bounds: mc_script::ScriptWorldBounds,
) -> bool {
    state.settlements.records().any(|record| {
        record.planted_trees.iter().any(|&root| {
            let inside = [0, 2]
                .into_iter()
                .all(|axis| (bounds.min[axis]..=bounds.max[axis]).contains(&root[axis]));
            !inside
                && [0, 2].into_iter().all(|axis| {
                    root[axis] as i64 + PLANTED_CROWN_RADIUS as i64 >= bounds.min[axis] as i64
                        && root[axis] as i64 - PLANTED_CROWN_RADIUS as i64
                            <= bounds.max[axis] as i64
                })
                && root[1] <= bounds.max[1]
                && root[1] + 64 >= bounds.min[1]
                && state
                    .world
                    .physics_block_at(root[0], root[1], root[2])
                    .is_none_or(|id| {
                        state
                            .registry
                            .blocks
                            .by_id(BlockStateId(id))
                            .is_none_or(|block| block.block.id.path().ends_with("_sapling"))
                    })
        })
    })
}

/// Retire vanished roots only when their whole possible trunk is loaded.
pub(crate) fn retire_missing(state: &mut TickState) {
    if !state.tick.is_multiple_of(20) {
        return;
    }
    let mut removed = Vec::new();
    for record in state.settlements.records() {
        for &root in &record.planted_trees {
            let mut missing = true;
            for y in root[1]..=(root[1] + 64).min(state.world.geometry().max_y() - 1) {
                let Some(id) = state.world.physics_block_at(root[0], y, root[2]) else {
                    missing = false;
                    break;
                };
                if state
                    .registry
                    .blocks
                    .by_id(BlockStateId(id))
                    .is_some_and(|block| {
                        let name = block.block.id.path();
                        name.ends_with("_log") || name.ends_with("_sapling")
                    })
                {
                    missing = false;
                    break;
                }
            }
            if missing {
                removed.push((record.id, root));
            }
        }
    }
    for (id, root) in removed {
        if let Some(record) = state.settlements.get_mut(id)
            && record.planted_trees.remove(&root)
        {
            record.revision = record.revision.saturating_add(1);
        }
    }
}

pub(super) fn protected_tree(state: &TickState, at: [i32; 3]) -> bool {
    state.block_log.player_touched(at) || planted(state, at)
}

pub(crate) fn clearable(state: &TickState, at: [i32; 3]) -> bool {
    let Some(block) = state.world.physics_block_at(at[0], at[1], at[2]) else {
        return false;
    };
    if !tree(state, block) || protected_tree(state, at) || near_unlogged_player(state, at) {
        return false;
    }
    natural(state, at, block)
}

fn near_unlogged_player(state: &TickState, at: [i32; 3]) -> bool {
    // With capture disabled, a nearby live player is an additional veto. The
    // deterministic baseline still protects all nonmatching historical edits.
    !state.block_log.enabled()
        && state.players.keys().any(|id| {
            state.entities.slot_of(*id).is_some_and(|slot| {
                let pos = state.entities.pos(slot);
                (pos[0] - f64::from(at[0])).abs() <= 32.0
                    && (pos[2] - f64::from(at[2])).abs() <= 32.0
            })
        })
}

fn natural(state: &TickState, at: [i32; 3], block: u32) -> bool {
    natural_evidence(state, at, block) == Some(true)
}

/// `None` when the forest cache has no evidence for the cell yet: a caller that
/// may not guess (the executor) retries instead of treating that as a refusal.
fn natural_evidence(state: &TickState, at: [i32; 3], block: u32) -> Option<bool> {
    let generated = state
        .settlements
        .forest
        .lock()
        .expect("forest cache poisoned")
        .block(state, at)?;
    if generated == block {
        return Some(true);
    }
    let actual = state.registry.blocks.by_id(BlockStateId(block))?;
    let baseline = state.registry.blocks.by_id(BlockStateId(generated))?;
    Some(
        actual.block.id == baseline.block.id
            && actual.block.id.path().ends_with("_leaves")
            && actual.properties.len() == baseline.properties.len()
            && actual.properties.iter().all(|(key, value)| {
                key == "distance"
                    || baseline
                        .properties
                        .iter()
                        .any(|(other, expected)| other == key && expected == value)
            }),
    )
}

/// A registered planting's own trunk cell: inside that root's column and height
/// bound. The executor's identity evidence for a log, without any volume proof.
fn registered_column(state: &TickState, at: [i32; 3]) -> bool {
    state.settlements.records().any(|record| {
        record.planted_trees.iter().any(|&root| {
            root[0] == at[0] && root[2] == at[2] && (root[1]..=root[1] + 64).contains(&at[1])
        })
    })
}

pub(crate) fn tree_or_sapling(state: &TickState, block: u32) -> bool {
    state
        .registry
        .blocks
        .by_id(BlockStateId(block))
        .is_some_and(|definition| {
            let name = definition.block.id.path();
            tree_name(name) || name.ends_with("_sapling")
        })
}

pub(crate) fn lot_bounds(
    state: &TickState,
    project: &super::construction::Project,
) -> Option<mc_script::ScriptWorldBounds> {
    let blueprint = super::blueprints::get(&state.registry, &project.blueprint)?;
    let size = blueprint.footprint(super::blueprints::Rotation::from_quarter_turns(
        project.rotation,
    ));
    mc_script::ScriptWorldBounds::new(
        project.origin,
        std::array::from_fn(|axis| project.origin[axis] + i32::from(size[axis]) - 1),
    )
    .ok()
}

fn in_halo(root: [i32; 3], at: [i32; 3]) -> bool {
    at[0].abs_diff(root[0]) <= PLANTED_CROWN_RADIUS
        && at[2].abs_diff(root[2]) <= PLANTED_CROWN_RADIUS
        && (root[1]..=root[1] + 64).contains(&at[1])
}

/// Missing roots cannot turn a conservative protection halo into permission.
fn registered_planting(state: &TickState, at: [i32; 3]) -> Option<bool> {
    let mut registered = false;
    for record in state.settlements.records() {
        for &root in &record.planted_trees {
            if in_halo(root, at) {
                state.world.physics_block_at(root[0], root[1], root[2])?;
                registered |= at == root;
            }
        }
    }
    Some(registered)
}

impl Clearance {
    /// Whole planted components are certified during selection, not per write.
    pub(crate) fn clearable(
        &mut self,
        state: &TickState,
        at: [i32; 3],
        lot: Option<mc_script::ScriptWorldBounds>,
    ) -> bool {
        self.missed = false;
        if vetoed(state, at) {
            return false;
        }
        let on_lot = lot.is_some_and(|bounds| {
            self.lot_free
                || [0, 2]
                    .into_iter()
                    .all(|axis| (bounds.min[axis]..=bounds.max[axis]).contains(&at[axis]))
        });
        if on_lot
            && let Some(block) = state.world.physics_block_at(at[0], at[1], at[2])
            && let Some(definition) = state.registry.blocks.by_id(BlockStateId(block))
        {
            let name = definition.block.id.path();
            if !tree_name(name) && !name.ends_with("_sapling") {
                return false;
            }
            let Some(registered) = registered_planting(state, at) else {
                return false;
            };
            if name.ends_with("_leaves") {
                let nonpersistent = definition
                    .properties
                    .iter()
                    .any(|(key, value)| key == "persistent" && value == "false");
                // A placed leaf is always persistent (placement.rs), and a
                // persistent leaf is refused here, so the executor needs no
                // further identity evidence for leaves.
                return nonpersistent
                    && (self.verify || self.owned(state, at, true) || natural(state, at, block));
            }
            if registered && name.ends_with("_sapling") {
                return true;
            }
            if !tree_name(name) {
                return false;
            }
            if self.verify {
                // The manifest is not evidence of identity: a log is either
                // its own registered trunk column or worldgen evidence.
                if registered_column(state, at) {
                    return true;
                }
                return match natural_evidence(state, at, block) {
                    Some(natural) => natural,
                    None => {
                        self.missed = true;
                        false
                    }
                };
            }
            return self.owned(state, at, false) || natural(state, at, block);
        }
        clearable(state, at)
    }
}

pub(crate) fn drops(
    state: &TickState,
    at: [i32; 3],
    block: u32,
    lot: Option<mc_script::ScriptWorldBounds>,
    clearance: &mut Clearance,
) -> Result<Vec<mc_data::ItemStack>, mc_script::ScriptOperationFailure> {
    if !clearance.clearable(state, at, lot) {
        // Missing forest evidence is a wait, not a permanent refusal.
        return Err(if clearance.lost_evidence() {
            mc_script::ScriptOperationFailure::Unloaded
        } else {
            mc_script::ScriptOperationFailure::Blocked
        });
    }
    roll(state, at, block)
}

/// Vanilla loot of a surface plant cleared without a tool: short grass drops
/// wheat seeds one time in eight. Before its first farm a settlement has no
/// other seed source, and that farm's wheat cells cost seeds. Air and blocks
/// outside the clearable plant veneer roll nothing.
pub(crate) fn plant_drops(
    state: &TickState,
    at: [i32; 3],
    block: u32,
) -> Result<Vec<mc_data::ItemStack>, mc_script::ScriptOperationFailure> {
    if block == state.registry.states.air || !super::construction::clearable(state, block) {
        return Ok(Vec::new());
    }
    roll(state, at, block)
}

pub(crate) fn roll(
    state: &TickState,
    at: [i32; 3],
    block: u32,
) -> Result<Vec<mc_data::ItemStack>, mc_script::ScriptOperationFailure> {
    let seed = at
        .into_iter()
        .fold(state.world.seed() as u64 ^ state.tick, |seed, value| {
            seed.rotate_left(17) ^ (value as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        });
    crate::block_drop_context::roll(
        state,
        crate::boundary::BlockPos {
            x: at[0],
            y: at[1],
            z: at[2],
        },
        block,
        &mc_data::ItemStack::EMPTY,
        seed,
    )
    .map_err(|_| mc_script::ScriptOperationFailure::Blocked)
}

pub(crate) struct Preparation {
    pub(crate) cells: Vec<crate::plugin_structures::settlement_builds::PlannedCell>,
    pub(crate) corridor: Option<mc_script::ScriptWorldBounds>,
}

/// Clear the lot and its walking corridor in physical work order. Only tree
/// blocks are added here; unrelated terrain keeps the planner's old refusals.
pub(crate) fn cells(
    state: &TickState,
    project: &super::construction::Project,
    size: [u8; 3],
    clearance: &mut Clearance,
) -> Option<Preparation> {
    use crate::plugin_structures::settlement_builds::PlannedCell;
    let record = state.settlements.get(project.settlement)?;
    let bounds = lot_bounds(state, project)?;
    // Proofs are scan-wide; the selected whole trees are candidate-local.
    clearance.needed.clear();
    let mut lot = Vec::new();
    let mut lot_has_tree = false;
    for x in project.origin[0]..project.origin[0] + i32::from(size[0]) {
        for z in project.origin[2]..project.origin[2] + i32::from(size[2]) {
            let y = super::construction::surface_y_for_lot(
                state,
                project.origin[1],
                x,
                z,
                Some(bounds),
                clearance,
            )?;
            lot_has_tree |= (y + 1..project.origin[1] + i32::from(size[1])).any(|above| {
                state
                    .world
                    .physics_block_at(x, above, z)
                    .is_some_and(|block| tree_or_sapling(state, block))
            });
            lot.push([x, y, z]);
        }
    }
    let route =
        super::layout::Network::for_construction(state, record).construction_access(project);
    let mut columns = match route {
        Some(route) => route,
        None if !lot_has_tree && !planted_crown_overlaps(state, bounds) => {
            return Some(Preparation {
                cells: Vec::new(),
                corridor: None,
            });
        }
        None => return None,
    };
    for &[x, y, z] in &columns {
        let on_lot = [0, 2]
            .into_iter()
            .all(|axis| (bounds.min[axis]..=bounds.max[axis]).contains(&[x, y, z][axis]));
        if !on_lot
            && planted_crown_overlaps(
                state,
                mc_script::ScriptWorldBounds::new([x, y, z], [x, y + 2, z]).ok()?,
            )
        {
            return None;
        }
    }
    let corridor = if let Some(&first) = columns.first() {
        let mut min = first;
        let mut max = [first[0], first[1] + 2, first[2]];
        for at in &columns[1..] {
            for axis in 0..3 {
                min[axis] = min[axis].min(at[axis]);
                max[axis] = max[axis].max(at[axis] + if axis == 1 { 2 } else { 0 });
            }
        }
        Some(mc_script::ScriptWorldBounds::new(min, max).ok()?)
    } else {
        None
    };
    columns.extend(lot);
    let mut seen = HashSet::default();
    let mut cells = Vec::new();
    for [x, floor, z] in columns {
        if !seen.insert((x, z)) {
            continue;
        }
        let on_lot = (project.origin[0]..project.origin[0] + i32::from(size[0])).contains(&x)
            && (project.origin[2]..project.origin[2] + i32::from(size[2])).contains(&z);
        let top = (floor + 64).min(state.world.geometry().max_y() - 1);
        for y in floor + 1..=top {
            let Some(block) = state.world.physics_block_at(x, y, z) else {
                break;
            };
            if !tree_or_sapling(state, block) {
                continue;
            }
            if !clearance.clearable(state, [x, y, z], Some(bounds)) {
                // A protected crown above the actual work volume stays there.
                if y <= floor + 2 || (on_lot && y < project.origin[1] + i32::from(size[1])) {
                    return None;
                }
                continue;
            }
            cells.push(PlannedCell {
                at: [x, y, z],
                state: state.registry.states.air,
                resource: None,
                quantity: 0,
            });
        }
    }
    clearance.append_whole_trees(state, &mut cells);
    let mut first = 0;
    for index in 0..cells.len() {
        let at = cells[index].at;
        if registered_planting(state, at) == Some(true)
            && state
                .world
                .physics_block_at(at[0], at[1], at[2])
                .is_some_and(|block| {
                    state
                        .registry
                        .blocks
                        .by_id(BlockStateId(block))
                        .is_some_and(|block| block.block.id.path().ends_with("_sapling"))
                })
        {
            // Stable: keep the corridor cells first and the manifest's own order.
            cells[first..=index].rotate_right(1);
            first += 1;
        }
    }
    Some(Preparation { cells, corridor })
}

pub(crate) fn footing(
    state: &TickState,
    settlement: super::SettlementId,
    at: [i32; 3],
) -> Option<crate::boundary::BlockPos> {
    let record = state.settlements.get(settlement)?;
    let lot = super::construction::preparation::active_lot(state, record);
    // The active project's own floor is the level the fill plan builds toward.
    let centre = super::construction::preparation::active_floor(record);
    let y = super::construction::surface_y_for_lot(
        state,
        centre,
        at[0],
        at[2],
        lot,
        &mut Clearance::prepared(),
    )?;
    Some(crate::boundary::BlockPos {
        x: at[0],
        y: y + 1,
        z: at[2],
    })
}

#[cfg(test)]
#[path = "forestry_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "forestry_native_tree_tests.rs"]
pub(crate) mod native_tree_tests;

#[cfg(test)]
#[path = "forestry_followup_tests.rs"]
mod followup_tests;

#[cfg(test)]
pub(crate) fn proof_builds() -> usize {
    tree_proof::BUILDS.with(std::cell::Cell::get)
}
