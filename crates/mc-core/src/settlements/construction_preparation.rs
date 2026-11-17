//! Canonical preparation materialization and unpaid interference recovery.
use super::*;
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
    let mut bottom = project.origin[1];
    let mut corridor = None;
    if project.building.is_none() {
        let dirt = state.registry.states.dirt;
        for x in project.origin[0]..project.origin[0] + i32::from(size[0]) {
            for z in project.origin[2]..project.origin[2] + i32::from(size[2]) {
                let ground =
                    surface_y_for_lot(state, project.origin[1], x, z, Some(lot), clearance)
                        .ok_or(Preparation)?;
                for y in ground + 1..=project.origin[1] {
                    // The authored floor already pays for its own cells.
                    if y == project.origin[1]
                        && rotated
                            .blocks
                            .iter()
                            .any(|(offset, _)| rotated.world(project.origin, *offset) == [x, y, z])
                    {
                        continue;
                    }
                    bottom = bottom.min(y);
                    cells.push(executor::PlannedCell {
                        at: [x, y, z],
                        state: dirt,
                        resource: Some("minecraft:dirt".into()),
                        quantity: 1,
                    });
                }
            }
        }
    }
    for x in project.origin[0]..project.origin[0] + i32::from(size[0]) {
        for z in project.origin[2]..project.origin[2] + i32::from(size[2]) {
            for y in project.origin[1] + 1..project.origin[1] + i32::from(size[1]) {
                let block = state.world.physics_block_at(x, y, z).ok_or(Preparation)?;
                if block != state.registry.states.air
                    && (clearable(state, block)
                        || (project.building.is_none() && excavatable(state, block)))
                    && !cells.iter().any(|cell| cell.at == [x, y, z])
                {
                    cells.push(executor::PlannedCell {
                        at: [x, y, z],
                        state: state.registry.states.air,
                        resource: None,
                        quantity: 0,
                    });
                }
            }
        }
    }
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
        executor::upgrade_trees_clear(
            state,
            project.settlement,
            &project.blueprint,
            project.origin,
            &cells,
        )
        .map_err(|failure| match failure {
            mc_script::ScriptOperationFailure::Unloaded => site_refusals::Reason::Unloaded,
            mc_script::ScriptOperationFailure::Forbidden
            | mc_script::ScriptOperationFailure::Blocked => {
                site_refusals::Reason::ProtectedObstruction
            }
            _ => Preparation,
        })?;
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

pub(crate) fn active_lot(state: &TickState, record: &SettlementRecord) -> Option<ScriptWorldBounds> {
    record
        .project
        .as_ref()
        .and_then(|project| crate::settlements::forestry::lot_bounds(state, project))
}

/// Called only after the canonical executor detects an unpaid, paused clear
/// manifest. Its world image is no longer a certificate of the current tree.
pub(crate) fn changed_project(
    state: &TickState,
    id: SettlementId,
    handle: &str,
) -> Option<Project> {
    state
        .settlements
        .get(id)?
        .project
        .as_ref()
        .filter(|project| {
            project.structure == handle
                && project.building.is_none()
                && !layout::infra(&project.blueprint)
        })
        .cloned()
}

#[cfg(test)]
#[path = "construction_preparation_tests.rs"]
mod tests;
