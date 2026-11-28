use super::builders::assign_builder;
use super::growth_tests::Warnings;
use super::tests::fixture;
use super::*;
use crate::villager::Activity;
use mc_data::ItemStack;

fn bed_project() -> (TickState, SettlementId, crate::entities::EntitySlot) {
    let (mut state, id, builder) = fixture();
    let bed = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:white_bed").unwrap())
        .unwrap();
    state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots[0] = ItemStack::new(bed, 1);
    let bounds = ScriptWorldBounds::new([4, 201, 4], [14, 209, 14]).unwrap();
    let block =
        super::super::allocator::tests::block(&state, "minecraft:white_bed", &[("part", "foot")]);
    let structure = executor::prepare(
        &mut state,
        id,
        "spanish_fortress".into(),
        bounds,
        bounds,
        vec![executor::PlannedCell {
            portion: executor::PortionKind::Build,
            at: [10, 202, 10],
            state: block,
            resource: Some("minecraft:white_bed".into()),
            quantity: 1,
        }],
    )
    .unwrap();
    state.settlements.get_mut(id).unwrap().projects = vec![Project {
        settlement: id,
        blueprint: "spanish_fortress".into(),
        level: 1,
        origin: [4, 201, 4],
        rotation: 0,
        building: None,
        structure,
        ordered: true,
        builder: Some((state.entities.row(builder).uuid, None)),
        order_seq: None,
    }];
    (state, id, builder)
}

fn log_stall(state: &mut TickState, id: SettlementId) -> String {
    let warnings = Warnings::default();
    tracing::subscriber::with_default(warnings.clone(), || {
        // Three windows distinguish the first stall, failed recovery and silence.
        for _ in 0..=450 {
            state.tick += 20;
            release_stalled_builder(state, id);
        }
    });
    warnings.0.lock().unwrap().join("\n")
}

#[test]
fn a_material_stall_reports_the_item_source_route_and_actual_builder_release() {
    let (mut state, id, builder) = bed_project();
    let uuid = state.entities.row(builder).uuid;
    let v = state.entities.row_mut(builder).villager.as_mut().unwrap();
    v.navigation_target = Some(crate::boundary::BlockPos { x: 1, y: 201, z: 1 });
    v.navigation_search_failed = true;
    state.settlements.get_mut(id).unwrap().orders = vec![OwnerOrder {
        seq: 0,
        blueprint: "spanish_house".into(),
        level: 1,
        building: None,
    }];
    let project = state.settlements.get(id).unwrap().projects[0].clone();
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    let text = log_stall(&mut state, id);
    assert_eq!(
        text.matches("settlement_construction_stalled").count(),
        1,
        "{text}"
    );
    for evidence in [
        "collecting_material",
        "minecraft:white_bed",
        "required=Some(1)",
        "carried=Some(0)",
        "pickup=Some([1, 201, 1])",
        "navigation_failed=Some(true)",
        "recovery=\"builder_released\"",
        &uuid.to_string(),
    ] {
        assert!(text.contains(evidence), "missing {evidence}: {text}");
    }
    assert_eq!(
        state
            .entities
            .row(builder)
            .villager
            .as_ref()
            .unwrap()
            .data
            .profession,
        Profession::None
    );
    // The unpaid bed never started: the failed rotation gives back its slot
    // and keeps the newer pending order, without touching stock.
    let record = state.settlements.get(id).unwrap();
    assert!(record.projects.is_empty(), "{text}");
    assert!(
        text.contains("recovery=\"preparation_cancelled\""),
        "{text}"
    );
    assert_eq!(record.orders[0].blueprint, "spanish_house");
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    assert!(
        executor::status(&state, id, &project.structure)
            .unwrap()
            .consumed
            .is_empty()
    );
}

#[test]
fn a_real_fetch_refusal_is_retained_for_the_stall_report_without_debiting_stock() {
    let (mut state, id, builder) = bed_project();
    let bread = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:bread").unwrap())
        .unwrap();
    let inventory = std::array::from_fn(|_| Some(ItemStack::new(bread, 64)));
    state
        .entities
        .row_mut(builder)
        .villager
        .as_mut()
        .unwrap()
        .inventory = inventory.clone();
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    release_stalled_builder(&mut state, id);
    crate::villager::building::tick(&mut state, id);
    let handle = state.settlements.get(id).unwrap().projects[0]
        .structure
        .clone();
    assert_eq!(
        state.settlements.builder_watch[&(id, handle)].refusal,
        Some("executor_capacity")
    );
    let text = log_stall(&mut state, id);
    assert!(
        text.contains("last_refusal=Some(\"executor_capacity\")"),
        "{text}"
    );
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    assert_eq!(
        state
            .entities
            .row(builder)
            .villager
            .as_ref()
            .unwrap()
            .inventory,
        inventory
    );
}

