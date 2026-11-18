//! Settlement sources for the canonical structure ledger and executor. This
//! adapter owns neither a placement cursor nor a second placement loop.
pub(crate) use super::portions::PortionKind;
use super::*;
use crate::settlements::{SettlementId, inventory};

pub(crate) fn owner(id: SettlementId) -> String {
    format!("@settlement:{}", id.0)
}

pub(crate) struct PlannedCell {
    pub at: [i32; 3],
    pub state: u32,
    pub resource: Option<String>,
    pub quantity: u32,
    pub portion: PortionKind,
}

pub(crate) fn prepare(
    state: &mut TickState,
    settlement: SettlementId,
    blueprint: String,
    bounds: ScriptWorldBounds,
    site: ScriptWorldBounds,
    mut cells: Vec<PlannedCell>,
) -> Result<String, Failure> {
    if state.settlements.get(settlement).is_none() || cells.is_empty() || cells.len() > 32768 {
        return Err(Failure::InvalidRequest);
    }
    let origin = crate::settlements::blueprints::get(&state.registry, &blueprint)
        .map(|blueprint| {
            let height = i32::from(
                blueprint
                    .footprint(crate::settlements::blueprints::Rotation::from_quarter_turns(0))[1],
            );
            [site.min[0], site.max[1] - height + 1, site.min[2]]
        })
        .unwrap_or(site.min);
    if expedition_reserved(state, bounds, Some((settlement, &blueprint, origin))) {
        return Err(Failure::Blocked);
    }
    let ledger = ledger(state)?;
    if ledger.structures().values().any(|record| {
        let same_settlement = record.settlement == Some(settlement);
        // An in-flight structure of this settlement holds its own site, not the
        // wide clear-stage window: a neighbour lot may be prepared beside it
        // (D8 parallel construction). Foreign and committed structures keep the
        // full reservation.
        let held = if same_settlement && record.snapshot.state != ScriptStructureState::Committed {
            site_reservation(state, record).ok()
        } else {
            reserved_bounds(record).ok()
        };
        let wanted = if same_settlement || record.snapshot.state == ScriptStructureState::Committed
        {
            site
        } else {
            bounds
        };
        record.snapshot.dimension == state.dimension.name()
            && record.snapshot.state != ScriptStructureState::Cancelled
            && !spent_free_work(record)
            && held.is_some_and(|existing| overlaps(existing, wanted))
            // Only a *committed* structure on the same lot may be rebuilt or
            // upgraded in place; an in-flight structure on that lot blocks a
            // second admission (D8).
            && !(record.settlement == Some(settlement)
                && record.snapshot.state == ScriptStructureState::Committed
                && same_site(state, record, origin)
                && record
                    .snapshot
                    .blueprint_id
                    .strip_prefix("krasnokamen-settlements:")
                    == Some(blueprint.as_str()))
    }) {
        return Err(Failure::Blocked);
    }
    let revision = next(ledger.revision)?;
    let id = format!("settlement-{}-{revision}", settlement.0);
    if cells
        .windows(2)
        .any(|pair| pair[0].portion > pair[1].portion)
    {
        return Err(Failure::InvalidRequest);
    }
    // Source fills lead all air cuts, including registered-tree clearing.
    let liquid_stage =
        crate::settlements::construction::preparation::liquid::stage_cells(state, &cells);
    let liquid_proof =
        crate::settlements::construction::preparation::liquid::proof(state, &cells, site.min[1]);
    let bounds = match &liquid_proof {
        Some(proof) => ScriptWorldBounds::new(
            [
                bounds.min[0] - 1,
                bounds.min[1].min(proof.lowest),
                bounds.min[2] - 1,
            ],
            [bounds.max[0] + 1, bounds.max[1], bounds.max[2] + 1],
        )
        .map_err(|_| Failure::InvalidRequest)?,
        None => bounds,
    };
    let image = fingerprint(state, bounds, true)?;
    if !liquid_stage.is_empty() {
        for index in liquid_stage {
            cells[index].portion = PortionKind::Liquid;
        }
        cells.sort_by_key(|cell| cell.portion);
    }
    if cells.iter().any(|cell| {
        (0..3).any(|axis| cell.at[axis] < bounds.min[axis] || cell.at[axis] > bounds.max[axis])
    }) {
        return Err(Failure::InvalidRequest);
    }
    let mut groups: Vec<(PortionKind, Vec<Cell>)> = Vec::with_capacity(7);
    for cell in cells {
        if groups.last().is_none_or(|(kind, _)| *kind != cell.portion) {
            groups.push((cell.portion, Vec::new()));
        }
        groups.last_mut().expect("portion group").1.push(Cell {
            at: cell.at,
            state: cell.state,
            resource: cell.resource,
            quantity: cell.quantity,
        });
    }
    let mut counts = BTreeMap::new();
    let mut stages = Vec::with_capacity(groups.len());
    let mut batches = Vec::with_capacity(groups.len());
    for (kind, cells) in groups {
        if cells.is_empty() {
            continue;
        }
        let mut stage_counts = BTreeMap::new();
        for cell in &cells {
            if let Some(resource) = &cell.resource {
                *stage_counts.entry(resource.clone()).or_insert(0) += u64::from(cell.quantity);
                *counts.entry(resource.clone()).or_insert(0) += u64::from(cell.quantity);
            }
        }
        stages.push(ScriptStructureStagePlan::new(
            kind.name().into(),
            cells.len() as u32,
            cells.len() as u64,
            materials(&stage_counts),
        ));
        batches.push(cells);
    }
    let portions = portions::certify(state, &batches)?;
    let plan = resource_plan(&stages);
    if !material_free(&stages) {
        plan.validate().map_err(|_| Failure::InvalidRequest)?;
    }
    let snapshot = ScriptStructureSnapshot::new(
        id.clone(),
        format!("krasnokamen-settlements:{blueprint}"),
        state.dimension.name().into(),
        ScriptStructureState::Prepared,
        revision,
        site.min,
        0,
        std::array::from_fn(|axis| site.max[axis] - site.min[axis] + 1),
        stages,
        0,
        0,
        inventories::resource_plan_hash(&plan),
        None,
        0,
        Vec::new(),
        materials(&counts),
        None,
        Vec::new(),
    );
    snapshot.validate().map_err(|_| Failure::InvalidRequest)?;
    drop(ledger);
    let mut ledger = ledger_mut(state)?;
    ledger.insert_structure(
        id.clone(),
        Structure {
            owner: owner(settlement),
            settlement: Some(settlement),
            liquid: liquid_proof,
            settlement_preparation: (bounds != site).then_some(bounds),
            snapshot,
            blueprint_hash: inventories::hash(&batches),
            image,
            portions,
            admission: provenance::Admission::survey(state),
            cells: batches,
        },
    );
    ledger.revision = revision;
    save(state, ledger);
    Ok(id)
}

pub(crate) fn area_reserved(state: &TickState, bounds: ScriptWorldBounds) -> bool {
    ledger(state).map_or(true, |ledger| {
        ledger.structures().values().any(|record| {
            record.snapshot.dimension == state.dimension.name()
                && record.snapshot.state != ScriptStructureState::Cancelled
                && reserved_bounds(record).is_ok_and(|existing| overlaps(existing, bounds))
        })
    })
}

/// Finished road paving and meadow clearing: free surface work that leaves
/// nothing to protect. Lots and clearings keep honouring it through
/// [`area_reserved`]; the perimeter wall, whose line crosses paved roads and
/// cleared meadow, and the well do not.
fn spent_free_work(record: &Structure) -> bool {
    record.snapshot.state == ScriptStructureState::Committed
        && record
            .snapshot
            .blueprint_id
            .strip_prefix("krasnokamen-settlements:")
            .is_some_and(|name| {
                matches!(
                    name,
                    crate::settlements::layout::ROAD | crate::settlements::layout::MEADOW
                )
            })
}

/// [`area_reserved`] without the spent free work of [`spent_free_work`].
pub(crate) fn area_held(state: &TickState, bounds: ScriptWorldBounds) -> bool {
    ledger(state).map_or(true, |ledger| {
        ledger.structures().values().any(|record| {
            record.snapshot.dimension == state.dimension.name()
                && record.snapshot.state != ScriptStructureState::Cancelled
                && !spent_free_work(record)
                && reserved_bounds(record).is_ok_and(|existing| overlaps(existing, bounds))
        })
    })
}

