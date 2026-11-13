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
            if dx > 4 || dz > 4 || at[1] < root[1] || at[1] - root[1] > 64 {
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

/// Retire vanished roots only when their whole possible trunk is loaded.
pub(super) fn retire_missing(state: &mut TickState) {
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

pub(crate) fn clearable(state: &TickState, at: [i32; 3]) -> bool {
    let Some(block) = state.world.physics_block_at(at[0], at[1], at[2]) else {
        return false;
    };
    if !tree(state, block) || state.block_log.player_touched(at) || planted(state, at) {
        return false;
    }
    // With capture disabled, a nearby live player is an additional veto. The
    // deterministic baseline still protects all nonmatching historical edits.
    if !state.block_log.enabled()
        && state.players.keys().any(|id| {
            state.entities.slot_of(*id).is_some_and(|slot| {
                let pos = state.entities.pos(slot);
                (pos[0] - f64::from(at[0])).abs() <= 32.0
                    && (pos[2] - f64::from(at[2])).abs() <= 32.0
            })
        })
    {
        return false;
    }
    let generated = state
        .settlements
        .forest
        .lock()
        .expect("forest cache poisoned")
        .block(state, at);
    let Some(generated) = generated else {
        return false;
    };
    if generated == block {
        return true;
    }
    let Some(actual) = state.registry.blocks.by_id(BlockStateId(block)) else {
        return false;
    };
    let Some(baseline) = state.registry.blocks.by_id(BlockStateId(generated)) else {
        return false;
    };
    actual.block.id == baseline.block.id
        && actual.block.id.path().ends_with("_leaves")
        && actual.properties.len() == baseline.properties.len()
        && actual.properties.iter().all(|(key, value)| {
            key == "distance"
                || baseline
                    .properties
                    .iter()
                    .any(|(other, expected)| other == key && expected == value)
        })
}

pub(crate) fn drops(
    state: &TickState,
    at: [i32; 3],
    block: u32,
) -> Result<Vec<mc_data::ItemStack>, mc_script::ScriptOperationFailure> {
    if !clearable(state, at) {
        return Err(mc_script::ScriptOperationFailure::Blocked);
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

fn roll(
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

/// Clear the lot and its walking corridor in physical work order. Only tree
/// blocks are added here; unrelated terrain keeps the planner's old refusals.
pub(crate) fn cells(
    state: &TickState,
    project: &super::construction::Project,
    size: [u8; 3],
) -> Option<Vec<crate::plugin_structures::settlement_builds::PlannedCell>> {
    use crate::plugin_structures::settlement_builds::PlannedCell;
    let record = state.settlements.get(project.settlement)?;
    let mut lot = Vec::new();
    let mut lot_has_tree = false;
    for x in project.origin[0]..project.origin[0] + i32::from(size[0]) {
        for z in project.origin[2]..project.origin[2] + i32::from(size[2]) {
            let y = super::construction::surface_y(state, record.anchor[1], x, z)?;
            lot_has_tree |= (y + 1..project.origin[1] + i32::from(size[1])).any(|above| {
                state
                    .world
                    .physics_block_at(x, above, z)
                    .is_some_and(|block| tree(state, block))
            });
            lot.push([x, y, z]);
        }
    }
    let route =
        super::layout::Network::for_construction(state, record).construction_access(project);
    let mut columns = match route {
        Some(route) => route,
        None if !lot_has_tree => return Some(Vec::new()),
        None => return None,
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
            if !tree(state, block) {
                continue;
            }
            if !clearable(state, [x, y, z]) {
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
    Some(cells)
}

pub(crate) fn footing(
    state: &TickState,
    settlement: super::SettlementId,
    at: [i32; 3],
) -> Option<crate::boundary::BlockPos> {
    let record = state.settlements.get(settlement)?;
    let y = super::construction::surface_y(state, record.anchor[1], at[0], at[2])?;
    Some(crate::boundary::BlockPos {
        x: at[0],
        y: y + 1,
        z: at[2],
    })
}

#[cfg(test)]
#[path = "forestry_tests.rs"]
pub(crate) mod tests;
