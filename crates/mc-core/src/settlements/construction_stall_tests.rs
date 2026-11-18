use super::growth_tests::Warnings;
use super::tests::fixture;
use super::*;
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