#[test]
fn a_repeated_stall_names_the_first_recovery_and_cell_once_without_discarding_work() {
    let (mut state, id, _) = bed_project();
    let project = state.settlements.get(id).unwrap().projects[0].clone();
    // Paid work keeps its slot through every failed recovery. A bed cell is
    // normally paid when placed; record the payment in the canonical ledger.
    let mut ledger = state
        .plugins
        .get_domain(
            "@native-structures",
            &format!("structure/{}", project.structure),
        )
        .unwrap()
        .clone();
    ledger["snapshot"]["consumed"] =
        serde_json::json!([{"resource": "minecraft:white_bed", "quantity": 1}]);
    state.plugins.set_domain(
        "@native-structures",
        &format!("structure/{}", project.structure),
        ledger,
    );
    let before = executor::status(&state, id, &project.structure).unwrap();
    assert!(!before.consumed.is_empty());
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    let text = log_stall(&mut state, id);
    assert_eq!(
        text.matches("settlement_construction_recovery_failed")
            .count(),
        1,
        "{text}"
    );
    let repeated = text
        .lines()
        .find(|line| line.contains("settlement_construction_recovery_failed"))
        .unwrap();
    for evidence in [
        "spanish_fortress",
        "next=Some([10, 202, 10])",
        "first_recovery=\"builder_released\"",
        "cause=\"no_builder\"",
        "recovery=\"no_builder\"",
    ] {
        assert!(
            repeated.contains(evidence),
            "missing {evidence}: {repeated}"
        );
    }
    // The release clears the binding; the paid project keeps its slot, site and
    // level through every failed recovery.
    let kept = state.settlements.get(id).unwrap().projects[0].clone();
    assert_eq!(kept.builder, None, "the released builder holds no project");
    assert_eq!(
        (
            kept.structure.as_str(),
            kept.blueprint.as_str(),
            kept.origin
        ),
        (
            project.structure.as_str(),
            project.blueprint.as_str(),
            project.origin
        )
    );
    assert_eq!(
        executor::status(&state, id, &project.structure).unwrap(),
        before
    );
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
}

#[test]
fn ordinary_grass_clearing_uses_work_reach_not_a_tree_felling_stand() {
    let (mut state, id, builder) = fixture();
    let next = [10, 201, 10];
    let grass = super::super::allocator::tests::block(&state, "minecraft:short_grass", &[]);
    state.world.set_block(next[0], next[1], next[2], grass);
    state.world.fold_overlay();
    let bounds = ScriptWorldBounds::new([4, 201, 4], [14, 209, 14]).unwrap();
    let air = state.registry.states.air;
    let structure = executor::prepare(
        &mut state,
        id,
        layout::MEADOW.into(),
        bounds,
        bounds,
        vec![executor::PlannedCell {
            portion: executor::PortionKind::Trees,
            at: next,
            state: air,
            resource: None,
            quantity: 0,
        }],
    )
    .unwrap();
    state.settlements.get_mut(id).unwrap().projects = vec![Project {
        settlement: id,
        blueprint: layout::MEADOW.into(),
        level: 1,
        origin: bounds.min,
        rotation: 0,
        building: None,
        structure: structure.clone(),
        ordered: false,
        builder: None,
        order_seq: None,
    }];
    state.entities.set_pos(builder, [5.5, 201.0, 10.5]);
    crate::villager::building::tick(&mut state, id);
    assert!(
        state.settlements.get(id).unwrap().projects.is_empty(),
        "a grass cell in ordinary work reach must not wait for an unrelated felling stand"
    );
    assert_eq!(
        executor::status(&state, id, &structure).unwrap().state,
        ScriptStructureState::Committed
    );
    assert_eq!(state.world.block_at(next[0], next[1], next[2]), Some(air));
}

