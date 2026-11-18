//! Village paths and farmland a needed lot covers (owner D10, 2026-11-17):
//! a path that carries the road network is re-laid around the lot before its
//! old cells are retired, farmland becomes the soil it is made of, and a lot
//! crop credits its vanilla drops to storage.
//!
//! The road network is read from the world, never stored: its nodes are the
//! plan's road, plaza and verge columns plus every walkable dirt-path column,
//! joined on foot (a step of at most one block). A lot may retire only the
//! path cells whose loss keeps every remaining node connected to the bell and
//! every door on the street it faces; a cut piece is bridged by a detour of
//! new path cells outside the lot and its one-block clearance. The detour is
//! laid before any old path cell is retired, so the walk stays connected at
//! every executor tick.
use std::collections::{HashMap, HashSet, VecDeque};

use mc_script::ScriptWorldBounds;

use crate::plugin_structures::settlement_builds::PlannedCell;
use crate::settlements::SettlementRecord;
use crate::settlements::construction::site_refusals::Reason;
use crate::settlements::construction::{self, SITE_BAND};
use crate::settlements::layout::{self, Cell, Column, PLAZA};
use crate::tick::TickState;

/// Highest detour level above the column's natural ground: one paid fill cell
/// under the path, so a two-block dip can still be crossed.
const DETOUR_RAISE: i32 = 2;
/// Blocks whose removal credits the field's harvest, as a villager reaps them.
const CROPS: [&str; 6] = [
    "wheat",
    "carrots",
    "potatoes",
    "beetroots",
    "torchflower_crop",
    "pitcher_crop",
];
/// Surfaces a detour turns into a dirt path, exactly as street paving does.
const SOIL: [&str; 6] = [
    "grass_block",
    "dirt",
    "coarse_dirt",
    "podzol",
    "mycelium",
    "rooted_dirt",
];

/// Re-laying work for one lot, in executor order.
pub(super) struct Relayout {
    /// New path cells with the paid fill under each, in walk order.
    pub(super) detour: Vec<PlannedCell>,
    /// Lot crops: removal cells whose vanilla loot the clear stage credits.
    pub(super) cleared: Vec<PlannedCell>,
    /// Path and farmland cells replaced with the soil they are made of, after
    /// every detour cell stands.
    pub(super) retired: Vec<PlannedCell>,
    /// Accepting lot floor by column `(x, z)`.
    floor: HashSet<(i32, i32)>,
    /// Ground height by column, for a lot column whose crop hides the surface
    /// from the ordinary survey.
    ground: HashMap<(i32, i32), i32>,
    /// Extent of the detour, for the executor reservation check.
    area: Option<ScriptWorldBounds>,
}

impl Relayout {
    /// The plan owns this column's lot floor block.
    pub(super) fn floor(&self, x: i32, z: i32) -> bool {
        self.floor.contains(&(x, z))
    }

    /// The plan removes this exact cell above the lot floor.
    pub(super) fn clears(&self, at: [i32; 3]) -> bool {
        self.cleared.iter().any(|cell| cell.at == at)
    }

    /// The plan retires this exact path or farmland cell before the building.
    pub(super) fn retires(&self, at: [i32; 3]) -> bool {
        self.retired.iter().any(|cell| cell.at == at)
    }

    /// Ground height where the ordinary surface survey saw a crop instead.
    pub(super) fn height(&self, x: i32, z: i32) -> Option<i32> {
        self.ground.get(&(x, z)).copied()
    }

    /// The detour's occupied extent, if a lot needed one.
    pub(super) fn area(&self) -> Option<ScriptWorldBounds> {
        self.area
    }
}

