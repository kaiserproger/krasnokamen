//! D10 liquid lot preparation: water and lava under a needed lot's authored
//! floor are filled with real blocks from a safe stand. Admission is a closed
//! bowl — the footprint's one-cell ring is solid up to the floor — so the flow
//! a filled source leaves behind cannot leave the work volume and no reach
//! model is needed. Execution gates only lava: a flowing or falling lava cell is
//! never written, it is waited out until the engine drains it to air.
use super::*;
use crate::fluids::Fluid;

/// Fluid work of one candidate lot: how many fill cells currently hold fluid.
#[derive(Debug, Default)]
pub(crate) struct Liquids {
    pub(crate) water: u32,
    pub(crate) lava: u32,
    /// Highest fluid cell of the work volume. The closed bowl must hold fluid
    /// back from the deepest support up to here: a fluid can descend and then
    /// leave at a lower level, never rise above its own top cell.
    pub(crate) top: Option<i32>,
    /// A non-fluid cell that holds a water source (waterlogged block, bubble
    /// column, `fluids::contained`). It cannot be cut off, so it refuses on its
    /// own and mixes with lava the same way a water cell does.
    pub(crate) preserved_water: bool,
}

impl Liquids {
    pub(crate) fn total(&self) -> u32 {
        self.water.saturating_add(self.lava)
    }
}

/// One lot's bounded fluid work. **Invariant:** a lot footprint is at most
/// 11×11 columns and the fill below the floor is at most `MAX_FILL_DEPTH` rows,
/// so a real lot never exceeds 11·11·(4+1) = 605 fluid cells; the 1024 cap is a
/// cheap independent guard, never reachable through `check`. It is kept so a
/// future wider window cannot make liquid work unbounded, and the named refusal
/// stays testable on the struct itself.
pub(crate) const MAX_LIQUID_CELLS: usize = 1024;

/// A block that *contains* fluid, including a waterlogged block: the shell and
/// solidity tests must treat those as wet (B-min's `shell_fluid` semantics).
pub(crate) fn has_fluid(state: &TickState, block: u32) -> bool {
    state
        .registry
        .blocks
        .by_id(mc_world::BlockStateId(block))
        .is_some_and(|definition| definition.has_fluid())
}

/// An actual fluid block (water/lava state, any level). A waterlogged block or
/// a bubble column is `contained` but not a fill target: it is a solid the plan
/// may not overwrite.
pub(crate) fn fluid_block(state: &TickState, block: u32) -> bool {
    crate::fluids::fluid_at(&state.registry.states, block).is_some()
}

/// A fluid block state and its vanilla level, or a contained water source.
pub(crate) fn contained(state: &TickState, block: u32) -> Option<(Fluid, u8)> {
    crate::fluids::contained(state, block)
}

/// A solid cell that holds fluid back: not air, not replaceable, not a fluid and
/// not a tree. A tree in the ring is not a wall: the same plan fells a
/// registered tree whole in its clear stage, so counting it would declare a
/// closed bowl that opens itself.
pub(crate) fn solid(state: &TickState, block: u32) -> bool {
    !has_fluid(state, block)
        && !clearable(state, block)
        && !crate::settlements::forestry::tree_or_sapling(state, block)
        && state
            .registry
            .collision_boxes
            .get(block as usize)
            .is_some_and(|boxes| !boxes.is_empty())
}

/// A block vanilla fire can ignite or burn. Fire replaces a flammable block, so
/// a flammable cell is not a retaining shell for lava.
pub(crate) fn combustible(state: &TickState, block: u32) -> bool {
    state
        .registry
        .blocks
        .by_id(mc_world::BlockStateId(block))
        .is_some_and(|definition| {
            mc_data::fire_flammability::odds(definition.block.id.as_str()) != (0, 0)
        })
}

/// Ground discovery for a lot column: the highest solid support whose cell
/// above is air, clearable or fluid. It is the lot path of
/// `surface_y_for_lot` (which tolerates fluid only for `Some(lot)`), kept as a
/// named entry so site checks and plan materialization cannot drift apart;
/// roads, frontage reach and layout keep the fluid-refusing `None` path.
pub(crate) fn ground_y_for_lot(
    state: &TickState,
    anchor_y: i32,
    x: i32,
    z: i32,
    lot: Option<ScriptWorldBounds>,
    clearance: &mut crate::settlements::forestry::Clearance,
) -> Option<i32> {
    surface_y_for_lot(state, anchor_y, x, z, lot, clearance)
}