#[test]
fn a_decayed_crown_clear_cell_keeps_its_footing_and_resumes_instead_of_cancelling() {
    let (mut state, id, builder) = fixture();
    let next = [8, 220, 8];
    let leaf = super::super::allocator::tests::block(
        &state,
        "minecraft:oak_leaves",
        &[("distance", "7"), ("persistent", "false")],
    );
    state.world.set_block(next[0], next[1], next[2], leaf);
    state.world.fold_overlay();
    let bounds = ScriptWorldBounds::new([4, 201, 4], [14, 222, 14]).unwrap();
    let air = state.registry.states.air;
    let log = super::super::allocator::tests::block(&state, "minecraft:oak_log", &[]);
    let build = [8, 202, 8];
    let structure = executor::prepare(
        &mut state,
        id,
        "house".into(),
        bounds,
        bounds,
        vec![
            executor::PlannedCell {
                portion: executor::PortionKind::Saplings,
                at: next,
                state: air,
                resource: None,
                quantity: 0,
            },
            executor::PlannedCell {
                portion: executor::PortionKind::Build,
                at: build,
                state: log,
                resource: Some("minecraft:oak_log".into()),
                quantity: 1,
            },
        ],
    )
    .unwrap();
    state.settlements.get_mut(id).unwrap().projects = vec![Project {
        settlement: id,
        blueprint: "house".into(),
        level: 1,
        origin: bounds.min,
        rotation: 0,
        building: None,
        structure: structure.clone(),
        ordered: true,
        builder: None,
        order_seq: None,
    }];
    // Decay happens after the canonical clear cell was planned. The crown
    // is nineteen blocks above its footing, beyond any perimeter stand.
    state.world.set_block(next[0], next[1], next[2], air);
    state.world.fold_overlay();
    state.entities.set_pos(builder, [8.5, 201.0, 8.5]);
    assert!(executor::clearing(&state, id, &structure));
    // The planner binds the builder; a direct work call needs the same authority.
    if let Some(record) = state.settlements.get_mut(id) {
        super::bind_builder(
            record,
            &structure,
            Some((state.entities.row(builder).uuid, None)),
        );
    }
    let mut v = state.entities.row_mut(builder).villager.take().unwrap();
    let target = crate::villager::building::work(
        &mut state,
        builder,
        &mut v,
        &mut crate::boundary::ChangeSet::default(),
    );
    state.entities.row_mut(builder).villager = Some(v);
    let target = target.expect("an emptied planned crown cell still has its column-footing target");
    assert_eq!([target.pos.x, target.pos.y, target.pos.z], [8, 201, 8]);
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    // Natural decay is not a stale certification: the footing stays valid and
    // the same structure resumes instead of cancelling the owner's order.
    for _ in 0..3 {
        crate::villager::building::tick(&mut state, id);
    }
    let snapshot = executor::status(&state, id, &structure).unwrap();
    assert_eq!(snapshot.state, ScriptStructureState::Running);
    assert_eq!(
        state.settlements.get(id).unwrap().projects[0].structure,
        structure
    );
    assert!(state.settlements.get(id).unwrap().orders.is_empty());
    assert_eq!(
        state.world.block_at(build[0], build[1], build[2]),
        Some(air)
    );
    assert!(snapshot.consumed.is_empty());
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
}

/// A newer pending order must never take the slot from the active owner order:
/// a cancellation here is not requeued and breaks the owner's FIFO decision.
#[test]
fn a_newer_pending_order_never_drops_the_active_clear_stage() {
    let (mut state, id, builder) = fixture();
    let next = [8, 220, 8];
    let air = state.registry.states.air;
    let log = super::super::allocator::tests::block(&state, "minecraft:oak_log", &[]);
    state.world.set_block(next[0], next[1], next[2], log);
    state.world.fold_overlay();
    let bounds = ScriptWorldBounds::new([4, 201, 4], [14, 222, 14]).unwrap();
    let structure = executor::prepare(
        &mut state,
        id,
        "house".into(),
        bounds,
        bounds,
        vec![
            executor::PlannedCell {
                portion: executor::PortionKind::Trees,
                at: next,
                state: air,
                resource: None,
                quantity: 0,
            },
            executor::PlannedCell {
                portion: executor::PortionKind::Build,
                at: [9, 220, 8],
                state: log,
                resource: Some("minecraft:oak_log".into()),
                quantity: 1,
            },
        ],
    )
    .unwrap();
    state.settlements.get_mut(id).unwrap().projects = vec![Project {
        settlement: id,
        blueprint: "house".into(),
        level: 1,
        origin: bounds.min,
        rotation: 0,
        building: None,
        structure: structure.clone(),
        ordered: true,
        builder: Some((state.entities.row(builder).uuid, None)),
        order_seq: None,
    }];
    // Natural decay after certification must not cancel the active order.
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .planted_trees
        .insert(next);
    state.settlements.get_mut(id).unwrap().orders = vec![OwnerOrder {
        seq: 0,
        blueprint: "storage".into(),
        level: 1,
        building: None,
    }];
    state.world.set_block(next[0], next[1], next[2], air);
    state.world.fold_overlay();
    let uuid = state.entities.row(builder).uuid;
    executor::advance(&mut state, id, &structure, uuid).unwrap();
    assert_eq!(
        executor::status(&state, id, &structure)
            .unwrap()
            .pause_reason
            .as_deref(),
        None
    );
    crate::villager::building::tick(&mut state, id);
    let snapshot = executor::status(&state, id, &structure).unwrap();
    assert_eq!(
        snapshot.state,
        ScriptStructureState::Running,
        "a newer pending order must not take the active order's slot"
    );
    assert_eq!(snapshot.current_stage_index, 1);
    assert_eq!(
        state.settlements.get(id).unwrap().projects[0].structure,
        structure
    );
    assert_eq!(
        state
            .settlements
            .get(id)
            .unwrap()
            .orders
            .first()
            .unwrap()
            .blueprint,
        "storage"
    );
}

