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
/// bubble column is `contained`, not liquid-fill work; D27 demolition instead
/// uses its ordinary block-drop authority.
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
/// `record` is the working settlement: inside its territory the player veto
/// reads no history (owner D27.4).
#[allow(clippy::too_many_arguments)]
pub(in crate::settlements::construction) fn scan_column(
    state: &TickState,
    record: &crate::settlements::SettlementRecord,
    liquids: &mut Liquids,
    cost: &mut site_refusals::Cost,
    column: [i32; 2],
    height: i32,
    floor: i32,
    authored_floor: bool,
) -> Result<(), site_refusals::Reason> {
    use site_refusals::Reason;
    let [x, z] = column;
    for y in height + 1..=floor {
        let at = [x, y, z];
        let block = state
            .world
            .physics_block_at(x, y, z)
            .ok_or(Reason::Unloaded)?;
        // The shared player veto covers every non-air cell, including fluid: a
        // player-attributed water or lava cell outside the territory is never
        // overwritten by a fill.
        if block != state.registry.states.air
            && !record.contains(at)
            && state.block_log.player_touched(at)
        {
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
pub(in crate::settlements::construction) fn finish(
    state: &TickState,
    liquids: &Liquids,
    origin: [i32; 3],
    max: [i32; 3],
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

/// The liquid proof of a plan: the discovered bottom, every fluid column's
/// support row and the cells that held fluid when the bowl was certified. Paid
/// cells stay in `cells`: the proof is the volume the admission certified, not
/// the unpaid tail, so a resume reads the same envelope it was admitted with.
pub(crate) fn proof(
    state: &TickState,
    cells: &[executor::PlannedCell],
    floor: i32,
) -> Option<crate::plugin_structures::LiquidProof> {
    let mut fluid = Vec::new();
    for cell in cells {
        if cell.state == state.registry.states.air {
            continue;
        }
        if state
            .world
            .physics_block_at(cell.at[0], cell.at[1], cell.at[2])
            .is_some_and(|block| fluid_block(state, block))
        {
            fluid.push(cell.at);
        }
    }
    if fluid.is_empty() {
        return None;
    }
    // A fill column runs densely from the discovered ground up to the authored
    // floor, so its lowest fill row is the ground's neighbour.
    let mut supports: std::collections::BTreeMap<[i32; 2], i32> = std::collections::BTreeMap::new();
    for cell in cells {
        if cell.state == state.registry.states.air || cell.at[1] > floor {
            continue;
        }
        supports
            .entry([cell.at[0], cell.at[2]])
            .and_modify(|y| *y = (*y).min(cell.at[1]))
            .or_insert(cell.at[1]);
    }
    let columns: Vec<([i32; 2], i32)> = supports
        .into_iter()
        .map(|(column, lowest_fill)| (column, lowest_fill - 1))
        .collect();
    let lowest = columns.iter().map(|(_, support)| *support).min()?;
    Some(crate::plugin_structures::LiquidProof {
        lowest,
        columns,
        cells: fluid,
    })
}

/// Re-prove a paused plan's liquid facts before it resumes. `proof` is the
/// envelope the admission certified, paid cells included; the cursor, the
/// consumed list and every paid cell stay untouched, so a resume never
/// re-plans and never re-charges a cell that is already paid. A new supply, a
/// mixed body, an opened shell, a player-attributed block or a removed support
/// refuses the resume, and the plan stays paused for the owner to see.
/// `territory` is the working settlement: inside it no block history is read
/// (owner D27.4).
pub(crate) fn revalidate(
    state: &TickState,
    territory: Option<&crate::settlements::SettlementRecord>,
    proof: &crate::plugin_structures::LiquidProof,
    origin: [i32; 3],
    footprint: [i32; 3],
) -> Result<(), mc_script::ScriptOperationFailure> {
    use mc_script::ScriptOperationFailure as Failure;
    let attributed = |at: [i32; 3]| {
        !territory.is_some_and(|record| record.contains(at)) && state.block_log.player_touched(at)
    };
    let mut liquids = Liquids::default();
    for at in &proof.cells {
        let Some(block) = state.world.physics_block_at(at[0], at[1], at[2]) else {
            continue;
        };
        if block == state.registry.states.air {
            continue;
        }
        // Admission refuses a player-attributed cell outside the territory,
        // fluid included; a block a player put there into the certified volume
        // is the same refusal on resume.
        if attributed(*at) {
            return Err(Failure::Blocked);
        }
        match contained(state, block) {
            Some((Fluid::Lava, _)) => liquids.lava += 1,
            Some((Fluid::Water, _)) if fluid_block(state, block) => liquids.water += 1,
            Some(_) => liquids.preserved_water = true,
            None => continue,
        }
        if fluid_block(state, block) {
            liquids.top = Some(liquids.top.map_or(at[1], |top| top.max(at[1])));
        }
    }
    if liquids.total() > MAX_LIQUID_CELLS as u32 {
        return Err(Failure::Blocked);
    }
    if liquids.lava > 0 && (liquids.water > 0 || liquids.preserved_water) {
        return Err(Failure::Blocked);
    }
    if liquids.preserved_water {
        return Err(Failure::Blocked);
    }
    if liquids.top.is_none() {
        return Ok(());
    }
    // Read the real supports: the ground the bowl was proved on must still hold
    // the liquid up, or the remaining flow leaves the lot.
    for ([x, z], support) in &proof.columns {
        let Some(ground) = state.world.physics_block_at(*x, *support, *z) else {
            return Err(Failure::Blocked);
        };
        if ground == state.registry.states.air
            || fluid_block(state, ground)
            || attributed([*x, *support, *z])
        {
            return Err(Failure::Blocked);
        }
    }
    let max: [i32; 3] = std::array::from_fn(|axis| origin[axis] + footprint[axis] - 1);
    finish(state, &liquids, origin, max, proof.lowest, &proof.columns).map_err(|_| Failure::Blocked)
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

/// An actual cut into air opens no path for a fluid: the target and its six
/// neighbours hold no water, lava or waterlogged block at the write, whatever
/// the cut (tree, plant cover, changed cell or terrain, credited or not). A
/// source placed after admission refuses the cut instead of flowing into the
/// site. The liquid stage still fills every admitted source before any cut, so
/// an admitted pool only waits here until its remaining flow has drained.
pub(crate) fn dry_cut(
    state: &TickState,
    at: [i32; 3],
) -> Result<(), mc_script::ScriptOperationFailure> {
    use mc_script::ScriptOperationFailure as Failure;
    for step in [
        [0, 0, 0],
        [1, 0, 0],
        [-1, 0, 0],
        [0, 1, 0],
        [0, -1, 0],
        [0, 0, 1],
        [0, 0, -1],
    ] {
        let block = state
            .world
            .block_at(at[0] + step[0], at[1] + step[1], at[2] + step[2])
            .ok_or(Failure::Unloaded)?;
        if state
            .registry
            .blocks
            .by_id(mc_world::BlockStateId(block))
            .is_none_or(|definition| definition.has_fluid())
        {
            return Err(Failure::Blocked);
        }
    }
    Ok(())
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
    let pos = crate::boundary::BlockPos {
        x: at[0],
        y: at[1],
        z: at[2],
    };
    // A tick already outstanding for that neighbourhood is the wake: the gate
    // stays a no-op until the drain consumes it, so a repeated wait costs no
    // extra queue entry, chunk dirt or journal record.
    if state.fluid_ticks.queued_around(pos) {
        return;
    }
    crate::fluids::schedule_around(state, pos);
}
