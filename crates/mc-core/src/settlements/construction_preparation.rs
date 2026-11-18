//! Canonical preparation materialization and unpaid interference recovery.
use super::*;

/// A prepared plan: the executor's bounds and its ordered work cells.
pub(super) type Plan = (ScriptWorldBounds, Vec<executor::PlannedCell>);

pub(super) fn planned_cells(
    state: &TickState,
    project: &Project,
    species: &str,
    clearance: &mut crate::settlements::forestry::Clearance,
) -> Result<Plan, site_refusals::Reason> {
    cell_plan(state, project, species, clearance, true)
}

/// The same plan, with the upgrade placement fence optionally applied. The
/// refusal diagnosis re-plans without it so a refusal can name its cell; every
/// other caller takes the fenced plan.
fn cell_plan(
    state: &TickState,
    project: &Project,
    species: &str,
    clearance: &mut crate::settlements::forestry::Clearance,
    fence: bool,
) -> Result<Plan, site_refusals::Reason> {
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
        // One evidence map per attempt: the tree half and the placement fence
        // read the same last-committed-state image.
        let evidence = executor::previous_committed(
            state,
            project.settlement,
            &project.blueprint,
            project.origin,
        )
        .map_err(|_| Preparation)?;
        let mut cleared =
            executor::upgrade_trees_clear(state, &evidence, &cells, Some(lot), clearance).map_err(
                |failure| match failure {
                    mc_script::ScriptOperationFailure::Unloaded => site_refusals::Reason::Unloaded,
                    mc_script::ScriptOperationFailure::Forbidden
                    | mc_script::ScriptOperationFailure::Blocked => {
                        site_refusals::Reason::ProtectedObstruction
                    }
                    _ => Preparation,
                },
            )?;
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
        // D7 §2.4: an upgrade may only write where the previous build wrote or
        // where the world is empty. Village territory is the village's own
        // property (owner D27): a foreign block standing there is cleared first
        // and its drop is credited to storage by the debit, so only a cell the
        // executor cannot clear refuses. Nothing is debited, receipted or placed
        // before this refuses.
        if fence {
            let mut cuts = Vec::new();
            for action in placement_actions(
                state,
                project.settlement,
                &cells,
                &evidence,
                project.origin[1],
            ) {
                match action {
                    Action::Refuse(divergence) => return Err(divergence.reason),
                    Action::Clear(at) => cuts.push(executor::PlannedCell {
                        at,
                        state: state.registry.states.air,
                        resource: None,
                        quantity: 0,
                    }),
                }
            }
            if !cuts.is_empty() {
                // The village's own blocks go first, so every placement lands on
                // air the plan created and the drop is credited exactly once.
                cuts.append(&mut cells);
                cells = cuts;
            }
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

/// A refused placement cell: the admission reason and the block the owner has
/// to clear, so the log, the owner status and the player message name the same
/// position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Divergence {
    pub(super) reason: site_refusals::Reason,
    pub(super) at: [i32; 3],
    /// `None` when the world itself is unloaded there.
    pub(super) live: Option<u32>,
}

/// The concrete admission refusal of an upgrade, planned again without the
/// fence so it names the cell. Read-only, and only called once a refusal has
/// already happened: an admitted pass pays nothing for it.
pub(super) fn refusal(state: &TickState, project: &Project, species: &str) -> Option<Divergence> {
    if project.building.is_none() {
        return None;
    }
    let evidence = executor::previous_committed(
        state,
        project.settlement,
        &project.blueprint,
        project.origin,
    )
    .ok()?;
    let (_, cells) = cell_plan(state, project, species, &mut Default::default(), false).ok()?;
    placement_actions(
        state,
        project.settlement,
        &cells,
        &evidence,
        project.origin[1],
    )
    .into_iter()
    .find_map(|action| match action {
        Action::Refuse(divergence) => Some(divergence),
        Action::Clear(_) => None,
    })
}

/// The admission refusal text of a project. An upgrade divergence names the cell
/// the owner has to clear, in the player message as in the owner status; every
/// other reason keeps its bare name.
pub(super) fn refusal_text(
    state: &TickState,
    project: &Project,
    species: &str,
    reason: site_refusals::Reason,
) -> String {
    match reason {
        site_refusals::Reason::PreviousBuildDiverged => match refusal(state, project, species) {
            Some(divergence) => format!(
                "Construction admission refused: PreviousBuildDiverged at {},{},{}.",
                divergence.at[0], divergence.at[1], divergence.at[2]
            ),
            None => "Construction admission refused: PreviousBuildDiverged.".into(),
        },
        reason => format!("Construction admission refused: {reason:?}."),
    }
}

/// The queued upgrade's concrete admission refusal, naming the cell the owner
/// has to clear. A read-only probe: it stores nothing, so it disappears by
/// itself once the obstacle is gone and no lifecycle state is involved.
pub(super) fn upgrade_refusal(
    state: &TickState,
    id: SettlementId,
    building: BuildingId,
) -> Option<String> {
    let record = state.settlements.get(id)?;
    let target = record.buildings.get(&building)?;
    let (blueprint, origin, rotation) = upgrade_blueprint(state, id, target)?;
    let level = target.level + 1;
    let images = executor::source_images(state, id);
    let (species, _) = funded_variant(state, id, &blueprint.level(level)?.materials, &images)?;
    let project = Project {
        settlement: id,
        blueprint: blueprint.id.clone(),
        level,
        origin,
        rotation,
        building: Some(building),
        structure: String::new(),
        ordered: true,
        builder: None,
        order_seq: None,
    };
    let divergence = refusal(state, &project, species)?;
    let [x, y, z] = divergence.at;
    Some(match divergence.reason {
        site_refusals::Reason::Unloaded => format!("unloaded_worksite at {x},{y},{z}"),
        _ => format!("previous_build_diverged at {x},{y},{z}"),
    })
}

/// Report a queued request's admission refusal once per distinct reason: the
/// request keeps its FIFO slot and is retried on every planning pass, so a
/// repeating reason must not page the owner or the log each time. Runtime only.
pub(super) fn note_order_refusal(
    state: &mut TickState,
    id: SettlementId,
    seq: u64,
    reason: String,
) {
    let repeated = state
        .settlements
        .order_refusal
        .get(&(id, seq))
        .is_some_and(|previous| *previous == reason);
    if repeated {
        return;
    }
    state
        .settlements
        .order_refusal
        .insert((id, seq), reason.clone());
    tracing::warn!(settlement = id.0, seq, %reason, "settlement_construction_order_refused");
    notify_owner(state, id, reason);
}

/// What an upgrade must do with one planned placement cell: keep it, refuse the
/// upgrade, or clear what stands there first and credit its drop.
pub(super) enum Action {
    Refuse(Divergence),
    Clear([i32; 3]),
}

/// Classify every planned placement cell of an upgrade against the previous
/// build's own last committed state. A cell is fine when it holds air — already,
/// or because the plan empties it earlier (Opus F10) — or the previous build's
/// own block, compared by raw block id as `reopen_emptied` does: an opened door,
/// a connected fence or a lit furnace is the same block, not a divergence.
///
/// The foundation row, `floor`, is the site's own ground: there the floor layer
/// replaces the surface block by construction, and a new site's admission grants
/// exactly that — floor-replaceable, no block entity, no player edit — so an
/// upgrade inherits the grant instead of calling the ground it stands on foreign.
///
/// Everything else is village property inside the village's territory (owner D27,
/// 2026-11-18): the plan clears it and the debit credits its drop to storage, no
/// matter who built it. Outside the territory, and for a cell holding a block
/// entity — clearing a container would lose its contents, which no credit covers
/// — the upgrade refuses instead.
fn placement_actions(
    state: &TickState,
    settlement: SettlementId,
    cells: &[executor::PlannedCell],
    evidence: &std::collections::BTreeMap<[i32; 3], u32>,
    floor: i32,
) -> Vec<Action> {
    let identity = |block: u32| {
        state
            .registry
            .blocks
            .by_id(mc_world::BlockStateId(block))
            .map(|definition| definition.block.raw_id)
    };
    let territory = state
        .settlements
        .get(settlement)
        .map(|record| (record.anchor, i64::from(record.radius).pow(2)));
    let in_territory = |at: [i32; 3]| {
        territory.is_some_and(|(anchor, radius_sq)| {
            at.iter()
                .zip(anchor)
                .map(|(a, b)| (i64::from(*a) - i64::from(b)).pow(2))
                .sum::<i64>()
                <= radius_sq
        })
    };
    let air = state.registry.states.air;
    let mut emptied = std::collections::BTreeSet::new();
    let mut actions = Vec::new();
    for cell in cells {
        if cell.state == air {
            emptied.insert(cell.at);
            continue;
        }
        // An earlier air cell at this position is applied first, so the cell is
        // placed on air the plan created.
        if emptied.contains(&cell.at) {
            continue;
        }
        let Some(live) = state
            .world
            .physics_block_at(cell.at[0], cell.at[1], cell.at[2])
        else {
            actions.push(Action::Refuse(Divergence {
                reason: site_refusals::Reason::Unloaded,
                at: cell.at,
                live: None,
            }));
            continue;
        };
        if live == air {
            continue;
        }
        let own = evidence.get(&cell.at).copied().and_then(identity);
        if own.is_some() && own == identity(live) {
            continue;
        }
        let container = state
            .block_entities
            .contains_key(&(cell.at[0], cell.at[1], cell.at[2]));
        // The foundation grant, with the vetoes the site search applies there.
        if cell.at[1] == floor
            && floor_replaceable(state, live)
            && !container
            && !state.block_log.player_touched(cell.at)
        {
            continue;
        }
        if !container && in_territory(cell.at) {
            actions.push(Action::Clear(cell.at));
            continue;
        }
        actions.push(Action::Refuse(Divergence {
            reason: site_refusals::Reason::PreviousBuildDiverged,
            at: cell.at,
            live: Some(live),
        }));
    }
    actions
}

pub(crate) fn active_lot(
    state: &TickState,
    record: &SettlementRecord,
) -> Option<ScriptWorldBounds> {
    projects(record)
        .next()
        .and_then(|project| crate::settlements::forestry::lot_bounds(state, project))
}

/// The level ground discovery centres on: the active project's own floor, so a
/// builder's footing and the fill plan agree, else the bell's elevation.
pub(crate) fn active_floor(record: &SettlementRecord) -> i32 {
    projects(record)
        .next()
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
    let keeps_its_place = projects(record)
        .next()
        .filter(|project| project.structure == handle)
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

/// Funded upgrade candidates an autonomous pass may plan before it settles for
/// the first one the placement fence accepts: one refused building must not
/// starve every other building's upgrade, and one pass stays bounded.
const UPGRADE_CANDIDATES: usize = 4;

/// The next funded upgrade an autonomous pass may admit. A building whose
/// placement fence refuses it is skipped, not retried ahead of the others on
/// every pass; an unrelated plan failure keeps the candidate, so its own reason
/// still surfaces.
pub(super) fn upgrade_project(
    state: &TickState,
    record: &SettlementRecord,
    images: &[crate::plugin_inventories::native::Image],
) -> Option<(Project, String, u8)> {
    record
        .buildings
        .values()
        .filter_map(|building| {
            let (blueprint, origin, rotation) = upgrade_blueprint(state, record.id, building)?;
            let level = building.level + 1;
            let (species, color) =
                funded_variant(state, record.id, &blueprint.level(level)?.materials, images)?;
            let species = species.to_owned();
            Some((
                Project {
                    settlement: record.id,
                    blueprint: blueprint.id.clone(),
                    level,
                    origin,
                    rotation,
                    building: Some(building.id),
                    structure: String::new(),
                    ordered: false,
                    builder: None,
                    order_seq: None,
                },
                species,
                color,
            ))
        })
        .take(UPGRADE_CANDIDATES)
        .find(|(project, species, _)| fence_admits(state, project, species))
}

/// Whether the placement fence admits this project's plan: the autonomous
/// planner's cheap skip, so one diverged building cannot starve the others.
pub(super) fn fence_admits(state: &TickState, project: &Project, species: &str) -> bool {
    !matches!(
        planned_cells(state, project, species, &mut Default::default()),
        Err(site_refusals::Reason::PreviousBuildDiverged)
    )
}

/// The refusal a queued owner request is waiting on, if any: the owner status
/// asks this, so a request that cannot start says why, with the cell.
pub(super) fn queued_refusal(
    state: &TickState,
    id: SettlementId,
    order: &OwnerOrder,
) -> Option<String> {
    upgrade_refusal(state, id, order.building?)
}

#[cfg(test)]
#[path = "construction_preparation_tests.rs"]
mod tests;
