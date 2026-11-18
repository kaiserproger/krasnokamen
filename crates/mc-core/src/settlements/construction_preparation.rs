//! Canonical preparation materialization and unpaid interference recovery.
use super::*;

/// A field: farmland and the crop standing on it. Inside the working
/// settlement's territory a needed lot cuts it with its vanilla loot credited
/// and re-places paid dirt where the plan needs soil (owner D27.4, fields
/// included); outside the territory a field keeps refusing the lot.
pub(crate) fn field(state: &TickState, block: u32) -> bool {
    crop(state, block)
        || state
            .registry
            .blocks
            .by_id(mc_world::BlockStateId(block))
            .is_some_and(|definition| matches!(definition.block.id.path(), "farmland"))
}

/// A crop that grows on farmland.
pub(super) fn crop(state: &TickState, block: u32) -> bool {
    state
        .registry
        .blocks
        .by_id(mc_world::BlockStateId(block))
        .is_some_and(|definition| {
            matches!(
                definition.block.id.path(),
                "wheat"
                    | "carrots"
                    | "potatoes"
                    | "beetroots"
                    | "melon_stem"
                    | "pumpkin_stem"
                    | "attached_melon_stem"
                    | "attached_pumpkin_stem"
                    | "torchflower_crop"
            )
        })
}