/// Plan the village work a needed lot owes.
///
/// `Ok(None)`: the lot covers no village path, farmland or crop.
/// `Err(reason)`: a named refusal; the lot is not admitted.
pub(super) fn plan(
    state: &TickState,
    record: &SettlementRecord,
    lot: ScriptWorldBounds,
) -> Result<Option<Relayout>, Reason> {
    let mut paths = Vec::new();
    let mut worked = Vec::new();
    let mut cleared = Vec::new();
    for x in lot.min[0]..=lot.max[0] {
        for z in lot.min[2]..=lot.max[2] {
            let Some((ground, block)) = ground_at(state, record.anchor[1], x, z) else {
                continue;
            };
            let path = name(state, block).is_some_and(|name| name == "dirt_path");
            let farmland = state.registry.farming.is_farmland(block);
            if path || farmland {
                // Never touch a player-placed block: the lot is not admitted
                // over a cell the player owns.
                if state.block_log.player_touched([x, ground, z]) {
                    return Err(Reason::ProtectedObstruction);
                }
                worked.push((x, ground, z, path));
                if path {
                    paths.push([x, ground, z]);
                }
            }
            for above in ground + 1..=ground + 4 {
                let Some(block) = state.world.physics_block_at(x, above, z) else {
                    break;
                };
                if !name(state, block).is_some_and(|name| CROPS.contains(&name)) {
                    continue;
                }
                if state.block_log.player_touched([x, above, z]) {
                    return Err(Reason::ProtectedObstruction);
                }
                cleared.push(PlannedCell {
                    at: [x, above, z],
                    state: state.registry.states.air,
                    resource: None,
                    quantity: 0,
                });
            }
        }
    }
    if worked.is_empty() && cleared.is_empty() {
        return Ok(None);
    }
    // Farmland inside another building's certificate is not this lot's to
    // remove; the one-block building buffer already refuses that lot, so this
    // names the cause instead of silently retiring a field in use.
    if worked
        .iter()
        .any(|&(x, _, z, _)| inside_certificate(record, x, z))
    {
        return Err(Reason::BuildingBuffer);
    }
    let detour = if paths.is_empty() {
        Salient::default()
    } else {
        reconnection(state, record, lot, &paths)?
    };
    let mut floor = HashSet::default();
    let mut ground_by_column = HashMap::default();
    let mut retired = Vec::new();
    for (x, ground, z, _) in worked {
        floor.insert((x, z));
        if cleared
            .iter()
            .any(|cell| cell.at[0] == x && cell.at[2] == z)
        {
            ground_by_column.insert((x, z), ground);
        }
        retired.push(PlannedCell {
            at: [x, ground, z],
            // A dirt path and farmland are dirt: digging either yields the
            // soil already in place, so the floor keeps it at no material cost.
            state: state.registry.states.dirt,
            resource: None,
            quantity: 0,
        });
    }
    Ok(Some(Relayout {
        detour: detour.cells,
        cleared,
        retired,
        floor,
        ground: ground_by_column,
        area: detour.area,
    }))
}

/// A lot retires a path or a field into the soil already in place: that soil
/// cell is village work on the lot, not a fabricated material cell, so the
/// canonical ledger admits it at no cost. Only a lot column qualifies.
pub(crate) fn free_soil(
    state: &TickState,
    settlement: Option<crate::settlements::SettlementId>,
    at: [i32; 3],
    block: u32,
) -> bool {
    let Some(record) = settlement.and_then(|id| state.settlements.get(id)) else {
        return false;
    };
    name(state, block).is_some_and(|name| name == "dirt")
        && matches!(layout::cell(record.anchor, at[0], at[2]), Cell::Lot)
}

/// Vanilla harvest of a lot crop the clear stage removes: the field is gone
/// with the lot, so its yield is settlement storage stock, exactly as the
/// settlement's own farmers reap one (`villager::harvest`): a ripe plant gives
/// its produce and one seed, an unripe one gives the seed back.
pub(crate) fn crop_drops(
    state: &TickState,
    at: [i32; 3],
    block: u32,
) -> Result<Vec<mc_data::ItemStack>, mc_script::ScriptOperationFailure> {
    let Some(path) = name(state, block) else {
        return Ok(Vec::new());
    };
    let Some((age, maximum)) = crate::farming::age_of(&state.registry.blocks, block) else {
        return Ok(Vec::new());
    };
    let ripe = age == maximum;
    let (produce, seed) = match path {
        "wheat" => (Some(("wheat", 1)), Some("wheat_seeds")),
        "beetroots" => (ripe.then_some(("beetroot", 1)), Some("beetroot_seeds")),
        "carrots" => (Some(("carrot", if ripe { 3 } else { 1 })), None),
        "potatoes" => (Some(("potato", if ripe { 3 } else { 1 })), None),
        "torchflower_crop" => (
            ripe.then_some(("torchflower", 1)),
            (!ripe).then_some("torchflower_seeds"),
        ),
        "pitcher_crop" => (
            ripe.then_some(("pitcher_plant", 1)),
            (!ripe).then_some("pitcher_pod"),
        ),
        _ => return Ok(Vec::new()),
    };
    let mut stacks = Vec::new();
    if let Some((item, count)) = produce
        && let Some(item) = item_id(state, item)
    {
        stacks.push(mc_data::ItemStack::new(item, count));
    }
    if let Some(seed) = seed
        && let Some(item) = item_id(state, seed)
    {
        stacks.push(mc_data::ItemStack::new(item, 1));
    }
    let _ = at;
    Ok(stacks)
}