/// Collect the fluid cells of one fill column and apply every per-cell veto.
/// `height` is the discovered support, `floor` the authored floor. Dry fill
/// cells are counted here so the caller never walks the column twice.
pub(crate) fn scan_column(
    state: &TickState,
    liquids: &mut Liquids,
    cost: &mut site_refusals::Cost,
    x: i32,
    z: i32,
    height: i32,
    floor: i32,
    authored_floor: bool,
) -> Result<(), site_refusals::Reason> {
    use site_refusals::Reason;
    for y in height + 1..=floor {
        let at = [x, y, z];
        let block = state
            .world
            .physics_block_at(x, y, z)
            .ok_or(Reason::Unloaded)?;
        // The shared player veto covers every non-air cell, including fluid: a
        // player-attributed water or lava cell is never overwritten by a fill.
        if block != state.registry.states.air && state.block_log.player_touched(at) {
            return Err(Reason::ProtectedObstruction);
        }
        // A fluid at the authored floor row is above the fill work and stays a
        // named refusal; the authored block itself rewrites that row.
        if y == floor && authored_floor {
            if contained(state, block).is_some() {
                return Err(Reason::LiquidInWork);
            }
            continue;
        }
        if fluid_block(state, block) {
            match contained(state, block) {
                Some((Fluid::Lava, _)) => liquids.lava += 1,
                _ => liquids.water += 1,
            }
            liquids.top = Some(liquids.top.map_or(y, |top| top.max(y)));
            continue;
        }
        // A waterlogged block or a bubble column keeps a water source that no
        // fill can cut off; it refuses on its own and counts as mixed with lava.
        if contained(state, block).is_some() {
            liquids.preserved_water = true;
            continue;
        }
        // The floor cell is a fill either way; the caller already admitted it.
        // A non-authored floor row may hold an actual fluid block (handled
        // above) or an ordinary replaceable/solid support cell.
        if y == floor {
            cost.fill += 1;
            continue;
        }
        if !clearable(state, block) {
            return Err(Reason::CutBeyondEnvelope);
        }
        cost.fill += 1;
    }
    Ok(())
}

/// Whole-lot checks after every column was scanned: the bound, mixed liquids
/// and the closed bowl. `lowest` is the deepest discovered support; the bowl
/// reaches the highest fluid cell because a fluid can descend and leave at a
/// lower level but never rises above its own top.
pub(crate) fn finish(
    state: &TickState,
    liquids: &Liquids,
    origin: [i32; 3],
    size: [u8; 3],
    lowest: i32,
    columns: &[([i32; 2], i32)],
) -> Result<(), site_refusals::Reason> {
    use site_refusals::Reason;
    if liquids.total() > MAX_LIQUID_CELLS as u32 {
        return Err(Reason::LiquidBeyondBound);
    }
    let lava = liquids.lava > 0;
    let water = liquids.water > 0 || liquids.preserved_water;
    if lava && water {
        return Err(Reason::MixedLiquids);
    }
    if liquids.preserved_water {
        // A water source the fill cannot cut off would refill every cell it
        // reaches: it is a supply outside the plan, not a fill target.
        return Err(Reason::LiquidInWork);
    }
    let Some(top) = liquids.top else {
        return Ok(());
    };
    // Closed bowl: every one-cell ring cell outside the footprint, from the
    // deepest support to the highest fluid cell, must hold fluid back. Lava
    // never flows up, fills only add solid blocks, and the stand stays outside
    // the footprint (`in_reach`), so a closed ring means nothing can reach it.
    // A flammable ring is not a retaining shell for lava: fire replaces it.
    let max: [i32; 3] = std::array::from_fn(|axis| origin[axis] + i32::from(size[axis]) - 1);
    for y in lowest..=top {
        for x in origin[0] - 1..=max[0] + 1 {
            for z in origin[2] - 1..=max[2] + 1 {
                let ring =
                    x == origin[0] - 1 || x == max[0] + 1 || z == origin[2] - 1 || z == max[2] + 1;
                if !ring {
                    continue;
                }
                let block = state
                    .world
                    .physics_block_at(x, y, z)
                    .ok_or(Reason::Unloaded)?;
                if !solid(state, block) {
                    return Err(Reason::LiquidOpenShell);
                }
                if lava && combustible(state, block) {
                    return Err(Reason::CombustibleShell);
                }
            }
        }
    }
    // Interior pockets: a fluid can flow sideways under a neighbouring column's
    // higher support, descend below `lowest` and leave under the ring. Every
    // column's cells from the deepest support up to (and including) the block
    // under its own support must be solid ground.
    for &([x, z], height) in columns {
        for y in lowest..=height - 1 {
            let block = state
                .world
                .physics_block_at(x, y, z)
                .ok_or(Reason::Unloaded)?;
            if !solid(state, block) {
                return Err(Reason::LiquidOpenShell);
            }
        }
    }
    Ok(())
}

/// A lot whose own door column holds fluid cannot be entered: the builder has
/// nowhere dry to stand. This stays a named refusal (backlog, owner D21) rather
/// than a silent `LotGeometry` miss; a fluid verge or street refuses the lot the
/// same way and keeps the road network unchanged.
pub(crate) fn door_fluid(
    state: &TickState,
    front: [i32; 3],
    normal: [i32; 2],
    anchor_y: i32,
) -> bool {
    let door = [front[0] + 2 * normal[0], front[2] + 2 * normal[1]];
    (anchor_y - 1..=anchor_y + 1).any(|y| {
        state
            .world
            .physics_block_at(door[0], y, door[1])
            .is_some_and(|block| contained(state, block).is_some())
    })
}