pub(super) fn planned_cells(
    state: &TickState,
    project: &Project,
    species: &str,
    clearance: &mut crate::settlements::forestry::Clearance,
) -> Result<(ScriptWorldBounds, Vec<executor::PlannedCell>), site_refusals::Reason> {
    use site_refusals::Reason::Preparation;
    let blueprint = blueprints::get(&state.registry, &project.blueprint).ok_or(Preparation)?;
    let level = blueprint.level(project.level).ok_or(Preparation)?;
    let rotated = level.rotated(Rotation::from_quarter_turns(project.rotation));
    let mut quota = level.materials.clone();
    let mut cells: Vec<_> = rotated
        .removals
        .iter()
        .map(|offset| executor::PlannedCell {
            at: rotated.world(project.origin, *offset),
            state: state.registry.states.air,
            resource: None,
            quantity: 0,
        })
        .collect();
    let size = blueprint.footprint(Rotation::from_quarter_turns(project.rotation));
    let lot = crate::settlements::forestry::lot_bounds(state, project).ok_or(Preparation)?;
    let record = state
        .settlements
        .get(project.settlement)
        .ok_or(Preparation)?;
    // Owner D27.4: the plan reads no player history inside its own territory.
    clearance.within(record);
    let mut bottom = project.origin[1];
    let mut corridor = None;
    if project.building.is_none() {
        let paid_dirt = |at: [i32; 3]| executor::PlannedCell {
            at,
            state: state.registry.states.dirt,
            resource: Some("minecraft:dirt".into()),
            quantity: 1,
        };
        let authored = |at: [i32; 3]| {
            rotated
                .blocks
                .iter()
                .any(|(offset, _)| rotated.world(project.origin, *offset) == at)
        };
        for x in project.origin[0]..project.origin[0] + i32::from(size[0]) {
            for z in project.origin[2]..project.origin[2] + i32::from(size[2]) {
                let ground =
                    surface_y_for_lot(state, project.origin[1], x, z, Some(lot), clearance)
                        .ok_or(Preparation)?;
                // A field inside the territory (owner D27.4): every field cell
                // of the column from its ground (or the floor under an
                // overhang) up is cut in the clear stage, top-down so a crop
                // never loses its farmland first, and the clear stage credits
                // its vanilla loot. Farmland at or below the floor is soil the
                // plan still needs, so the same cell is re-placed as paid dirt
                // unless the authored floor covers it; a crop in the fill range
                // gets the paid fill below. The lot neither creates nor loses material.
                let top = project.origin[1] + i32::from(size[1]);
                for y in (ground.min(project.origin[1])..top).rev() {
                    let block = state.world.physics_block_at(x, y, z).ok_or(Preparation)?;
                    if !field(state, block) || !record.contains([x, y, z]) {
                        continue;
                    }
                    cells.push(executor::PlannedCell {
                        at: [x, y, z],
                        state: state.registry.states.air,
                        resource: None,
                        quantity: 0,
                    });
                    if !crop(state, block) && y <= project.origin[1] && !authored([x, y, z]) {
                        bottom = bottom.min(y);
                        cells.push(paid_dirt([x, y, z]));
                    }
                }
                for y in ground + 1..=project.origin[1] {
                    // The authored floor already pays for its own cells.
                    if y == project.origin[1] && authored([x, y, z]) {
                        continue;
                    }
                    bottom = bottom.min(y);
                    cells.push(paid_dirt([x, y, z]));
                }
            }
        }
    }
    let mut cuts = Vec::new();
    for x in project.origin[0]..project.origin[0] + i32::from(size[0]) {
        for z in project.origin[2]..project.origin[2] + i32::from(size[2]) {
            for y in project.origin[1] + 1..project.origin[1] + i32::from(size[1]) {
                let block = state.world.physics_block_at(x, y, z).ok_or(Preparation)?;
                if block != state.registry.states.air
                    && (clearable(state, block)
                        || (project.building.is_none() && excavatable(state, block)))
                    && !cells.iter().any(|cell| cell.at == [x, y, z])
                {
                    cuts.push(executor::PlannedCell {
                        at: [x, y, z],
                        state: state.registry.states.air,
                        resource: None,
                        quantity: 0,
                    });
                }
            }
        }
    }
    // Cut top-down: a standing builder loses at most one block of support per
    // work unit, and no cell is removed from under surviving terrain above it.
    cuts.sort_by_key(|cell| std::cmp::Reverse(cell.at[1]));
    cells.append(&mut cuts);
    for (offset, block) in &rotated.blocks {
        let placed = *block;
        let (resource, quantity) =
            if let Some((item, quantity)) = blueprints::material_for(&state.registry, *block) {
                let (_, remaining) = quota
                    .iter_mut()
                    .find(|(id, _)| *id == item)
                    .ok_or(Preparation)?;
                *remaining = remaining.checked_sub(quantity).ok_or(Preparation)?;
                (
                    Some(
                        state
                            .registry
                            .items
                            .name_of(item)
                            .ok_or(Preparation)?
                            .to_string(),
                    ),
                    quantity,
                )
            } else {
                (None, 0)
            };
        cells.push(executor::PlannedCell {
            at: rotated.world(project.origin, *offset),
            state: placed.0,
            resource,
            quantity,
        });
    }
    // Catalog materials must map exactly to physical cells; never build free
    // blocks because a blueprint cannot be funded by its declared stock.
    if quota.iter().any(|(_, count)| *count != 0) {
        return Err(Preparation);
    }
    apply_species(state, &mut cells, species).ok_or(Preparation)?;
    if project.building.is_none() {
        let preparation = crate::settlements::forestry::cells(state, project, size, clearance)
            .ok_or(Preparation)?;
        corridor = preparation.corridor;
        let mut preparation = preparation.cells;
        if !preparation.is_empty() {
            let removed: std::collections::BTreeSet<_> =
                preparation.iter().map(|cell| cell.at).collect();
            cells.retain(|cell| {
                cell.state != state.registry.states.air || !removed.contains(&cell.at)
            });
            preparation.append(&mut cells);
            cells = preparation;
        }
    }
    if project.building.is_some() {
        let lot = crate::settlements::forestry::lot_bounds(state, project).ok_or(Preparation)?;
        let mut cleared = executor::upgrade_trees_clear(
            state,
            project.settlement,
            &project.blueprint,
            project.origin,
            &cells,
            Some(lot),
            clearance,
        )
        .map_err(|failure| match failure {
            mc_script::ScriptOperationFailure::Unloaded => site_refusals::Reason::Unloaded,
            mc_script::ScriptOperationFailure::Forbidden
            | mc_script::ScriptOperationFailure::Blocked => {
                site_refusals::Reason::ProtectedObstruction
            }
            _ => Preparation,
        })?;
        if !cleared.is_empty() {
            // The same whole-tree evidence a new site uses: a registered root
            // whose trunk is cut here is felled as a whole tree.
            clearance.append_whole_trees(state, &mut cleared);
            let removed: std::collections::BTreeSet<_> =
                cleared.iter().map(|cell| cell.at).collect();
            cells.retain(|cell| {
                cell.state != state.registry.states.air || !removed.contains(&cell.at)
            });
            cleared.append(&mut cells);
            cells = cleared;
        }
    }
    // Include the whole placement extent (roof crowns/yards) in executor
    // interference fencing; certification uses the narrower authored bounds.
    let mut min = [project.origin[0], bottom, project.origin[2]];
    let mut max = std::array::from_fn(|axis| project.origin[axis] + i32::from(size[axis]) - 1);
    // Lot cells use the shared managed-tree exception; access outside the lot
    // remains protected in forestry::cells, not the enclosing empty rectangle.
    for cell in &cells {
        for axis in 0..3 {
            min[axis] = min[axis].min(cell.at[axis]);
            max[axis] = max[axis].max(cell.at[axis]);
        }
    }
    if let Some(corridor) = corridor {
        for axis in 0..3 {
            min[axis] = min[axis].min(corridor.min[axis]);
            max[axis] = max[axis].max(corridor.max[axis]);
        }
    }
    let bounds = ScriptWorldBounds::new(min, max).map_err(|_| Preparation)?;
    Ok((bounds, cells))
}