/// A build-stage material refusal keeps the same handle without a stale pause;
/// only a preparation-stage stale tree certification can cancel unpaid work.
#[test]
fn a_build_stage_refusal_before_payment_keeps_the_same_handle_without_a_pause() {
    let (mut state, id, builder) = fixture();
    let at = [8, 202, 8];
    let log = super::super::allocator::tests::block(&state, "minecraft:oak_log", &[]);
    state.world.set_block(at[0], at[1], at[2], log);
    state.world.fold_overlay();
    let bounds = ScriptWorldBounds::new([4, 201, 4], [14, 222, 14]).unwrap();
    let structure = executor::prepare(
        &mut state,
        id,
        "house".into(),
        bounds,
        bounds,
        vec![executor::PlannedCell {
            portion: executor::PortionKind::Build,
            at,
            state: log,
            resource: Some("minecraft:oak_log".into()),
            quantity: 1,
        }],
    )
    .unwrap();
    state.settlements.get_mut(id).unwrap().projects = vec![Project {
        settlement: id,
        blueprint: "house".into(),
        level: 1,
        origin: bounds.min,
        rotation: 0,
        building: None,
        structure: structure.clone(),
        ordered: true,
        builder: Some((state.entities.row(builder).uuid, None)),
        order_seq: None,
    }];
    // A grown registered root in build work must never trigger the
    // preparation-only N3 cancellation path.
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .planted_trees
        .insert(at);
    let stone = super::super::allocator::tests::block(&state, "minecraft:stone", &[]);
    state.world.set_block(at[0], at[1], at[2], stone);
    let uuid = state.entities.row(builder).uuid;
    assert_eq!(
        executor::advance(&mut state, id, &structure, uuid),
        Err(mc_script::ScriptOperationFailure::InsufficientItems)
    );
    assert_eq!(
        executor::status(&state, id, &structure)
            .unwrap()
            .pause_reason,
        None
    );
    crate::villager::building::tick(&mut state, id);
    let snapshot = executor::status(&state, id, &structure).unwrap();
    assert_eq!(snapshot.state, ScriptStructureState::Prepared);
    assert_eq!(snapshot.current_stage_index, 0);
    assert_eq!(
        state.settlements.get(id).unwrap().projects[0].structure,
        structure
    );
    assert!(snapshot.consumed.is_empty());
}

/// The planner's builder pass, as `tick` runs it, for two stall windows of a
/// builder that never reaches the next cell; the shift lasts the whole run.
fn stall_with_rotation(state: &mut TickState, id: SettlementId) -> Warnings {
    state.day_time = 2_000;
    let warnings = Warnings::default();
    tracing::subscriber::with_default(warnings.clone(), || {
        for _ in 0..=300 {
            crate::villager::building::tick(state, id);
            state.tick += 20;
            state.day_time += 20;
            assign_builder(state, id);
        }
    });
    warnings
}

fn free_project(
    state: &mut TickState,
    id: SettlementId,
    blueprint: &str,
    cells: Vec<executor::PlannedCell>,
) -> String {
    let bounds = ScriptWorldBounds::new([2, 201, 2], [14, 201, 14]).unwrap();
    // Free paving transforms the soil in place: give each free dirt_path cell
    // real soil to turn into a path (the executor no-ops a paving cell without
    // it, SR3-2).
    if let Some(path) = crate::settlements::layout::paved_state(state) {
        let soil = super::super::allocator::tests::block(state, "minecraft:grass_block", &[]);
        for cell in &cells {
            if cell.state == path {
                state
                    .world
                    .set_block(cell.at[0], cell.at[1], cell.at[2], soil);
            }
        }
        state.world.fold_overlay();
    }
    let structure = executor::prepare(state, id, blueprint.into(), bounds, bounds, cells).unwrap();
    state.settlements.get_mut(id).unwrap().projects = vec![Project {
        settlement: id,
        blueprint: blueprint.into(),
        level: 1,
        origin: bounds.min,
        rotation: 0,
        building: None,
        structure: structure.clone(),
        ordered: false,
        builder: None,
        order_seq: None,
    }];
    structure
}