/// Plan order inside the fill group: every source first, top-down within its
/// column, so a filled source stops the supply before any dependent cut runs;
/// the remaining fill keeps the column order it was built in.
pub(crate) fn order(state: &TickState, cells: &mut [executor::PlannedCell]) {
    let source = |cell: &executor::PlannedCell| {
        state
            .world
            .physics_block_at(cell.at[0], cell.at[1], cell.at[2])
            .is_some_and(|block| {
                fluid_block(state, block)
                    && contained(state, block).is_some_and(|(_, level)| level == 0)
            })
    };
    // A dry lot keeps its authored order and reads nothing extra.
    if !cells.iter().any(source) {
        return;
    }
    cells.sort_by_cached_key(|cell| {
        if source(cell) {
            (0, cell.at[0], cell.at[2], std::cmp::Reverse(cell.at[1]))
        } else {
            (1, 0, 0, std::cmp::Reverse(0))
        }
    });
}

/// Indices of the fill cells that are fluid sources at preparation time: the
/// cells a settlement plan puts in the stage before `clear`, so a cut next to
/// lava is never blocked by a fill that runs after it.
pub(crate) fn stage_cells(state: &TickState, cells: &[executor::PlannedCell]) -> Vec<usize> {
    cells
        .iter()
        .enumerate()
        .filter(|(_, cell)| cell.state != state.registry.states.air)
        .filter(|(_, cell)| {
            state
                .world
                .physics_block_at(cell.at[0], cell.at[1], cell.at[2])
                .is_some_and(|block| {
                    fluid_block(state, block)
                        && contained(state, block).is_some_and(|(_, level)| level == 0)
                })
        })
        .map(|(index, _)| index)
        .collect()
}

/// Why the next planned cell cannot be written yet, if it cannot.
/// `planned` is the cell's plan state (air for a clear cell).
pub(crate) fn write_wait(state: &TickState, at: [i32; 3], planned: u32) -> Option<&'static str> {
    let block = state.world.physics_block_at(at[0], at[1], at[2])?;
    if let Some((Fluid::Lava, level)) = contained(state, block)
        && level != 0
    {
        // Flowing or falling lava is never written: the fill waits for the
        // engine to drain it to air after its source was filled.
        return Some("lava_draining");
    }
    if planned == state.registry.states.air && lava_beside(state, at) {
        // Clearing beside lava frees the cell its flow would enter.
        return Some("lava_neighbour");
    }
    None
}

/// A lava cell in the six neighbours of `at`.
pub(crate) fn lava_beside(state: &TickState, at: [i32; 3]) -> bool {
    [
        [1, 0, 0],
        [-1, 0, 0],
        [0, 1, 0],
        [0, -1, 0],
        [0, 0, 1],
        [0, 0, -1],
    ]
    .into_iter()
    .any(|step| {
        let cell = [at[0] + step[0], at[1] + step[1], at[2] + step[2]];
        state
            .world
            .physics_block_at(cell[0], cell[1], cell[2])
            .and_then(|block| contained(state, block))
            .is_some_and(|(fluid, _)| fluid == Fluid::Lava)
    })
}

/// A stand cell a builder can occupy without touching lava: support, feet and
/// head plus their four horizontal neighbours are all lava-free.
pub(crate) fn stand_safe(state: &TickState, at: [i32; 3]) -> bool {
    for y in at[1] - 1..=at[1] + 1 {
        for (dx, dz) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
            let cell = [at[0] + dx, y, at[2] + dz];
            if state
                .world
                .physics_block_at(cell[0], cell[1], cell[2])
                .and_then(|block| contained(state, block))
                .is_some_and(|(fluid, _)| fluid == Fluid::Lava)
            {
                return false;
            }
        }
    }
    true
}

/// Self-heal a waiting gate: a static flow that no executor write touches has
/// no scheduled fluid tick, so it would never recede. Scheduling its
/// neighbourhood gives it one.
///
/// Call this **once when a wait starts**, never on every waiting tick: the queue
/// deduplicates only inside one tick (an equal `(pos, family, flowing,
/// deadline)`), so a per-tick call would add a fresh tick per neighbour on each
/// deadline and mark the checkpoint/journal dirty for no gain. The S3 wiring
/// therefore calls it at the transition into `lava_draining`.
pub(crate) fn wake(state: &mut TickState, at: [i32; 3]) {
    crate::fluids::schedule_around(
        state,
        crate::boundary::BlockPos {
            x: at[0],
            y: at[1],
            z: at[2],
        },
    );
}