fn item_id(state: &TickState, name: &str) -> Option<u32> {
    state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse(format!("minecraft:{name}")).ok()?)
}

fn name(state: &TickState, block: u32) -> Option<&str> {
    state
        .registry
        .blocks
        .by_id(mc_world::BlockStateId(block))
        .map(|definition| definition.block.id.path())
}

/// The highest solid, non-tree ground of one column inside the survey band,
/// with a crop allowed above it (a cropped field still has its soil).
fn ground_at(state: &TickState, anchor_y: i32, x: i32, z: i32) -> Option<(i32, u32)> {
    (anchor_y - SITE_BAND..=anchor_y + SITE_BAND + 3)
        .rev()
        .find_map(|y| {
            let block = state.world.physics_block_at(x, y, z)?;
            let definition = state.registry.blocks.by_id(mc_world::BlockStateId(block))?;
            if definition.has_fluid()
                || crate::settlements::forestry::tree(state, block)
                || !state
                    .registry
                    .collision_boxes
                    .get(block as usize)
                    .is_some_and(|boxes| !boxes.is_empty())
            {
                return None;
            }
            Some((y, block))
        })
}

fn inside_certificate(record: &SettlementRecord, x: i32, z: i32) -> bool {
    record.buildings.values().any(|building| {
        (building.bounds.min[0]..=building.bounds.max[0]).contains(&x)
            && (building.bounds.min[2]..=building.bounds.max[2]).contains(&z)
    })
}

fn vacant(state: &TickState, block: u32) -> bool {
    name(state, block).is_some_and(|name| matches!(name, "air" | "cave_air" | "void_air"))
}

fn soil(state: &TickState, block: u32) -> bool {
    name(state, block).is_some_and(|name| SOIL.contains(&name))
}

/// The walk a lot's detour must keep: plan road, plaza and verge columns plus
/// every dirt-path column, joined on foot.
type Nodes = HashMap<(i32, i32), i32>;

#[derive(Default)]
struct Salient {
    cells: Vec<PlannedCell>,
    area: Option<ScriptWorldBounds>,
}

/// Bridge every piece the lot cuts off the bell with new path cells, then
/// prove the whole network and every door still walk to it.
fn reconnection(
    state: &TickState,
    record: &SettlementRecord,
    lot: ScriptWorldBounds,
    paths: &[[i32; 3]],
) -> Result<Salient, Reason> {
    let nodes = walk(state, record);
    let seeds = bell_columns(record, &nodes);
    if seeds.is_empty() {
        // No walk starts from the bell: nothing can be proved about the
        // network, so a lot may not cut it.
        return Err(Reason::NoPathDetour);
    }
    let removed: HashSet<(i32, i32)> = paths.iter().map(|at| (at[0], at[2])).collect();
    let before = within(&nodes, &seeds);
    let mut passable: Nodes = nodes
        .iter()
        .filter(|(column, _)| !removed.contains(column))
        .map(|(column, &y)| (*column, y))
        .collect();
    let mut reach = within(&passable, &seeds);
    let mut cut: Vec<(i32, i32)> = before
        .iter()
        .filter(|column| !removed.contains(column) && !reach.contains(column))
        .copied()
        .collect();
    cut.sort_unstable();
    let mut plan = Salient::default();
    let mut laid: HashSet<[i32; 3]> = HashSet::default();
    while !cut.is_empty() {
        let start = cut[0];
        let (bridged, cells) =
            bridge(state, record, lot, &passable, &reach, start).ok_or(Reason::NoPathDetour)?;
        plan.cells
            .extend(cells.into_iter().filter(|cell| laid.insert(cell.at)));
        passable.extend(bridged);
        reach = within(&passable, &seeds);
        let held = reach.clone();
        cut.retain(|column| !held.contains(column));
    }
    // Prove the network before admission: every node the lot does not retire
    // still walks to the bell, and every door still faces such a street.
    if before
        .iter()
        .any(|column| !removed.contains(column) && !reach.contains(column))
    {
        return Err(Reason::NoPathDetour);
    }
    let doors = crate::settlements::layout::Network::walk(state, record);
    for building in record.buildings.values() {
        let Some(at) = doors.access_column(building) else {
            continue;
        };
        let column = (at[0], at[2]);
        if before.contains(&column) && !reach.contains(&column) {
            return Err(Reason::NoPathDetour);
        }
    }
    if let Some(first) = plan.cells.first() {
        let mut min = first.at;
        let mut max = first.at;
        for cell in &plan.cells {
            for axis in 0..3 {
                min[axis] = min[axis].min(cell.at[axis]);
                max[axis] = max[axis].max(cell.at[axis]);
            }
        }
        plan.area = ScriptWorldBounds::new(min, max).ok();
    }
    Ok(plan)
}

