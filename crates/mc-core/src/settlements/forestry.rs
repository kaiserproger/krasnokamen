//! Funded site preparation: immutable worldgen evidence, never admission writes.
use std::collections::VecDeque;
use std::sync::mpsc;

use mc_world::{BlockStateId, Chunk, ChunkPos};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

use crate::tick::TickState;

// Covers the largest legal settlement, with room for boundary chunks. Old
// towns may evict immutable evidence; they can request it again when evaluated.
const CACHE_CHUNKS: usize = 1_156;
const QUEUED_CHUNKS: usize = 32;

type Generated = (ChunkPos, Chunk, u128);

#[derive(Default)]
pub(crate) struct Cache {
    ready: HashMap<ChunkPos, Chunk>,
    order: VecDeque<ChunkPos>,
    pending: HashSet<ChunkPos>,
    worker: Option<(mpsc::SyncSender<ChunkPos>, mpsc::Receiver<Generated>)>,
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

impl Cache {
    fn block(&mut self, state: &TickState, at: [i32; 3]) -> Option<u32> {
        if !*self.trusted.get_or_insert_with(|| generator_known(state)) {
            return None;
        }
        if self.worker.is_none() {
            let generator = state.world.generator();
            let (send, requests) = mpsc::sync_channel::<ChunkPos>(QUEUED_CHUNKS);
            let (results, receive) = mpsc::sync_channel(QUEUED_CHUNKS);
            let spawn = std::thread::Builder::new()
                .name("settlement-forest".into())
                .spawn(move || {
                    while let Ok(pos) = requests.recv() {
                        let started = std::time::Instant::now();
                        let chunk = generator.generate(pos);
                        if results
                            .send((pos, chunk, started.elapsed().as_micros()))
                            .is_err()
                        {
                            break;
                        }
                    }
                });
            if let Err(error) = spawn {
                tracing::error!(%error, "settlement forest evidence worker could not start");
                self.trusted = Some(false);
                return None;
            }
            self.worker = Some((send, receive));
        }
        let (send, receive) = self.worker.as_ref().expect("forest worker");
        while let Ok((pos, chunk, micros)) = receive.try_recv() {
            self.pending.remove(&pos);
            if self.ready.len() == CACHE_CHUNKS
                && let Some(oldest) = self.order.pop_front()
            {
                self.ready.remove(&oldest);
            }
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
            return chunk
                .get_block(
                    at[0].rem_euclid(16) as u8,
                    at[1],
                    at[2].rem_euclid(16) as u8,
                )
                .map(|id| id.0);
        }
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

/// Startup already rejects a native generation-contract mismatch. Imported
/// Anvil templates have no authenticated Solaris generator identity, so they
/// cannot authorize clearing, even when individual cells happen to match.
fn generator_known(state: &TickState) -> bool {
    let Some(record) = state.native_globals.get("level_metadata") else {
        return state.world.chunk_source().native.is_none();
    };
    matches!(crate::native_checkpoint::decode_tag(record, "level_metadata"),
        Ok(mc_nbt::Tag::Compound(fields)) if fields.is_empty())
}

pub(crate) fn tree(state: &TickState, block: u32) -> bool {
    state
        .registry
        .blocks
        .by_id(BlockStateId(block))
        .is_some_and(|definition| {
            let name = definition.block.id.path();
            name.ends_with("_leaves") || (name.ends_with("_log") && !name.starts_with("stripped_"))
        })
}

/// Lumberjack roots are durable. The conservative crown envelope includes
/// branched and giant grown trees, not just the original sapling column.
fn planted(state: &TickState, at: [i32; 3]) -> bool {
    state.settlements.records().any(|record| {
        record.planted_trees.iter().any(|root| {
            at[0].abs_diff(root[0]) <= 8
                && at[2].abs_diff(root[2]) <= 8
                && (0..=384).contains(&(i64::from(at[1]) - i64::from(root[1])))
        })
    })
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
    state
        .settlements
        .forest
        .lock()
        .expect("forest cache poisoned")
        .block(state, at)
        == Some(block)
}

pub(crate) fn drops(
    state: &TickState,
    at: [i32; 3],
    block: u32,
) -> Result<Vec<mc_data::ItemStack>, mc_script::ScriptOperationFailure> {
    if !clearable(state, at) {
        return Err(mc_script::ScriptOperationFailure::Blocked);
    }
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