pub(crate) fn cancel(state: &mut TickState, id: SettlementId) -> Result<(), Failure> {
    let ledger = ledger(state)?;
    if !ledger.structures().values().any(|record| {
        record.settlement == Some(id) && record.snapshot.state != ScriptStructureState::Cancelled
    }) {
        return Ok(());
    }
    drop(ledger);
    let mut ledger = ledger_mut(state)?;
    let revision = next(ledger.revision)?;
    for record in ledger.structures_mut_where(|record| record.settlement == Some(id)) {
        record.snapshot.state = ScriptStructureState::Cancelled;
        record.snapshot.revision = revision;
        record.snapshot.pause_reason = None;
        record.snapshot.reservation_ref = None;
    }
    ledger.revision = revision;
    save(state, ledger);
    Ok(())
}

/// Cancel one prepared structure of this settlement, leaving committed
/// structures and the materials already in stock or carry untouched.
pub(crate) fn cancel_structure(
    state: &mut TickState,
    settlement: SettlementId,
    handle: &str,
) -> Result<(), Failure> {
    let mut ledger = ledger_mut(state)?;
    structure(&ledger, &owner(settlement), handle)?;
    let revision = next(ledger.revision)?;
    let record = ledger.structure_mut(handle).ok_or(Failure::NotFound)?;
    record.snapshot.state = ScriptStructureState::Cancelled;
    record.snapshot.revision = revision;
    record.snapshot.pause_reason = None;
    record.snapshot.reservation_ref = None;
    ledger.revision = revision;
    save(state, ledger);
    Ok(())
}

pub(crate) fn status(
    state: &TickState,
    settlement: SettlementId,
    handle: &str,
) -> Result<ScriptStructureSnapshot, Failure> {
    let ledger = ledger(state)?;
    Ok(structure(&ledger, &owner(settlement), handle)?
        .snapshot
        .clone())
}