/// SETTLEMENT-ROAD-STALL-HOLDS-SLOT-TRIAL4, shape A: trial-4's road stood at
/// `Running work=34 consumed=0` for 78,000 ticks while every builder the
/// rotation chose stayed trapped away from cell 35, holding the only slot.
#[test]
fn an_unpaid_road_that_stops_running_gives_back_the_slot_after_a_failed_rotation() {
    let (mut state, id, _) = fixture();
    let path = super::super::allocator::tests::block(&state, "minecraft:dirt_path", &[]);
    let (near, far) = ([3, 201, 4], [14, 201, 14]);
    let structure = free_project(
        &mut state,
        id,
        layout::ROAD,
        [near, far]
            .map(|at| executor::PlannedCell {
                portion: executor::PortionKind::Build,
                at,
                state: path,
                resource: None,
                quantity: 0,
            })
            .into(),
    );
    crate::villager::building::tick(&mut state, id);
    let running = executor::status(&state, id, &structure).unwrap();
    assert_eq!(
        (running.state, running.completed_work_units),
        (ScriptStructureState::Running, 1)
    );
    assert!(running.consumed.is_empty(), "road paving is free work");
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    let warnings = stall_with_rotation(&mut state, id);
    let text = warnings.0.lock().unwrap().join("\n");
    assert!(
        state.settlements.get(id).unwrap().projects.is_empty(),
        "a rotation that still cannot reach the cell must free the slot: {text}"
    );
    assert_eq!(
        executor::status(&state, id, &structure).unwrap().state,
        ScriptStructureState::Cancelled
    );
    assert!(
        text.contains("first_recovery=\"builder_released\""),
        "{text}"
    );
    assert!(
        text.contains("recovery=\"preparation_cancelled\""),
        "{text}"
    );
    assert_eq!(state.world.block_at(near[0], near[1], near[2]), Some(path));
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
}

/// Shape B: growth-03ef2e297a72-laptop-2's perimeter stretch stood `Prepared
/// work=0` while its builders carried the cobblestone but never reached the
/// cell; the owner's queued order waited `busy` behind it.
#[test]
fn an_unpaid_project_that_never_starts_gives_back_the_slot_to_the_queued_order() {
    let (mut state, id, builder) = fixture();
    let cobblestone = super::super::allocator::tests::block(&state, "minecraft:cobblestone", &[]);
    let far = [14, 201, 14];
    let structure = free_project(
        &mut state,
        id,
        layout::PERIMETER,
        vec![executor::PlannedCell {
            portion: executor::PortionKind::Build,
            at: far,
            state: cobblestone,
            resource: Some("minecraft:cobblestone".into()),
            quantity: 1,
        }],
    );
    let item = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:cobblestone").unwrap())
        .unwrap();
    state
        .entities
        .row_mut(builder)
        .villager
        .as_mut()
        .unwrap()
        .inventory[0] = Some(ItemStack::new(item, 1));
    let order = OwnerOrder {
        seq: 0,
        blueprint: "spanish_house".into(),
        level: 1,
        building: None,
    };
    state.settlements.get_mut(id).unwrap().orders = vec![order.clone()];
    let warnings = stall_with_rotation(&mut state, id);
    let text = warnings.0.lock().unwrap().join("\n");
    let record = state.settlements.get(id).unwrap();
    assert!(
        record.projects.is_empty(),
        "a project that never starts must not hold the slot: {text}"
    );
    assert_eq!(record.orders.first(), Some(&order));
    let snapshot = executor::status(&state, id, &structure).unwrap();
    assert_eq!(snapshot.state, ScriptStructureState::Cancelled);
    assert_eq!(snapshot.completed_work_units, 0);
    assert!(text.contains("cause=\"worksite_unreached\""), "{text}");
    assert!(
        text.contains("recovery=\"preparation_cancelled\""),
        "{text}"
    );
    // The carried load returns to storage; nothing is debited or lost.
    let stored: u32 = state.block_entities[&(1, 201, 1)]
        .slots
        .iter()
        .filter(|stack| stack.item_id == item)
        .map(|stack| stack.count as u32)
        .sum();
    assert_eq!(stored, 1);
    assert!(
        state
            .entities
            .row(builder)
            .villager
            .as_ref()
            .unwrap()
            .inventory
            .iter()
            .flatten()
            .all(ItemStack::is_empty)
    );
    assert_eq!(
        state.world.block_at(far[0], far[1], far[2]),
        Some(state.registry.states.air)
    );
}