/// Shortest chain of new path cells from the bell's walk to `start`, honouring
/// the walk's one-block step and the detour's fill rule.
fn bridge(
    state: &TickState,
    record: &SettlementRecord,
    lot: ScriptWorldBounds,
    passable: &Nodes,
    reach: &HashSet<(i32, i32)>,
    start: (i32, i32),
) -> Option<(Nodes, Vec<PlannedCell>)> {
    let mut columns: HashMap<(i32, i32), Option<(i32, u32)>> = HashMap::default();
    let mut seen: HashSet<(i32, i32, i32)> = HashSet::default();
    let mut parent: HashMap<(i32, i32, i32), (i32, i32, i32)> = HashMap::default();
    let mut queue: VecDeque<(i32, i32, i32)> = VecDeque::new();
    for &column in reach {
        if let Some(&y) = passable.get(&column)
            && seen.insert((column.0, column.1, y))
        {
            queue.push_back((column.0, column.1, y));
        }
    }
    let mut arrived = None;
    while let Some((x, z, y)) = queue.pop_front() {
        if (x, z) == start {
            arrived = Some((x, z, y));
            break;
        }
        for [dx, dz] in [[1, 0], [-1, 0], [0, 1], [0, -1]] {
            let next = (x + dx, z + dz);
            let levels: Vec<i32> = match passable.get(&next) {
                Some(&ny) => vec![ny],
                None => {
                    let ground = *columns
                        .entry(next)
                        .or_insert_with(|| candidate_ground(state, record, lot, next));
                    (0..=DETOUR_RAISE)
                        .filter_map(|raise| ground.map(|(natural, _)| natural + raise))
                        .filter(|&level| path_ok(state, ground, next, level))
                        .collect()
                }
            };
            for level in levels {
                if (level - y).abs() > 1 || !seen.insert((next.0, next.1, level)) {
                    continue;
                }
                parent.insert((next.0, next.1, level), (x, z, y));
                queue.push_back((next.0, next.1, level));
            }
        }
    }
    let end = arrived?;
    let mut chain = Vec::new();
    let mut at = end;
    while let Some(&previous) = parent.get(&at) {
        chain.push(at);
        at = previous;
    }
    chain.reverse();
    let mut cells = Vec::new();
    let mut nodes = Nodes::default();
    for (x, z, level) in chain {
        // A chain may pass through street the walk already has: it needs no
        // new cell, only its own level in the graph.
        if passable.contains_key(&(x, z)) {
            nodes.insert((x, z), level);
            continue;
        }
        let natural = columns
            .get(&(x, z))
            .and_then(|ground| ground.map(|(natural, _)| natural))
            .unwrap_or(level);
        for under in natural + 1..level {
            // Fill under a raised detour cell is real storage dirt.
            cells.push(PlannedCell {
                at: [x, under, z],
                state: state.registry.states.dirt,
                resource: Some("minecraft:dirt".into()),
                quantity: 1,
            });
        }
        if let Some(path) = dirt_path(state) {
            cells.push(PlannedCell {
                at: [x, level, z],
                state: path,
                resource: None,
                quantity: 0,
            });
        }
        nodes.insert((x, z), level);
    }
    Some((nodes, cells))
}

/// A detour column's natural ground, if the column may carry a path at all:
/// outside the lot and its one-block clearance, inside the territory, off a
/// wall column and off a certified building.
fn candidate_ground(
    state: &TickState,
    record: &SettlementRecord,
    lot: ScriptWorldBounds,
    (x, z): (i32, i32),
) -> Option<(i32, u32)> {
    if lot.min[0] - 1 <= x && x <= lot.max[0] + 1 && lot.min[2] - 1 <= z && z <= lot.max[2] + 1 {
        return None;
    }
    let (dx, dz) = (
        i64::from(x - record.anchor[0]),
        i64::from(z - record.anchor[2]),
    );
    if dx * dx + dz * dz > i64::from(record.radius) * i64::from(record.radius) {
        return None;
    }
    if matches!(layout::cell(record.anchor, x, z), Cell::Wall)
        || record
            .buildings
            .values()
            .any(|building| inside(building.bounds, x, z))
    {
        return None;
    }
    ground_at(state, record.anchor[1], x, z)
}