/// One project's own lot: parallel sites must not read the first project's
/// bounds (D8 review R2-5).
pub(crate) fn project_lot(
    state: &TickState,
    record: &SettlementRecord,
    handle: &str,
) -> Option<ScriptWorldBounds> {
    access::project_for_structure(record, handle)
        .and_then(|project| crate::settlements::forestry::lot_bounds(state, project))
}

/// One project's own floor level (D8 review R2-5).
pub(crate) fn project_floor(record: &SettlementRecord, handle: &str) -> i32 {
    access::project_for_structure(record, handle)
        .map_or(record.anchor[1], |project| project.origin[1])
}

/// The only stale-certification trigger for a paused clear stage, evaluated
/// inside the construction owner: the active order keeps its queue slot and a
/// registered root in the remaining clear cells is no longer a sapling. Natural
/// decay and foreign edits are not this condition.
pub(crate) fn stale_tree_certification(
    state: &TickState,
    id: SettlementId,
    handle: &str,
    remaining: &[[i32; 3]],
    is_sapling: impl Fn(u32) -> bool,
) -> bool {
    let Some(record) = state.settlements.get(id) else {
        return false;
    };
    let keeps_its_place = access::project_for_structure(record, handle)
        .is_none_or(|project| project.ordered)
        && orders(record).next().is_none();
    keeps_its_place
        && remaining.iter().any(|at| {
            record.planted_trees.contains(at)
                && state
                    .world
                    .physics_block_at(at[0], at[1], at[2])
                    .is_some_and(|block| !is_sapling(block))
        })
}

/// Called only after the canonical executor detects an unpaid, paused clear
/// manifest. Its world image is no longer a certificate of the current tree.
/// A paid upgrade's clear stage follows the same rule as a new site: cancel and
/// requeue the owner order rather than resume a stale tree certification.
pub(crate) fn changed_project(
    state: &TickState,
    id: SettlementId,
    handle: &str,
) -> Option<Project> {
    project_for_structure(state.settlements.get(id)?, handle)
        .filter(|project| !layout::infra(&project.blueprint))
        .cloned()
}

#[cfg(test)]
#[path = "construction_preparation_tests.rs"]
mod tests;