/// Spain seed 100 (pin 3068dfb1b857): the owner's church waited for a builder
/// through two stall windows, was cancelled and re-sited twice, autonomous
/// work took its freed ground, and the third search ended site_refused
/// 587/587 with the order lost. A project nobody was bound to keeps its site.
#[test]
fn an_order_nobody_was_bound_to_keeps_its_site_through_stall_windows() {
    let (mut state, id, builder) = bed_project();
    state.settlements.get_mut(id).unwrap().projects[0].builder = None;
    state
        .entities
        .row_mut(builder)
        .villager
        .as_mut()
        .unwrap()
        .data
        .profession = Profession::Farmer;
    let project = state.settlements.get(id).unwrap().projects[0].clone();
    let text = log_stall(&mut state, id);
    assert!(
        text.contains("settlement_construction_recovery_failed"),
        "setup: a second window without progress: {text}"
    );
    assert!(!text.contains("preparation_cancelled"), "{text}");
    let record = state.settlements.get(id).unwrap();
    assert_eq!(record.projects, vec![project.clone()], "{text}");
    assert!(
        record.orders.is_empty(),
        "the order was not requeued: {text}"
    );
    assert_ne!(
        executor::status(&state, id, &project.structure)
            .unwrap()
            .state,
        ScriptStructureState::Cancelled
    );
}

/// Diagnostic harness for the House L2 stall: the watcher over a committed terminal record reports
/// what the certificate is missing, once per shift, from the pure query (`certificate_missing`).
/// `finish_project` itself no longer logs on every builder tick. `missing = <description>: found/required`.
#[test]
fn a_committed_house_upgrade_reports_its_missing_certificate_requirement() {
    let (mut state, id, builder) = fixture();
    let bounds = ScriptWorldBounds::new([4, 201, 4], [14, 209, 14]).unwrap();
    let block = super::super::allocator::tests::block(&state, "minecraft:dirt", &[]);
    let handle = executor::prepare(
        &mut state,
        id,
        "spanish_house".into(),
        bounds,
        bounds,
        vec![executor::PlannedCell {
            portion: executor::PortionKind::Build,
            at: [10, 202, 10],
            state: block,
            resource: None,
            quantity: 0,
        }],
    )
    .unwrap();
    state.settlements.get_mut(id).unwrap().projects = vec![Project {
        settlement: id,
        blueprint: "spanish_house".into(),
        level: 2,
        origin: [4, 201, 4],
        rotation: 0,
        building: None,
        structure: handle.clone(),
        ordered: true,
        builder: Some((state.entities.row(builder).uuid, None)),
        order_seq: None,
    }];
    state.world.set_block(10, 202, 10, block);
    let uuid = state.entities.row(builder).uuid;
    executor::advance(&mut state, id, &handle, uuid).unwrap();
    // The refusal is named by the watcher's pure query, not by a per-tick warn in the finish path.
    let warnings = Warnings::default();
    tracing::subscriber::with_default(warnings.clone(), || {
        // The watcher reports once per stall shift (3000 ticks of no progress).
        for _ in 0..=450 {
            state.tick += 20;
            super::release_stalled_builder(&mut state, id);
        }
    });
    let text = warnings.0.lock().unwrap().join("\n");
    assert!(
        text.contains("cause=\"certificate_incomplete\"") && text.contains("missing="),
        "the refusal names its missing requirements: {text}"
    );
}

/// One native tick whose simulation is held: the global tick advances and
/// `game_ticks` does not, exactly as between the retention gate's sprints.
fn held_tick(state: &mut TickState) {
    state.tick += 1;
    state.tick_control.begin_tick(state.tick);
}

/// One simulated tick: both the global tick and the simulation clock advance.
fn running_tick(state: &mut TickState) {
    state.tick += 1;
    state.tick_control.begin_tick(state.tick);
}

#[test]
fn a_frozen_simulation_adds_no_builder_stall_window() {
    let (mut state, id, builder) = bed_project();
    let uuid = state.entities.row(builder).uuid;
    let handle = state.settlements.get(id).unwrap().projects[0]
        .structure
        .clone();
    let game = state.tick_control.game_ticks(state.tick);
    let warnings = Warnings::default();
    tracing::subscriber::with_default(warnings.clone(), || {
        state.tick_control.set_frozen(true);
        for _ in 0..9_000 {
            held_tick(&mut state);
            release_stalled_builder(&mut state, id);
        }
    });
    assert_eq!(
        state.tick_control.game_ticks(state.tick),
        game,
        "9,000 held ticks are not shift time"
    );
    let text = warnings.0.lock().unwrap().join("\n");
    assert!(
        !text.contains("settlement_construction_stalled"),
        "a frozen world must not expire the window: {text}"
    );
    assert_eq!(
        state
            .entities
            .row(builder)
            .villager
            .as_ref()
            .unwrap()
            .data
            .profession,
        Profession::Builder,
        "a frozen world must not strip the builder"
    );
    assert_eq!(
        state.settlements.get(id).unwrap().projects[0]
            .builder
            .map(|(uuid, _)| uuid),
        Some(uuid),
        "the project keeps its builder"
    );
    assert!(
        state
            .settlements
            .builder_watch
            .get(&(id, handle))
            .is_none_or(|watch| watch.shunned.is_empty()),
        "no builder may be shunned by a frozen world"
    );
}