fn inside(bounds: ScriptWorldBounds, x: i32, z: i32) -> bool {
    (bounds.min[0]..=bounds.max[0]).contains(&x) && (bounds.min[2]..=bounds.max[2]).contains(&z)
}

/// Whether a detour path may stand at `(x, z)` on `level`: it paves natural
/// soil, or sits on a paid fill of vacant cells, with two clear blocks above.
fn path_ok(state: &TickState, ground: Option<(i32, u32)>, (x, z): (i32, i32), level: i32) -> bool {
    let Some((natural, block)) = ground else {
        return false;
    };
    if level < natural || level > natural + DETOUR_RAISE {
        return false;
    }
    if level == natural {
        if !soil(state, block) {
            return false;
        }
    } else {
        // The path cell itself is vacant, and every gap down to the ground is
        // filled with storage dirt: a raised path never stands over a gap.
        for under in natural + 1..=level {
            let empty = state
                .world
                .physics_block_at(x, under, z)
                .is_some_and(|block| vacant(state, block));
            if !empty || state.block_log.player_touched([x, under, z]) {
                return false;
            }
        }
    }
    [level + 1, level + 2].into_iter().all(|above| {
        state
            .world
            .physics_block_at(x, above, z)
            .is_some_and(|block| vacant(state, block) || construction::clearable(state, block))
    }) && !state.block_log.player_touched([x, level, z])
        && !state.block_entities.contains_key(&(x, level, z))
        && !state.block_entities.contains_key(&(x, level + 1, z))
}

fn walk(state: &TickState, record: &SettlementRecord) -> Nodes {
    let anchor = record.anchor;
    let reach = i32::try_from(record.radius)
        .unwrap_or(i32::MAX)
        .max(layout::RING)
        + 1;
    let mut nodes = Nodes::default();
    for dx in -reach..=reach {
        for dz in -reach..=reach {
            let (x, z) = (anchor[0] + dx, anchor[2] + dz);
            let Column::Ground { y, .. } = layout::sample(state, record, x, z, false) else {
                continue;
            };
            let plan = layout::cell(anchor, x, z);
            let paved = state
                .world
                .physics_block_at(x, y, z)
                .and_then(|block| state.registry.blocks.by_id(mc_world::BlockStateId(block)))
                .is_some_and(|definition| definition.block.id.path() == "dirt_path");
            if paved || matches!(plan, Cell::Road | Cell::Plaza | Cell::Verge) {
                nodes.insert((x, z), y);
            }
        }
    }
    nodes
}

fn bell_columns(record: &SettlementRecord, nodes: &Nodes) -> Vec<(i32, i32)> {
    let [ax, _, az] = record.anchor;
    let mut seeds = Vec::new();
    for ring in 0..=PLAZA {
        for dx in -ring..=ring {
            for dz in -ring..=ring {
                if dx.abs().max(dz.abs()) == ring && nodes.contains_key(&(ax + dx, az + dz)) {
                    seeds.push((ax + dx, az + dz));
                }
            }
        }
    }
    seeds
}

/// Nodes reachable on foot from `seeds`.
fn within(nodes: &Nodes, seeds: &[(i32, i32)]) -> HashSet<(i32, i32)> {
    let mut seen: HashSet<(i32, i32)> = HashSet::default();
    let mut queue: VecDeque<(i32, i32)> = VecDeque::new();
    for &seed in seeds {
        if nodes.contains_key(&seed) && seen.insert(seed) {
            queue.push_back(seed);
        }
    }
    while let Some(column) = queue.pop_front() {
        let y = nodes[&column];
        for [dx, dz] in [[1, 0], [-1, 0], [0, 1], [0, -1]] {
            let next = (column.0 + dx, column.1 + dz);
            if seen.contains(&next) {
                continue;
            }
            if let Some(&ny) = nodes.get(&next)
                && (ny - y).abs() <= 1
            {
                seen.insert(next);
                queue.push_back(next);
            }
        }
    }
    seen
}

fn dirt_path(state: &TickState) -> Option<u32> {
    let id = mc_data::Identifier::parse("minecraft:dirt_path").ok()?;
    Some(state.registry.blocks.block(&id)?.default.0)
}

#[cfg(test)]
#[path = "relayout_tests.rs"]
mod tests;