/// N3 alone pauses a grown certified sapling as `footprint_changed`. Before
/// any felling, its owner order may be cancelled and requeued in the same FIFO
/// slot. Otherwise recovery resumes with the pending per-cell evidence intact.
/// Completed fill/build damage is appended as a newly certified repair portion;
/// a solid foreign replacement of an already-processed cell is left alone.
/// Returns whether recovery resumed or cancelled the structure.
pub(crate) fn recover(
    state: &mut TickState,
    settlement: SettlementId,
    handle: &str,
) -> Result<bool, Failure> {
    let ledger = ledger(state)?;
    let record = structure(&ledger, &owner(settlement), handle)?;
    let paused = record.snapshot.state == ScriptStructureState::Paused
        && record.snapshot.pause_reason.as_deref() == Some("footprint_changed");
    let committed = record.snapshot.state == ScriptStructureState::Committed;
    if !paused && !committed {
        return Ok(false);
    }
    let retry = (paused
        && record.snapshot.consumed.is_empty()
        // Once any clear cell is worked the manifest itself authorizes the
        // remaining cells; cancelling here could never re-certify a partially
        // felled tree whose registration the cut already retired.
        && record.snapshot.completed_work_units == 0
        && record
            .snapshot
            .stages
            .get(record.snapshot.current_stage_index as usize)
            .is_some_and(|stage| PortionKind::is_preparation(&stage.stage))
        // Only a registered root that grew out of its certified sapling, and
        // only while the order keeps its queue slot, may cancel.
        && crate::settlements::construction::preparation::stale_tree_certification(
            state,
            settlement,
            handle,
            &record
                .cells
                .get(record.snapshot.current_stage_index as usize)
                .map(|cells| {
                    cells
                        .iter()
                        .skip(record.snapshot.completed_work_units as usize)
                        .map(|cell| cell.at)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
            |block| {
                state
                    .registry
                    .blocks
                    .by_id(mc_world::BlockStateId(block))
                    .is_some_and(|definition| {
                        definition.block.id.path().ends_with("_sapling")
                    })
            },
        ))
    .then(|| {
        crate::settlements::construction::preparation::changed_project(state, settlement, handle)
    })
    .flatten();
    if let Some(project) = retry {
        drop(ledger);
        crate::settlements::construction::retry_preparation(state, settlement, &project)
            .map_err(|_| Failure::Blocked)?;
        return Ok(true);
    }
    let target = world(state, &record.snapshot.dimension)?;
    let mut repair = Vec::new();
    let mut seen = BTreeSet::new();
    for (index, stage) in record.snapshot.stages.iter().enumerate().rev() {
        if !matches!(stage.stage.as_str(), "liquid" | "fill" | "build") {
            continue;
        }
        let current = record.snapshot.current_stage_index as usize;
        let completed = if current > index {
            record.cells[index].len()
        } else if current == index {
            record.snapshot.completed_work_units as usize
        } else {
            0
        };
        for cell in record.cells[index].iter().take(completed).rev() {
            if free_soil_work(target, cell) || !seen.insert(cell.at) {
                continue;
            }
            let now = target
                .world
                .block_at(cell.at[0], cell.at[1], cell.at[2])
                .ok_or(Failure::Unloaded)?;
            let actual = target
                .registry
                .blocks
                .by_id(mc_world::BlockStateId(now))
                .ok_or(Failure::InvalidRequest)?;
            let expected = target
                .registry
                .blocks
                .by_id(mc_world::BlockStateId(cell.state))
                .ok_or(Failure::InvalidRequest)?;
            // Only an emptied cell is re-placed, as in `reopen_emptied`: a
            // solid foreign block in a placed cell keeps its place.
            if actual.block.raw_id != expected.block.raw_id
                && crate::settlements::construction::clearable(target, now)
            {
                repair.push(cell.clone());
            }
        }
    }
    repair.reverse();
    if committed && repair.is_empty() {
        return Ok(false);
    }
    if let Some(proof) = &record.liquid {
        crate::settlements::construction::preparation::liquid::revalidate(
            target,
            proof,
            record.snapshot.origin,
            record.snapshot.reserved_footprint,
        )?;
    }
    drop(ledger);
    let mut ledger = ledger_mut(state)?;
    let mut record = take_structure(&mut ledger, &owner(settlement), handle)?;
    // The repair stage is derived afresh, so repeated interference cannot
    // accumulate stages or charge again for cells already repaired.
    if let Some(index) = record
        .snapshot
        .stages
        .iter()
        .position(|stage| stage.stage == "repair")
    {
        record.snapshot.stages.truncate(index);
        record.cells.truncate(index);
        record.portions.truncate(index);
        if record.snapshot.current_stage_index as usize >= index {
            record.snapshot.current_stage_index = index as u32;
            record.snapshot.completed_work_units = 0;
        }
    }
    if !repair.is_empty() {
        let mut counts = BTreeMap::new();
        for cell in &repair {
            if let Some(resource) = &cell.resource {
                *counts.entry(resource.clone()).or_insert(0) += u64::from(cell.quantity);
            }
        }
        record.snapshot.stages.push(ScriptStructureStagePlan::new(
            "repair".into(),
            repair.len() as u32,
            repair.len() as u64,
            materials(&counts),
        ));
        record
            .portions
            .extend(portions::certify(state, std::slice::from_ref(&repair))?);
        record.cells.push(repair);
    }
    record.snapshot.resource_plan_hash =
        inventories::resource_plan_hash(&resource_plan(&record.snapshot.stages));
    record.snapshot.remaining = remaining(&record);
    record.snapshot.state = if record.snapshot.current_stage_index as usize == record.cells.len() {
        ScriptStructureState::Committed
    } else {
        ScriptStructureState::Running
    };
    record.snapshot.pause_reason = None;
    ledger.revision = next(ledger.revision)?;
    record.snapshot.revision = ledger.revision;
    ledger.insert_structure(handle.to_owned(), record);
    save(state, ledger);
    Ok(true)
}

/// Name of the final stage that re-places authored cells a foreign edit emptied.
const REPAIR: &str = "repair";

/// Whether a manifest cell is free soil work: no material was ever charged, so
/// the cell only rewrites the ground already there.
fn free_soil_work(state: &TickState, cell: &Cell) -> bool {
    cell.resource.is_none()
        && state
            .registry
            .blocks
            .by_id(mc_world::BlockStateId(cell.state))
            .is_some_and(|definition| matches!(definition.block.id.path(), "dirt" | "dirt_path"))
}

/// Free soil work can only rewrite the ground already present, never create
/// ground or destroy a foreign block. Debit and publication share this gate.
pub(super) fn soil_work_noop(state: &TickState, cell: &Cell, current: u32) -> bool {
    if cell.resource.is_some()
        || crate::settlements::construction::preparation::liquid::fluid_block(state, current)
    {
        return false;
    }
    if cell.state == state.registry.states.dirt {
        return !crate::settlements::construction::relayout::village_soil(state, current);
    }
    Some(cell.state) == crate::settlements::layout::paved_state(state)
        && current != cell.state
        && !crate::settlements::construction::relayout::soil(state, current)
}

/// Completion check of a committed building structure: an authored
/// (non-air) cell that a foreign edit emptied (air or a replaceable plant where
/// the blueprint needs a block) is queued into the `repair` stage, which the
/// ordinary placement loop then re-places, paying the cell's material again;
/// the structure leaves `Committed` for `Running`. A solid foreign block in an
/// authored cell is not emptied and is never replaced.
/// One pass over the structure's cells at completion, never per tick. Returns
/// whether cells were queued (the building must not be certified yet).
pub(crate) fn reopen_emptied(
    state: &mut TickState,
    settlement: SettlementId,
    handle: &str,
) -> Result<bool, Failure> {
    let ledger = ledger(state)?;
    let record = structure(&ledger, &owner(settlement), handle)?;
    if record.snapshot.state != ScriptStructureState::Committed {
        return Ok(false);
    }
    let target = world(state, &record.snapshot.dimension)?;
    let identity = |block: u32| {
        target
            .registry
            .blocks
            .by_id(mc_world::BlockStateId(block))
            .map(|definition| definition.block.raw_id)
    };
    let mut seen = BTreeSet::new();
    let mut stale = Vec::new();
    for cell in record
        .cells
        .iter()
        .rev()
        .flat_map(|cells| cells.iter().rev())
    {
        if free_soil_work(target, cell)
            || !seen.insert(cell.at)
            || cell.state == target.registry.states.air
        {
            continue;
        }
        let now = target
            .world
            .block_at(cell.at[0], cell.at[1], cell.at[2])
            .ok_or(Failure::Unloaded)?;
        if identity(now) != identity(cell.state)
            && crate::settlements::construction::clearable(target, now)
        {
            stale.push(cell.clone());
        }
    }
    if stale.is_empty() {
        return Ok(false);
    }
    drop(ledger);
    let mut ledger = ledger_mut(state)?;
    let mut record = take_structure(&mut ledger, &owner(settlement), handle)?;
    let certificate = portions::certify(state, std::slice::from_ref(&stale))?.remove(0);
    let mut counts = BTreeMap::new();
    for cell in &stale {
        if let Some(resource) = &cell.resource {
            *counts.entry(resource.clone()).or_insert(0) += u64::from(cell.quantity);
        }
    }
    let plan = ScriptStructureStagePlan::new(
        REPAIR.into(),
        stale.len() as u32,
        stale.len() as u64,
        materials(&counts),
    );
    // The repair stage is always the last: a second pass replaces its cells.
    let index = match record
        .snapshot
        .stages
        .iter()
        .position(|s| s.stage == REPAIR)
    {
        Some(index) => {
            record.snapshot.stages[index] = plan;
            record.cells[index] = stale;
            record.portions[index] = certificate;
            index
        }
        None => {
            record.snapshot.stages.push(plan);
            record.cells.push(stale);
            record.portions.push(certificate);
            record.snapshot.stages.len() - 1
        }
    };
    record.snapshot.current_stage_index = index as u32;
    record.snapshot.completed_work_units = 0;
    record.snapshot.state = ScriptStructureState::Running;
    record.snapshot.remaining = remaining(&record);
    record.snapshot.resource_plan_hash =
        inventories::resource_plan_hash(&resource_plan(&record.snapshot.stages));
    record.blueprint_hash = inventories::hash(&record.cells);
    ledger.revision = next(ledger.revision)?;
    record.snapshot.revision = ledger.revision;
    ledger.insert_structure(handle.to_owned(), record);
    save(state, ledger);
    Ok(true)
}

pub(crate) fn next_cell(
    state: &TickState,
    settlement: SettlementId,
    handle: &str,
) -> Result<Option<[i32; 3]>, Failure> {
    let ledger = ledger(state)?;
    let record = structure(&ledger, &owner(settlement), handle)?;
    Ok(record
        .cells
        .get(record.snapshot.current_stage_index as usize)
        .and_then(|cells| cells.get(record.snapshot.completed_work_units as usize))
        .map(|cell| cell.at))
}

/// The planned world state of the next cell, for naming a wait: the gate reads
/// the plan, not the block that is about to be replaced.
pub(crate) fn next_plan_state(
    state: &TickState,
    settlement: SettlementId,
    handle: &str,
) -> Result<Option<u32>, Failure> {
    let ledger = ledger(state)?;
    let record = structure(&ledger, &owner(settlement), handle)?;
    Ok(record
        .cells
        .get(record.snapshot.current_stage_index as usize)
        .and_then(|cells| cells.get(record.snapshot.completed_work_units as usize))
        .map(|cell| cell.state))
}

/// The cell outside the territory whose lapsed evidence or retained player
/// edit refuses this structure's next commit (PrepLifecycle §2.3), if any.
pub(crate) fn provenance_refusal(
    state: &TickState,
    settlement: SettlementId,
    handle: &str,
) -> Option<[i32; 3]> {
    let ledger = ledger(state).ok()?;
    let record = structure(&ledger, &owner(settlement), handle).ok()?;
    let index = record.snapshot.current_stage_index as usize;
    let offset = record.snapshot.completed_work_units as usize;
    let next = record.cells.get(index)?.get(offset..offset + 1)?;
    portions::lapsed_refusal(state, record)
        .ok()
        .flatten()
        .or_else(|| {
            provenance::refused_cell(
                state,
                settlement,
                record.admission,
                &record.cells,
                [index, offset],
                next,
            )
        })
}

/// Borrow the canonical stage rather than inferring clearing from a live tree:
/// leaves may decay before their already-planned removal cell is reached.
pub(crate) fn clearing(state: &TickState, settlement: SettlementId, handle: &str) -> bool {
    let Some(record) = state
        .plugins
        .get_domain(OWNER, &ledger::structure_key(handle))
    else {
        return false;
    };
    if record.get("settlement").and_then(serde_json::Value::as_u64) != Some(settlement.0) {
        return false;
    }
    let Some(snapshot) = record.get("snapshot") else {
        return false;
    };
    let Some(index) = snapshot
        .get("current_stage_index")
        .and_then(serde_json::Value::as_u64)
        .and_then(|index| usize::try_from(index).ok())
    else {
        return false;
    };
    snapshot
        .get("stages")
        .and_then(|stages| stages.get(index))
        .and_then(|stage| stage.get("stage"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(PortionKind::is_preparation)
}

pub(crate) fn advance(
    state: &mut TickState,
    settlement: SettlementId,
    handle: &str,
    builder: uuid::Uuid,
) -> Result<bool, Failure> {
    if crate::settlements::economy::worker_reserved(state, Some(settlement), builder) {
        return Err(Failure::Busy);
    }
    let snapshot = status(state, settlement, handle)?;
    let stage = snapshot
        .stages
        .get(snapshot.current_stage_index as usize)
        .ok_or(Failure::Blocked)?;
    let terminal = snapshot.current_stage_index as usize + 1 == snapshot.stages.len()
        && snapshot.completed_work_units + 1 == stage.work_units;
    let (_, result) = execute_operation(
        state,
        &owner(settlement),
        &ScriptWorldStructureOperation::AdvanceStructure {
            operation_id: format!("settlement-build-{}", snapshot.revision),
            structure_id: handle.into(),
            stage: stage.stage.clone(),
            reservation_ref: builder.to_string(),
            expected_revision: snapshot.revision,
            work_units: 1,
        },
    )?;
    Ok(terminal && matches!(result, ScriptWorldStructureResult::Receipt { .. }))
}

pub(super) fn authorized(state: &TickState, record: &Structure) -> bool {
    record.settlement.is_some_and(|id| {
        record.owner == owner(id)
            && record.snapshot.dimension == state.dimension.name()
            && state.settlements.get(id).is_some_and(|settlement| {
                crate::settlements::construction::project_for_structure(
                    settlement,
                    &record.snapshot.structure_id,
                )
                .is_some()
            })
    })
}

pub(crate) fn source_images(state: &TickState, id: SettlementId) -> Vec<inventory::Image> {
    let mut images: Vec<inventory::Image> = Vec::new();
    for position in crate::settlements::construction::sources(state, id)
        .into_iter()
        .take(64)
    {
        let endpoint = inventory::Endpoint::Container {
            dimension: state.dimension.name().into(),
            position,
        };
        let Ok(image) = inventory::load(state, &endpoint) else {
            continue;
        };
        // Fences contain physical identity, including both large-chest halves.
        if !images.iter().any(|before| before.fence == image.fence) {
            images.push(image);
        }
    }
    images
}

pub(super) fn debit(
    state: &mut TickState,
    id: SettlementId,
    handle: &str,
    builder: &str,
    required: &BTreeMap<String, u64>,
    cells: &[super::Cell],
    expected: &[u32],
) -> Result<(), Failure> {
    if cells.len() != expected.len() {
        return Err(Failure::InvalidRequest);
    }
    let uuid = uuid::Uuid::parse_str(builder).map_err(|_| Failure::InvalidRequest)?;
    if crate::settlements::economy::worker_reserved(state, Some(id), uuid) {
        return Err(Failure::Busy);
    }
    let slot = state.entities.slot_of_uuid(uuid).ok_or(Failure::NotFound)?;
    if !state.entities.row(slot).villager.as_ref().is_some_and(|v| {
        v.settlement == Some(id)
            && v.data.profession == crate::villager::VillagerProfession::Builder
    }) {
        return Err(Failure::Forbidden);
    }
    // The handle names the project this step belongs to, and only the builder
    // bound to it may pay for its cells (S2). A step for another project's
    // handle is refused rather than run as an unowned advance, which would move
    // blocks without the clear-stage drop credit.
    let project = state
        .settlements
        .get(id)
        .and_then(|record| crate::settlements::construction::project_for_structure(record, handle))
        .filter(|project| project.builder.map(|(bound, _)| bound) == Some(uuid))
        .ok_or(Failure::Forbidden)?;
    // A solid cell may not be placed through another project's builder: that
    // body is not this site's to trap (D8 review SOL-4). The body is a box, not
    // a column: at 0.6 wide it straddles a cell edge and at 1.95 tall it owns
    // the cell above, so every non-air cell is tested against the whole box
    // (D8 review R3-3).
    let elsewhere: Vec<uuid::Uuid> = state
        .settlements
        .get(id)
        .map(|record| {
            crate::settlements::construction::projects(record)
                .filter(|other| other.structure != handle)
                .filter_map(|other| other.builder.map(|(bound, _)| bound))
                .collect()
        })
        .unwrap_or_default();
    for cell in cells
        .iter()
        .filter(|cell| cell.state != state.registry.states.air)
    {
        for resident in crate::settlements::construction::residents(state, id) {
            if !elsewhere.contains(&state.entities.row(resident).uuid) {
                continue;
            }
            let body = state.entities.body(resident);
            if body.height <= 0.0 || body.half_width <= 0.0 {
                continue;
            }
            let pos = state.entities.pos(resident);
            let lo = [pos[0] - body.half_width, pos[1], pos[2] - body.half_width];
            let hi = [
                pos[0] + body.half_width,
                pos[1] + body.height,
                pos[2] + body.half_width,
            ];
            let intersects = (0..3).all(|axis| {
                let min = f64::from(cell.at[axis]);
                lo[axis] < min + 1.0 && hi[axis] > min
            });
            if intersects {
                return Err(Failure::Blocked);
            }
        }
    }
    let lot = crate::settlements::forestry::lot_bounds(state, project);
    let mut drops = Vec::new();
    let mut loot = Vec::new();
    let mut clearance = crate::settlements::forestry::Clearance::prepared();
    for (cell, expected) in cells.iter().zip(expected) {
        let block = state
            .world
            .block_at(cell.at[0], cell.at[1], cell.at[2])
            .ok_or(Failure::Unloaded)?;
        if soil_work_noop(state, cell, block) {
            continue;
        }
        // D10 source fills lead cuts; remaining lava must drain before a write.
        if let Some(wait) = crate::settlements::construction::preparation::liquid::write_wait(
            state, cell.at, cell.state,
        ) {
            if wait == "lava_draining" {
                crate::settlements::construction::preparation::liquid::wake(state, cell.at);
            }
            return Err(Failure::Blocked);
        }
        // Paving and field retirement reshape existing soil without mining it.
        if free_soil_work(state, cell) {
            continue;
        }
        if let Some(credit) = portions::village_drops(state, id, cell, *expected)? {
            drops.extend(credit);
            continue;
        }
        if cell.state != state.registry.states.air {
            continue;
        }
        if crate::settlements::forestry::tree_or_sapling(state, block) {
            drops.extend(if portions::inside(state, id, cell.at) {
                crate::settlements::forestry::roll(state, cell.at, block)?
            } else {
                crate::settlements::forestry::drops(state, cell.at, block, lot, &mut clearance)?
            });
        } else if crate::settlements::construction::excavatable(state, block) {
            // Bounded terrain cut: recheck the cell the survey certified
            // immediately before the debit, credit its own mining drop in
            // this same transaction, and never write fluid or lava in.
            if !portions::inside(state, id, cell.at) && state.block_log.player_touched(cell.at) {
                return Err(Failure::Blocked);
            }
            let path = state
                .registry
                .blocks
                .by_id(mc_world::BlockStateId(block))
                .map(|definition| definition.block.id.path())
                .ok_or(Failure::Blocked)?;
            let item =
                crate::villager::mining::item_id(state, crate::villager::mining::drop_name(path))
                    .ok_or(Failure::Blocked)?;
            let count = crate::villager::mining::drop_count(path);
            for (dx, dy, dz) in [
                (1, 0, 0),
                (-1, 0, 0),
                (0, 1, 0),
                (0, -1, 0),
                (0, 0, 1),
                (0, 0, -1),
            ] {
                let at = [cell.at[0] + dx, cell.at[1] + dy, cell.at[2] + dz];
                let neighbour = state
                    .world
                    .block_at(at[0], at[1], at[2])
                    .ok_or(Failure::Unloaded)?;
                // Lava or water next to the cell would flow into the cut.
                if state.registry.states.lava_levels.contains(&neighbour)
                    || state
                        .registry
                        .blocks
                        .by_id(mc_world::BlockStateId(neighbour))
                        .is_some_and(|definition| definition.has_fluid())
                {
                    return Err(Failure::Blocked);
                }
            }
            drops.push(mc_data::ItemStack::new(item, count));
        } else if portions::inside(state, id, cell.at)
            && !crate::settlements::construction::clearable(state, block)
            && !crate::settlements::construction::relayout::crop(state, block)
        {
            drops.extend(portions::conserved_drops(state, cell.at, block)?);
        } else {
            // A log-tag variant excluded from tree ownership must never
            // pass through the flora path with an empty drop list.
            if state.registry.tree_logs.get(block as usize) == Some(&true) {
                return Err(Failure::Blocked);
            }
            loot.extend(
                crate::settlements::forestry::plant_drops(state, cell.at, block)?
                    .into_iter()
                    .map(|stack| (cell.at, stack)),
            );
            loot.extend(
                crate::settlements::construction::relayout::crop_drops(state, block)
                    .into_iter()
                    .map(|stack| (cell.at, stack)),
            );
        }
    }
    if required.is_empty() && drops.is_empty() && loot.is_empty() {
        retire_cleared_roots(state, cells);
        return Ok(());
    }
    let before = inventory::load(state, &inventory::Endpoint::VillagerCarry { uuid })?;
    let mut after = before.slots.clone();
    for (resource, quantity) in required {
        let mut remaining = *quantity;
        for stack in &mut after {
            if stack.count > 0
                && state
                    .registry
                    .items
                    .name_of(stack.item_id)
                    .is_some_and(|name| name.as_str() == resource)
            {
                let count = remaining.min(stack.count as u64);
                remaining -= count;
                stack.count -= count as i32;
                if stack.count == 0 {
                    *stack = mc_data::ItemStack::EMPTY;
                }
            }
        }
        if remaining != 0 {
            return Err(Failure::InsufficientItems);
        }
    }
    let mut changes = Vec::new();
    if before.slots != after {
        changes.push(inventory::Change { before, after });
    }
    let mut spilled = Vec::new();
    if !drops.is_empty() || !loot.is_empty() {
        let mut images = source_images(state, id);
        // A receiver must remain physical after this step, including both
        // halves of a paired chest. Fetching sources are intentionally unchanged.
        let modified = |at: [i32; 3]| {
            cells.iter().any(|cell| {
                cell.at == at
                    && state
                        .world
                        .block_at(at[0], at[1], at[2])
                        .is_none_or(|now| now != cell.state && !soil_work_noop(state, cell, now))
            })
        };
        images.retain(|image| {
            let inventory::Endpoint::Container { position, .. } = &image.endpoint else {
                return false;
            };
            !modified(*position)
                && !crate::chests::storage_partner(
                    &state.registry.blocks,
                    crate::boundary::BlockPos {
                        x: position[0],
                        y: position[1],
                        z: position[2],
                    },
                    &|at| state.world.block_at(at.x, at.y, at.z),
                )
                .is_some_and(|(_, at)| modified([at.x, at.y, at.z]))
        });
        let mut post: Vec<_> = images.iter().map(|image| image.slots.clone()).collect();
        for stack in drops {
            if !crate::settlements::construction_crafting::add_stack(state, &mut post, &stack) {
                return Err(Failure::InsufficientItems);
            }
        }
        // Plant loot is new, not conserved stock: what storage cannot hold
        // drops at the cleared cell as in vanilla, rather than stalling the
        // clear (and the only project slot) on a full chest.
        for (at, stack) in loot {
            let mut fitted = post.clone();
            if crate::settlements::construction_crafting::add_stack(state, &mut fitted, &stack) {
                post = fitted;
            } else {
                spilled.push((at, stack));
            }
        }
        changes.extend(images.into_iter().zip(post).filter_map(|(before, after)| {
            (before.slots != after).then_some(inventory::Change { before, after })
        }));
    }
    // Loot credit and material debit precede the infallible native block edit
    // in the same tick transaction. Full storage leaves the tree untouched.
    inventory::commit(state, changes)?;
    retire_cleared_roots(state, cells);
    for (at, stack) in spilled {
        crate::tick::spawn_item_entity(
            state,
            [
                f64::from(at[0]) + 0.5,
                f64::from(at[1]) + 0.5,
                f64::from(at[2]) + 0.5,
            ],
            stack,
        );
    }
    Ok(())
}

fn retire_cleared_roots(state: &mut TickState, cells: &[super::Cell]) {
    let air = state.registry.states.air;
    for cell in cells.iter().filter(|cell| cell.state == air) {
        loop {
            let owner = state
                .settlements
                .records()
                .find(|record| record.planted_trees.contains(&cell.at))
                .map(|record| record.id);
            let Some(id) = owner else { break };
            let record = state
                .settlements
                .get_mut(id)
                .expect("registered planting owner");
            record.planted_trees.remove(&cell.at);
            record.revision = record.revision.saturating_add(1);
        }
    }
}

pub(crate) fn missing(
    state: &TickState,
    id: SettlementId,
    handle: &str,
) -> Result<BTreeMap<String, u64>, Failure> {
    let snapshot = status(state, id, handle)?;
    let mut required: BTreeMap<String, u64> = snapshot
        .remaining
        .iter()
        .map(|material| (material.resource.clone(), material.quantity))
        .collect();
    // Only the resident bound to this project carries its materials: another
    // project's load never pays for these cells (D8 review SOL-5).
    let bound = state
        .settlements
        .get(id)
        .and_then(|record| crate::settlements::construction::project_for_structure(record, handle))
        .and_then(|project| project.builder)
        .map(|(uuid, _)| uuid);
    if let Some(uuid) = bound
        && let Some(slot) = state.entities.slot_of_uuid(uuid)
        && let Some(v) = state.entities.row(slot).villager.as_ref()
        && v.settlement == Some(id)
    {
        for stack in v.inventory.iter().flatten().filter(|stack| stack.count > 0) {
            if let Some(name) = state.registry.items.name_of(stack.item_id)
                && let Some(count) = required.get_mut(name.as_str())
            {
                *count = count.saturating_sub(stack.count as u64);
            }
        }
    }
    required.retain(|_, count| *count > 0);
    missing_stock(state, id, &required)
}

pub(super) fn remaining(record: &Structure) -> Vec<ScriptStructureMaterial> {
    let mut counts = BTreeMap::new();
    let current = record.snapshot.current_stage_index as usize;
    for (index, cells) in record.cells.iter().enumerate().skip(current) {
        let offset = if index == current {
            record.snapshot.completed_work_units as usize
        } else {
            0
        };
        for cell in cells.iter().skip(offset) {
            if let Some(resource) = &cell.resource {
                *counts.entry(resource.clone()).or_insert(0) += u64::from(cell.quantity);
            }
        }
    }
    materials(&counts)
}

pub(crate) fn missing_stock(
    state: &TickState,
    id: SettlementId,
    required: &BTreeMap<String, u64>,
) -> Result<BTreeMap<String, u64>, Failure> {
    let items = required
        .iter()
        .map(|(name, count)| {
            let name = mc_data::Identifier::parse(name).map_err(|_| Failure::InvalidRequest)?;
            let item = state
                .registry
                .items
                .id_of(&name)
                .ok_or(Failure::InvalidRequest)?;
            Ok((item, u32::try_from(*count).map_err(|_| Failure::Capacity)?))
        })
        .collect::<Result<Vec<_>, Failure>>()?;
    let images = source_images(state, id);
    if crate::settlements::construction_crafting::can_fund(state, &items, &images) {
        return Ok(BTreeMap::new());
    }
    Ok(
        crate::settlements::construction_crafting::missing_base(state, &items, &images)
            .into_iter()
            .filter_map(|(item, count)| {
                state
                    .registry
                    .items
                    .name_of(item)
                    .map(|name| (name.to_string(), u64::from(count)))
            })
            .collect(),
    )
}

/// Foundation cells extend the bottom of the ledger box, never its top.
/// Recover the exact authored floor from that top and the future footprint;
/// do not treat arbitrary vertically overlapping buildings as the same site.
/// The site a structure reserves, without its wide clear-stage window.
fn site_reservation(state: &TickState, record: &Structure) -> Result<ScriptWorldBounds, Failure> {
    let site = bounds(record.snapshot.origin, record.snapshot.reserved_footprint)?;
    // An unfinished structure holds the whole detour it must lay before it
    // retires the old line: a parallel lot may not take that corridor (P2-5).
    let paved = crate::settlements::layout::paved_state(state);
    let mut extent: Option<([i32; 3], [i32; 3])> = None;
    for cell in record.cells.iter().flatten() {
        if Some(cell.state) != paved {
            continue;
        }
        let (mut min, mut max) = extent.unwrap_or((cell.at, cell.at));
        for axis in 0..3 {
            min[axis] = min[axis].min(cell.at[axis]);
            max[axis] = max[axis].max(cell.at[axis]);
        }
        extent = Some((min, max));
    }
    let Some((min, max)) = extent else {
        return Ok(site);
    };
    let min = std::array::from_fn(|axis| site.min[axis].min(min[axis]));
    let max = std::array::from_fn(|axis| site.max[axis].max(max[axis]));
    ScriptWorldBounds::new(min, max).map_err(|_| Failure::InvalidRequest)
}

/// The authored site box of one active project: the canonical reservation the
/// structure was prepared with (origin plus reserved footprint). It exists for
/// every project kind, including the infrastructure blueprints that are not in
/// the catalog and so have no blueprint footprint (D8 review S2-2).
pub(crate) fn site_bounds(
    state: &TickState,
    settlement: SettlementId,
    handle: &str,
) -> Option<ScriptWorldBounds> {
    let ledger = ledger(state).ok()?;
    let record = ledger.structures().get(handle)?;
    if record.settlement != Some(settlement) {
        return None;
    }
    site_reservation(state, record).ok()
}

/// Whether a body standing with its feet at `pos` (half width, height) would
/// reach another active project's authored site, in another dimension or under
/// another owner excluded by construction. The test is against the canonical
/// site reservation, not the clear/felling envelope, and not a blueprint
/// footprint: a road, lamp, meadow, perimeter or standalone furnace has no
/// catalog footprint and must protect its site all the same (D8 review S2-2).
pub(crate) fn body_in_other_site(
    state: &TickState,
    settlement: SettlementId,
    except: &str,
    pos: [f64; 3],
    half_width: f64,
    height: f64,
) -> bool {
    if half_width <= 0.0 || height <= 0.0 {
        return false;
    }
    let lo = [pos[0] - half_width, pos[1], pos[2] - half_width];
    let hi = [pos[0] + half_width, pos[1] + height, pos[2] + half_width];
    crate::settlements::construction::projects_of(&state.settlements, settlement)
        .filter(|other| other.structure != except)
        .filter_map(|other| site_bounds(state, other.settlement, &other.structure))
        .any(|site| {
            (0..3).all(|axis| {
                lo[axis] < f64::from(site.max[axis]) + 1.0 && hi[axis] > f64::from(site.min[axis])
            })
        })
}

fn same_site(state: &TickState, record: &Structure, origin: [i32; 3]) -> bool {
    let Some(blueprint) = record
        .snapshot
        .blueprint_id
        .strip_prefix("krasnokamen-settlements:")
        .and_then(|id| crate::settlements::blueprints::get(&state.registry, id))
    else {
        return false;
    };
    let size = blueprint.footprint(crate::settlements::blueprints::Rotation::from_quarter_turns(0));
    bounds(record.snapshot.origin, record.snapshot.reserved_footprint).is_ok_and(|bounds| {
        origin
            == [
                bounds.min[0],
                bounds.max[1] - i32::from(size[1]) + 1,
                bounds.min[2],
            ]
    })
}

/// Resolve a town's workstation reference without decoding the world's
/// structure ledger on every villager admission. The borrowed JSON is the
/// canonical record itself; the settlement handle grants no custody alone.
pub(crate) fn furnace_workstation(
    state: &TickState,
    id: SettlementId,
    handle: &str,
) -> Option<[i32; 3]> {
    let record = state
        .plugins
        .get_domain(OWNER, &ledger::structure_key(handle))?;
    if record.get("settlement")?.as_u64()? != id.0 {
        return None;
    }
    let snapshot = record.get("snapshot")?;
    if snapshot.get("dimension")?.as_str()? != state.dimension.name()
        || snapshot.get("blueprint_id")?.as_str()? != "krasnokamen-settlements:furnace"
        || ScriptStructureState::deserialize(snapshot.get("state")?).ok()?
            != ScriptStructureState::Committed
        || <[i32; 3]>::deserialize(snapshot.get("reserved_footprint")?).ok()? != [1, 1, 1]
    {
        return None;
    }
    <[i32; 3]>::deserialize(snapshot.get("origin")?).ok()
}

pub(crate) fn built_at(
    state: &TickState,
    id: SettlementId,
    blueprint: &str,
    origin: [i32; 3],
) -> bool {
    ledger(state).is_ok_and(|ledger| {
        ledger.structures().values().any(|record| {
            record.settlement == Some(id)
                && record
                    .snapshot
                    .blueprint_id
                    .strip_prefix("krasnokamen-settlements:")
                    == Some(blueprint)
                && same_site(state, record, origin)
                && record.snapshot.state == ScriptStructureState::Committed
        })
    })
}

/// Upgrade tree cells follow the same fence as a new site: a player-touched or
/// persistent cell, unknown generator evidence, a protected root halo or a live
/// player with capture disabled refuses; a previous committed building's exact
/// authored state is a beam, not forest; every other tree or sapling cell becomes
/// clear-stage work, so the executor cuts it with conserved drops.
pub(crate) fn upgrade_trees_clear(
    state: &TickState,
    id: SettlementId,
    blueprint: &str,
    origin: [i32; 3],
    cells: &[PlannedCell],
    lot: Option<mc_script::ScriptWorldBounds>,
    clearance: &mut crate::settlements::forestry::Clearance,
) -> Result<Vec<PlannedCell>, Failure> {
    let ledger = ledger(state)?;
    let previous = ledger
        .structures()
        .values()
        .filter(|record| {
            record.settlement == Some(id)
                && record
                    .snapshot
                    .blueprint_id
                    .strip_prefix("krasnokamen-settlements:")
                    == Some(blueprint)
                && same_site(state, record, origin)
                && record.snapshot.state == ScriptStructureState::Committed
        })
        .max_by_key(|record| record.snapshot.revision);
    let mut cleared = Vec::new();
    for cell in cells {
        let block = state
            .world
            .physics_block_at(cell.at[0], cell.at[1], cell.at[2])
            .ok_or(Failure::Unloaded)?;
        if !crate::settlements::forestry::tree_or_sapling(state, block) {
            continue;
        }
        // Attribution and the disabled-history/live-player veto precede even
        // an exact match with a building's own previous authored beam.
        if crate::settlements::forestry::vetoed(state, cell.at) {
            return Err(Failure::Forbidden);
        }
        if crate::settlements::forestry::persistent(state, block) {
            return Err(Failure::Blocked);
        }
        if previous.is_some_and(|record| {
            record
                .cells
                .iter()
                .flatten()
                .any(|authored| authored.at == cell.at && authored.state == block)
        }) {
            // The building's own authored beam; the upgrade replaces it.
            continue;
        }
        if !clearance.clearable(state, cell.at, lot) {
            return Err(Failure::Blocked);
        }
        cleared.push(PlannedCell {
            portion: if state
                .settlements
                .records()
                .any(|record| record.planted_trees.contains(&cell.at))
                && state
                    .registry
                    .blocks
                    .by_id(mc_world::BlockStateId(block))
                    .is_some_and(|definition| definition.block.id.path().ends_with("_sapling"))
            {
                PortionKind::Saplings
            } else {
                PortionKind::Trees
            },
            at: cell.at,
            state: state.registry.states.air,
            resource: None,
            quantity: 0,
        });
    }
    Ok(cleared)
}

pub(crate) fn next_material(
    state: &TickState,
    id: SettlementId,
    handle: &str,
) -> Result<Option<(u32, u32)>, Failure> {
    let ledger = ledger(state)?;
    let record = structure(&ledger, &owner(id), handle)?;
    let Some(cell) = record
        .cells
        .get(record.snapshot.current_stage_index as usize)
        .and_then(|cells| cells.get(record.snapshot.completed_work_units as usize))
    else {
        return Ok(None);
    };
    let Some(name) = cell.resource.as_ref() else {
        return Ok(None);
    };
    let item = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse(name).map_err(|_| Failure::InvalidRequest)?)
        .ok_or(Failure::InvalidRequest)?;
    Ok(Some((item, cell.quantity)))
}

/// Materials for the next cells of the current stage, in first-need order,
/// that carry cannot yet cover, packed into the carry's slot capacity. The
/// first uncovered cell is always planned; later cells join while every
/// distinct item still fits in stacks of the villager's real inventory.
fn haul_plan(
    state: &TickState,
    record: &Structure,
    carry: &[mc_data::ItemStack],
) -> Vec<(u32, u32)> {
    let rules = state.registry.item_rules();
    let stack_limit = |item: u32| rules.max_stack(&mc_data::ItemStack::new(item, 1)).max(1) as u32;
    let mut carried: BTreeMap<u32, u32> = BTreeMap::new();
    for stack in carry.iter().filter(|stack| stack.count > 0) {
        *carried.entry(stack.item_id).or_default() += stack.count as u32;
    }
    let slots = |totals: &BTreeMap<u32, u32>| -> usize {
        totals
            .iter()
            .map(|(item, total)| total.div_ceil(stack_limit(*item)) as usize)
            .sum()
    };
    // Per-item quantity the window needs beyond what is already carried.
    let mut totals = carried.clone();
    let mut need: BTreeMap<u32, u32> = BTreeMap::new();
    let mut order: Vec<u32> = Vec::new();
    let mut covered: BTreeMap<u32, u32> = carried;
    let Some(cells) = record
        .cells
        .get(record.snapshot.current_stage_index as usize)
    else {
        return Vec::new();
    };
    for cell in cells
        .iter()
        .skip(record.snapshot.completed_work_units as usize)
    {
        let Some(name) = cell.resource.as_ref() else {
            continue;
        };
        let Some(item) = mc_data::Identifier::parse(name)
            .ok()
            .and_then(|id| state.registry.items.id_of(&id))
        else {
            break;
        };
        let quantity = cell.quantity;
        let have = covered.entry(item).or_default();
        let from_carry = (*have).min(quantity);
        let missing = quantity - from_carry;
        if missing > 0 {
            let mut trial = totals.clone();
            *trial.entry(item).or_default() += missing;
            if !order.is_empty() && slots(&trial) > carry.len() {
                break;
            }
            totals = trial;
            if !need.contains_key(&item) {
                order.push(item);
            }
            *need.entry(item).or_default() += missing;
        }
        *have -= from_carry;
    }
    order.into_iter().map(|item| (item, need[&item])).collect()
}

/// A transfer, not a debit: real items enter resident carry and are debited
/// only by the shared AdvanceStructure transaction alongside the block write.
/// One trip loads the next cells' materials across several item types, up to
/// the carry's slot capacity; `item` (the next cell's material) is mandatory.
pub(crate) fn fetch(
    state: &mut TickState,
    uuid: uuid::Uuid,
    handle: &str,
    position: [i32; 3],
    item: u32,
) -> Result<(), Failure> {
    let slot = state.entities.slot_of_uuid(uuid).ok_or(Failure::NotFound)?;
    let id = state
        .entities
        .row(slot)
        .villager
        .as_ref()
        .and_then(|v| v.settlement)
        .ok_or(Failure::Forbidden)?;
    if crate::settlements::economy::worker_reserved(state, Some(id), uuid) {
        return Err(Failure::Busy);
    }
    let carry = inventory::load(state, &inventory::Endpoint::VillagerCarry { uuid })?;
    let (plan, mut remaining_bill) = {
        let ledger = ledger(state)?;
        // Only the builder bound to this project hauls for it, and only against
        // that project's remaining bill (S1b/S2: carry is per project).
        let project = state
            .settlements
            .get(id)
            .and_then(|record| {
                crate::settlements::construction::project_for_structure(record, handle)
            })
            .filter(|project| project.builder.map(|(bound, _)| bound) == Some(uuid))
            .ok_or(Failure::Forbidden)?;
        let record = structure(&ledger, &owner(id), &project.structure)?;
        let mut bill = remaining(record)
            .into_iter()
            .map(|material| {
                let name = mc_data::Identifier::parse(&material.resource)
                    .map_err(|_| Failure::InvalidRequest)?;
                let item = state
                    .registry
                    .items
                    .id_of(&name)
                    .ok_or(Failure::InvalidRequest)?;
                Ok((
                    item,
                    u32::try_from(material.quantity).map_err(|_| Failure::Capacity)?,
                ))
            })
            .collect::<Result<Vec<_>, Failure>>()?;
        for stack in carry.slots.iter().filter(|stack| stack.count > 0) {
            if let Some((_, count)) = bill.iter_mut().find(|(item, _)| *item == stack.item_id) {
                *count = count.saturating_sub(stack.count as u32);
            }
        }
        (haul_plan(state, record, &carry.slots), bill)
    };
    if !plan.iter().any(|(planned, _)| *planned == item) {
        return Err(Failure::Blocked);
    }
    let images = source_images(state, id);
    if !images.iter().any(|image| {
        matches!(&image.endpoint,
        inventory::Endpoint::Container { position: point, .. } if *point == position)
    }) {
        return Err(Failure::Forbidden);
    }
    let mut stock_after: Vec<Vec<mc_data::ItemStack>> =
        images.iter().map(|image| image.slots.clone()).collect();
    let mut carry_after = carry.slots.clone();
    let rules = state.registry.item_rules();
    // The rest of the settlement's active bills are not this haul's to spend
    // (D8 review SOL-3).
    let Some(neighbours) = crate::settlements::construction::other_active_bill(state, id, handle)
    else {
        // Another project's bill cannot be read, so its materials cannot be
        // reserved: refuse the haul instead of spending them (SOL-3b).
        return Err(Failure::Blocked);
    };
    for (planned, wanted) in plan {
        // Craft the whole haul; on a shortfall settle for what stock can give.
        let mut amount = wanted;
        let mut crafted = None;
        while amount > 0 {
            let mut future = remaining_bill.clone();
            if let Some((_, count)) = future.iter_mut().find(|(id, _)| *id == planned) {
                *count = count.saturating_sub(amount);
            }
            for (item, count) in &neighbours {
                match future.iter_mut().find(|(known, _)| known == item) {
                    Some((_, sum)) => *sum = sum.saturating_add(*count),
                    None => future.push((*item, *count)),
                }
            }
            let specific = crate::settlements::construction_crafting::wood_specific(state, planned);
            let reserved = if specific {
                Default::default()
            } else if let Ok(reserved) = crate::settlements::construction_crafting::reserve_wood(
                state,
                &future,
                &stock_after,
            ) {
                reserved
            } else {
                amount /= 2;
                continue;
            };
            let mut trial = stock_after.clone();
            if crate::settlements::construction_crafting::ensure_units(
                state,
                &mut trial,
                planned,
                amount as i32,
                &reserved,
            ) {
                let reserved = if specific {
                    // A recipe batch can also supply future cells. Check the
                    // actual post-haul stock instead of reserving another
                    // whole batch before crafting the current cell.
                    let mut after_haul = trial.clone();
                    let mut remove = amount as i32;
                    for stack in after_haul
                        .iter_mut()
                        .flatten()
                        .filter(|stack| stack.item_id == planned && stack.count > 0)
                    {
                        let take = remove.min(stack.count);
                        stack.count -= take;
                        remove -= take;
                        if stack.count == 0 {
                            *stack = mc_data::ItemStack::EMPTY;
                        }
                        if remove == 0 {
                            break;
                        }
                    }
                    match crate::settlements::construction_crafting::reserve_wood(
                        state,
                        &future,
                        &after_haul,
                    ) {
                        Ok(reserved) => reserved,
                        Err(_) => {
                            amount /= 2;
                            continue;
                        }
                    }
                } else {
                    reserved
                };
                crafted = Some((trial, reserved));
                break;
            }
            amount /= 2;
        }
        let Some((trial, reserved)) = crafted else {
            if planned == item {
                return Err(Failure::InsufficientItems);
            }
            continue;
        };
        stock_after = trial;
        let mut available = stock_after
            .iter()
            .flatten()
            .filter(|stack| stack.item_id == planned && stack.count > 0)
            .map(|stack| stack.count)
            .sum::<i32>()
            .saturating_sub(reserved.get(&planned).copied().unwrap_or(0))
            .max(0);
        let mut moved = 0;
        for source in stock_after
            .iter_mut()
            .flatten()
            .filter(|stack| stack.item_id == planned && stack.count > 0)
        {
            let maximum = rules.max_stack(source);
            while moved < amount as i32 && source.count > 0 && available > 0 {
                let destination = carry_after
                    .iter()
                    .position(|stack| {
                        !stack.is_empty()
                            && mc_data::inventory_semantics_26_1_2::can_stack(stack, source)
                            && stack.count < maximum
                    })
                    .or_else(|| carry_after.iter().position(mc_data::ItemStack::is_empty));
                let Some(destination) = destination else {
                    break;
                };
                let destination = &mut carry_after[destination];
                if destination.is_empty() {
                    *destination = source.clone();
                    destination.count = 0;
                }
                let take = (amount as i32 - moved)
                    .min(source.count)
                    .min(maximum - destination.count)
                    .min(available);
                if take <= 0 {
                    break;
                }
                destination.count += take;
                source.count -= take;
                moved += take;
                available -= take;
            }
            if source.count == 0 {
                *source = mc_data::ItemStack::EMPTY;
            }
        }
        if moved == 0 && planned == item {
            return Err(Failure::Capacity);
        }
        if let Some((_, count)) = remaining_bill.iter_mut().find(|(id, _)| *id == planned) {
            *count = count.saturating_sub(moved as u32);
        }
    }
    let changes = images
        .into_iter()
        .zip(stock_after)
        .filter_map(|(before, after)| {
            (before.slots != after).then_some(inventory::Change { before, after })
        })
        .chain(std::iter::once(inventory::Change {
            before: carry,
            after: carry_after,
        }))
        .collect();
    inventory::commit(state, changes)?;
    Ok(())
}

/// What one addressable carry return achieved (D8 review SOL-6/R3-1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CarryReturn {
    /// The load reached storage, or the body carried nothing.
    Delivered,
    /// Storage could not take it all yet: a later pass retries.
    Pending,
    /// The body cannot be addressed right now (dead, converting, gone). It
    /// never holds a binding, and its load is picked up by the planner pass
    /// once the body is addressable again.
    Unreachable,
}

/// Deposit at most `owed` of these items from one body into settlement storage
/// and return what is still owed. Everything the body carries beyond the owed
/// amounts stays with it, so a resident that later picks up its own food or a
/// new working load never hands that over (D8 review R4-1).
pub(crate) fn return_owed_carry(
    state: &mut TickState,
    id: SettlementId,
    uuid: uuid::Uuid,
    owed: &[(u32, u32)],
) -> Option<Vec<(u32, u32)>> {
    if crate::settlements::economy::worker_reserved(state, Some(id), uuid) {
        return Some(owed.to_vec());
    }
    let Ok(carry) = inventory::load(state, &inventory::Endpoint::VillagerCarry { uuid }) else {
        // Not addressable right now (infected, converting, gone): the debt
        // stays until the body is a villager again.
        return if state.entities.slot_of_uuid(uuid).is_some() {
            Some(owed.to_vec())
        } else {
            None
        };
    };
    let images = source_images(state, id);
    let mut post: Vec<Vec<mc_data::ItemStack>> =
        images.iter().map(|image| image.slots.clone()).collect();
    let mut after = carry.slots.clone();
    let mut residual: Vec<(u32, u32)> = Vec::new();
    for &(item, count) in owed {
        let mut left = count;
        for stack in after
            .iter_mut()
            .filter(|stack| !stack.is_empty() && stack.item_id == item)
        {
            if left == 0 {
                break;
            }
            let take = left.min(stack.count as u32);
            let mut trial = post.clone();
            let mut piece = mc_data::ItemStack::new(item, take as i32);
            if !crate::settlements::construction_crafting::add_stack(state, &mut trial, &piece) {
                break;
            }
            post = trial;
            stack.count -= take as i32;
            if stack.count == 0 {
                *stack = mc_data::ItemStack::EMPTY;
            }
            left -= take;
            piece.count = 0;
        }
        if left > 0 {
            residual.push((item, left));
        }
    }
    let mut changes: Vec<_> = images
        .into_iter()
        .zip(post)
        .filter_map(|(before, after)| {
            (before.slots != after).then_some(inventory::Change { before, after })
        })
        .collect();
    if changes.is_empty() {
        return Some(residual);
    }
    changes.push(inventory::Change {
        before: carry,
        after,
    });
    if inventory::commit(state, changes).is_err() {
        // A refused commit keeps the whole debt: nothing moved.
        return Some(owed.to_vec());
    }
    if residual.is_empty() {
        Some(Vec::new())
    } else {
        Some(residual)
    }
}

/// Return one builder's load to settlement storage. Release, rebind and
/// stalled-builder recovery address a single project's builder, never the
/// whole settlement.
pub(crate) fn return_carry_of(
    state: &mut TickState,
    id: SettlementId,
    uuid: uuid::Uuid,
) -> CarryReturn {
    if crate::settlements::economy::worker_reserved(state, Some(id), uuid) {
        return CarryReturn::Pending;
    }
    let Ok(carry) = inventory::load(state, &inventory::Endpoint::VillagerCarry { uuid }) else {
        // The row is not addressable as a live villager right now: its load
        // waits on the body for the planner pass (R4-1).
        return CarryReturn::Unreachable;
    };
    if carry.slots.iter().all(mc_data::ItemStack::is_empty) {
        return CarryReturn::Delivered;
    }
    let images = source_images(state, id);
    let mut post: Vec<Vec<mc_data::ItemStack>> =
        images.iter().map(|image| image.slots.clone()).collect();
    let mut after = carry.slots.clone();
    for stack in after.iter_mut().filter(|stack| !stack.is_empty()) {
        let mut trial = post.clone();
        if crate::settlements::construction_crafting::add_stack(state, &mut trial, stack) {
            post = trial;
            *stack = mc_data::ItemStack::EMPTY;
        }
    }
    let mut changes: Vec<_> = images
        .into_iter()
        .zip(post)
        .filter_map(|(before, after)| {
            (before.slots != after).then_some(inventory::Change { before, after })
        })
        .collect();
    let delivered = after.iter().all(mc_data::ItemStack::is_empty);
    if changes.is_empty() {
        return CarryReturn::Pending;
    }
    changes.push(inventory::Change {
        before: carry,
        after,
    });
    // A refused commit leaves the load on the body: report it as pending, not
    // as delivered (D8 review R3-2).
    if inventory::commit(state, changes).is_err() {
        return CarryReturn::Pending;
    }
    if delivered {
        CarryReturn::Delivered
    } else {
        CarryReturn::Pending
    }
}

pub(crate) fn pickup_point(
    state: &TickState,
    id: SettlementId,
    item: u32,
    origin: [f64; 3],
) -> Option<[i32; 3]> {
    let images = source_images(state, id);
    let has_item = |image: &inventory::Image| {
        image
            .slots
            .iter()
            .any(|stack| stack.item_id == item && stack.count > 0)
    };
    let exact = images.iter().any(has_item);
    if !exact && !crate::settlements::construction_crafting::can_craft(state, item, &images) {
        return None;
    }
    images
        .iter()
        .filter(|image| !exact || has_item(image))
        .filter_map(|image| {
            let inventory::Endpoint::Container { position, .. } = &image.endpoint else {
                return None;
            };
            let distance = (origin[0] - f64::from(position[0])).powi(2)
                + (origin[2] - f64::from(position[2])).powi(2);
            Some((*position, distance))
        })
        .min_by(|(_, a), (_, b)| a.total_cmp(b))
        .map(|(point, _)| point)
}

/// A durable outpost reserves its full future site before its first structure.
pub(crate) fn expedition_reserved(
    state: &TickState,
    bounds: ScriptWorldBounds,
    admission: Option<(SettlementId, &str, [i32; 3])>,
) -> bool {
    state.settlements.records().any(|town| {
        town.founding.as_ref().is_some_and(|site| {
            overlaps(site.bounds(), bounds)
                && !admission.is_some_and(|(id, name, origin)| {
                    town.id == id && site.sites().contains(&(name, origin))
                })
        })
    })
}

pub(crate) fn founding_claim_clear(
    state: &TickState,
    source: SettlementId,
    bounds: ScriptWorldBounds,
    sites: &[(&str, [i32; 3])],
) -> bool {
    let source_owner = owner(source);
    ledger(state).is_ok_and(|ledger| {
        ledger.structures().values().all(|record| {
            record.snapshot.dimension != state.dimension.name()
                || record.snapshot.state == ScriptStructureState::Cancelled
                || spent_free_work(record)
                || reserved_bounds(record).is_ok_and(|existing| !overlaps(existing, bounds))
                || (record.settlement == Some(source)
                    && record.owner == source_owner
                    && sites.iter().any(|(name, origin)| {
                        record
                            .snapshot
                            .blueprint_id
                            .strip_prefix("krasnokamen-settlements:")
                            == Some(*name)
                            && same_site(state, record, *origin)
                    }))
        })
    })
}

/// An unpublished ledger post-image, prepared before any civil record changes.
pub(crate) struct PreparedFoundingTransfer(LedgerView);

pub(crate) fn prepare_founding_transfer(
    state: &TickState,
    source: SettlementId,
    destination: SettlementId,
    sites: &[(&str, [i32; 3])],
) -> Result<PreparedFoundingTransfer, Failure> {
    if source == destination
        || state.settlements.get(destination).is_some()
        || sites.len() != 4
        || state.settlements.get(source).is_none()
    {
        return Err(Failure::InvalidRequest);
    }
    let mut ledger = ledger(state)?;
    let source_owner = owner(source);
    let matches = |record: &Structure| {
        record.settlement == Some(source)
            && record.owner == source_owner
            && record.snapshot.state == ScriptStructureState::Committed
            && sites.iter().any(|(name, origin)| {
                record
                    .snapshot
                    .blueprint_id
                    .strip_prefix("krasnokamen-settlements:")
                    == Some(*name)
                    && same_site(state, record, *origin)
            })
    };
    for (name, origin) in sites {
        if !ledger.structures().values().any(|record| {
            matches(record)
                && record.snapshot.state == ScriptStructureState::Committed
                && record
                    .snapshot
                    .blueprint_id
                    .strip_prefix("krasnokamen-settlements:")
                    == Some(*name)
                && same_site(state, record, *origin)
        }) {
            return Err(Failure::Blocked);
        }
    }
    for record in ledger
        .structures()
        .values()
        .filter(|record| matches(record))
    {
        if record.snapshot.dimension != state.dimension.name()
            || record.cells.iter().flatten().any(|cell| {
                cell.state != state.registry.states.air
                    && state
                        .world
                        .physics_block_at(cell.at[0], cell.at[1], cell.at[2])
                        .is_none_or(|actual| {
                            state
                                .registry
                                .blocks
                                .by_id(mc_world::BlockStateId(actual))
                                .map(|definition| &definition.block.id)
                                != state
                                    .registry
                                    .blocks
                                    .by_id(mc_world::BlockStateId(cell.state))
                                    .map(|definition| &definition.block.id)
                        })
            })
        {
            return Err(Failure::Blocked);
        }
    }
    let revision = next(ledger.revision)?;
    for record in ledger.structures_mut_where(|record| matches(record)) {
        // Stable opaque structure IDs remain the construction history; only
        // the one live owner/provenance changes.
        record.owner = owner(destination);
        record.settlement = Some(destination);
        record.snapshot.revision = revision;
    }
    ledger.revision = revision;
    Ok(PreparedFoundingTransfer(ledger))
}

pub(crate) fn commit_founding_transfer(state: &mut TickState, prepared: PreparedFoundingTransfer) {
    save(state, prepared.0);
}

#[cfg(test)]
#[path = "settlement_builds_preparation_tests.rs"]
mod preparation_tests;