#[test]
fn a_held_day_clock_does_not_stop_the_builder_stall_window() {
    let (mut state, id, _builder) = bed_project();
    let day = state.day_time;
    assert!(
        (2_000..9_000).contains(&day.rem_euclid(24_000)),
        "the fixture must stand inside the work shift"
    );
    let warnings = Warnings::default();
    tracing::subscriber::with_default(warnings.clone(), || {
        state.tick_control.set_frozen(false);
        for _ in 0..=150 {
            for _ in 0..20 {
                state.tick += 1;
                state.tick_control.begin_tick(state.tick);
            }
            release_stalled_builder(&mut state, id);
        }
    });
    assert_eq!(
        state.day_time, day,
        "the day clock is held, as with doDaylightCycle=false"
    );
    let text = warnings.0.lock().unwrap().join("\n");
    assert!(
        text.contains("settlement_construction_stalled"),
        "simulated ticks must still expire the window: {text}"
    );
}

#[test]
fn a_released_builder_is_not_rebound_before_an_unshunned_builder() {
    let (mut state, id, released) = bed_project();
    let handle = state.settlements.get(id).unwrap().projects[0]
        .structure
        .clone();
    state.settlements.get_mut(id).unwrap().projects[0].builder = None;
    // Beside the storage chest, like the fixture's builder, so both pass the
    // storage probe and the ordering rule is what decides.
    let second = super::super::allocator::tests::villager(&mut state, [2.5, 201.0, 1.5]);
    let entity = state.entities.body(second).id;
    crate::settlements::bind(&mut state, id, entity).unwrap();
    {
        let v = state.entities.row_mut(second).villager.as_mut().unwrap();
        v.data.profession = Profession::Builder;
        v.activity = Activity::Work;
    }
    let released_uuid = state.entities.row(released).uuid;
    let second_uuid = state.entities.row(second).uuid;
    let mut cache = super::super::allocator::PoiCache::default();
    let mut path = Vec::new();
    assert!(
        super::builders::probe_storage(&mut state, id, released, &mut cache, &mut path),
        "setup: the released builder must reach storage"
    );
    assert!(
        super::builders::probe_storage(&mut state, id, second, &mut cache, &mut path),
        "setup: the second builder must reach storage"
    );
    // One watcher pass creates this project's episode entry; the shun below is
    // what that episode records when it releases a builder.
    release_stalled_builder(&mut state, id);
    let game = state.tick_control.game_ticks(state.tick);
    shun(&mut state, id, &handle, released_uuid, game);
    assert!(
        state
            .settlements
            .builder_watch
            .get(&(id, handle.clone()))
            .is_some_and(|watch| watch
                .shunned
                .iter()
                .any(|&(uuid, until)| uuid == released_uuid && game < until)),
        "setup: the released builder's shun must be recorded"
    );
    assign_builder(&mut state, id);
    assert_eq!(
        state.settlements.get(id).unwrap().projects[0]
            .builder
            .map(|(uuid, _)| uuid),
        Some(second_uuid),
        "the recovery must rotate to the builder that was not just released"
    );
}

/// An unbound single-project settlement with the fixture's native Builder and,
/// optionally, one `Profession::None` retrain resident beside the storage
/// chest. The watcher pass seeds the project's episode entry.
struct Bench {
    state: TickState,
    id: SettlementId,
    handle: String,
    native: crate::entities::EntitySlot,
    retrain: Option<crate::entities::EntitySlot>,
}

fn bench(with_retrain: bool) -> Bench {
    let (mut state, id, native) = bed_project();
    let handle = state.settlements.get(id).unwrap().projects[0]
        .structure
        .clone();
    state.settlements.get_mut(id).unwrap().projects[0].builder = None;
    let retrain = with_retrain.then(|| {
        let slot = super::super::allocator::tests::villager(&mut state, [2.5, 201.0, 1.5]);
        let entity = state.entities.body(slot).id;
        crate::settlements::bind(&mut state, id, entity).unwrap();
        slot
    });
    release_stalled_builder(&mut state, id);
    Bench {
        state,
        id,
        handle,
        native,
        retrain,
    }
}

/// (native uuid, retrain uuid) with both routes to storage proven, so the
/// ordering rule is what decides the binding.
fn ready_pair(b: &mut Bench) -> (uuid::Uuid, Option<uuid::Uuid>) {
    let (native_slot, retrain_slot) = (b.native, b.retrain);
    let native = b.state.entities.row(native_slot).uuid;
    let retrain = retrain_slot.map(|slot| b.state.entities.row(slot).uuid);
    assert_reaches_storage(b, native_slot);
    if let Some(slot) = retrain_slot {
        assert_reaches_storage(b, slot);
    }
    (native, retrain)
}

fn assert_reaches_storage(bench: &mut Bench, slot: crate::entities::EntitySlot) {
    let mut cache = super::super::allocator::PoiCache::default();
    let mut path = Vec::new();
    assert!(
        super::builders::probe_storage(&mut bench.state, bench.id, slot, &mut cache, &mut path),
        "setup: the candidate must reach storage"
    );
}

fn bound(bench: &Bench) -> Option<uuid::Uuid> {
    bench.state.settlements.get(bench.id).unwrap().projects[0]
        .builder
        .map(|(uuid, _)| uuid)
}

#[test]
fn an_unshunned_retrain_candidate_is_bound_before_a_shunned_native_builder() {
    let mut b = bench(true);
    let (native, retrain) = ready_pair(&mut b);
    let game = b.state.tick_control.game_ticks(b.state.tick);
    shun(&mut b.state, b.id, &b.handle, native, game);
    assign_builder(&mut b.state, b.id);
    assert_eq!(
        bound(&b),
        retrain,
        "the just-released builder must not beat a fresh retrain"
    );
}

#[test]
fn a_sole_shunned_native_builder_is_still_assigned() {
    let mut b = bench(false);
    let (native, _) = ready_pair(&mut b);
    let game = b.state.tick_control.game_ticks(b.state.tick);
    shun(&mut b.state, b.id, &b.handle, native, game);
    assign_builder(&mut b.state, b.id);
    assert_eq!(
        bound(&b),
        Some(native),
        "the only eligible builder takes the site even when shunned"
    );
}

#[test]
fn a_frozen_simulation_does_not_lift_a_builder_shun() {
    let mut b = bench(true);
    let (native, retrain) = ready_pair(&mut b);
    let game = b.state.tick_control.game_ticks(b.state.tick);
    shun(&mut b.state, b.id, &b.handle, native, game);
    b.state.tick_control.set_frozen(true);
    // Longer than BUILDER_SHUN_TICKS in native ticks: a shun measured on the
    // global tick would have lapsed here.
    for _ in 0..(BUILDER_SHUN_TICKS + 6_000) {
        held_tick(&mut b.state);
    }
    assert_eq!(b.state.tick_control.game_ticks(b.state.tick), game);
    assign_builder(&mut b.state, b.id);
    assert_eq!(
        bound(&b),
        retrain,
        "held ticks are not shun time, so the released builder stays shunned"
    );
}

#[test]
fn a_builder_shun_lifts_on_its_game_tick_deadline() {
    let mut b = bench(true);
    let (native, retrain) = ready_pair(&mut b);
    let game = b.state.tick_control.game_ticks(b.state.tick);
    shun(&mut b.state, b.id, &b.handle, native, game);
    b.state.tick_control.set_frozen(false);
    for _ in 0..(BUILDER_SHUN_TICKS - 1) {
        running_tick(&mut b.state);
    }
    assert_eq!(
        b.state.tick_control.game_ticks(b.state.tick),
        game + BUILDER_SHUN_TICKS - 1
    );
    assign_builder(&mut b.state, b.id);
    assert_eq!(
        bound(&b),
        retrain,
        "one tick short of the deadline the released builder is still shunned"
    );

    let mut b = bench(true);
    let (native, _) = ready_pair(&mut b);
    let game = b.state.tick_control.game_ticks(b.state.tick);
    shun(&mut b.state, b.id, &b.handle, native, game);
    b.state.tick_control.set_frozen(false);
    for _ in 0..BUILDER_SHUN_TICKS {
        running_tick(&mut b.state);
    }
    assert_eq!(
        b.state.tick_control.game_ticks(b.state.tick),
        game + BUILDER_SHUN_TICKS
    );
    assign_builder(&mut b.state, b.id);
    assert_eq!(
        bound(&b),
        Some(native),
        "at game == until the shun lapses and the native builder takes the site"
    );
}
