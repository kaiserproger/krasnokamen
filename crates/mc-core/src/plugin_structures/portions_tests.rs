use super::*;
use crate::plugin_structures::settlement_builds as builds;
use crate::settlements::allocator::tests::block;
use crate::settlements::{SettlementId, construction::Project};
use mc_data::ItemStack;

#[test]
fn settlement_extraction_credits_canonical_snow_layers_without_silk() {
    // Canonical table: data/vanilla/26.3/data/minecraft/loot_table/blocks/snow.json:7-200.
    let (state, _, _, _) = plan();
    let snowball = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:snowball").unwrap())
        .unwrap();
    for layers in 1..=8 {
        let layer = layers.to_string();
        let raw = block(&state, "minecraft:snow", &[("layers", &layer)]);
        assert_eq!(
            conserved_drops(&state, [8, 201, 4], raw).unwrap(),
            vec![ItemStack::new(snowball, layers)]
        );
    }
    let raw = block(&state, "minecraft:snow_block", &[]);
    assert_eq!(
        conserved_drops(&state, [8, 201, 4], raw).unwrap(),
        vec![ItemStack::new(snowball, 4)]
    );
}

#[test]
fn implicit_preparation_tools_use_the_lowest_suitable_ordinary_tier() {
    let (state, _, _, _) = plan();
    for (name, tool_name, drop_name) in [
        (
            "minecraft:cobblestone",
            "minecraft:wooden_pickaxe",
            "minecraft:cobblestone",
        ),
        (
            "minecraft:iron_ore",
            "minecraft:stone_pickaxe",
            "minecraft:raw_iron",
        ),
        (
            "minecraft:diamond_ore",
            "minecraft:iron_pickaxe",
            "minecraft:diamond",
        ),
        (
            "minecraft:obsidian",
            "minecraft:diamond_pickaxe",
            "minecraft:obsidian",
        ),
        (
            "minecraft:snow",
            "minecraft:wooden_shovel",
            "minecraft:snowball",
        ),
    ] {
        let block = block(&state, name, &[]);
        let definition = state
            .registry
            .blocks
            .by_id(mc_world::BlockStateId(block))
            .unwrap();
        let tool = canonical_tool(&state, &definition.block).unwrap();
        assert_eq!(
            state.registry.items.name_of(tool.item_id).unwrap().as_str(),
            tool_name
        );
        assert!(tool.enchantments.is_empty());
        let drops = conserved_drops(&state, [8, 201, 4], block)
            .unwrap_or_else(|failure| panic!("{name}: {failure:?}"));
        assert!(!drops.is_empty(), "{name} must yield normal mining loot");
        assert!(drops.iter().all(|stack| {
            stack.count > 0
                && state
                    .registry
                    .items
                    .name_of(stack.item_id)
                    .unwrap()
                    .as_str()
                    == drop_name
        }));
    }
}

#[test]
fn unchanged_village_placements_credit_nonterrain_but_not_foundation_soil() {
    let (mut state, id, builder, handle) =
        plan_with_build([Some("minecraft:oak_planks"), Some("minecraft:dirt")]);
    for _ in 0..5 {
        builds::advance(&mut state, id, &handle, builder).unwrap();
    }
    assert_eq!(stock(&state, "minecraft:oak_planks"), 1);
    let soil = stock(&state, "minecraft:dirt");
    assert!(builds::advance(&mut state, id, &handle, builder).unwrap());
    assert_eq!(stock(&state, "minecraft:oak_planks"), 1);
    assert_eq!(stock(&state, "minecraft:dirt"), soil);
}

fn plan() -> (TickState, SettlementId, uuid::Uuid, String) {
    plan_with_build([None, None])
}

fn plan_with_build(before: [Option<&str>; 2]) -> (TickState, SettlementId, uuid::Uuid, String) {
    let (mut state, id, builder) = crate::settlements::construction::tests::fixture();
    for (index, name) in before.into_iter().enumerate() {
        if let Some(name) = name {
            let block = block(&state, name, &[]);
            state.world.set_block(8 + index as i32, 201, 4, block);
        }
    }
    state.registry = Arc::new(
        crate::registry::CoreRegistry::load(Some(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla"),
        ))
        .unwrap(),
    );
    let uuid = state.entities.row(builder).uuid;
    let sapling = block(&state, "minecraft:oak_sapling", &[]);
    let log = block(&state, "minecraft:oak_log", &[]);
    let air = state.registry.states.air;
    state.world.set_block(4, 201, 4, sapling);
    state.world.set_block(5, 201, 4, log);
    state
        .world
        .set_block(6, 201, 4, state.registry.states.stone);
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .planted_trees
        .extend([[4, 201, 4], [5, 201, 4]]);
    let bounds = ScriptWorldBounds::new([4, 201, 4], [9, 201, 4]).unwrap();
    let cells = [
        (PortionKind::Saplings, [4, 201, 4], air, None),
        (PortionKind::Trees, [5, 201, 4], air, None),
        (PortionKind::Cut, [6, 201, 4], air, None),
        (
            PortionKind::Fill,
            [7, 201, 4],
            state.registry.states.dirt,
            Some("minecraft:dirt"),
        ),
        (
            PortionKind::Build,
            [8, 201, 4],
            log,
            Some("minecraft:oak_log"),
        ),
        (
            PortionKind::Build,
            [9, 201, 4],
            log,
            Some("minecraft:oak_log"),
        ),
    ]
    .into_iter()
    .map(|(portion, at, state, resource)| builds::PlannedCell {
        portion,
        at,
        state,
        quantity: u32::from(resource.is_some()),
        resource: resource.map(str::to_owned),
    })
    .collect();
    let handle = builds::prepare(&mut state, id, "house".into(), bounds, bounds, cells).unwrap();
    state.settlements.get_mut(id).unwrap().projects = vec![Project {
        settlement: id,
        blueprint: "house".into(),
        level: 1,
        origin: bounds.min,
        rotation: 0,
        building: None,
        structure: handle.clone(),
        ordered: false,
        builder: Some((uuid, None)),
        order_seq: None,
    }];
    for (slot, name) in ["minecraft:dirt", "minecraft:oak_log"]
        .into_iter()
        .enumerate()
    {
        let item = state
            .registry
            .items
            .id_of(&mc_data::Identifier::parse(name).unwrap())
            .unwrap();
        state
            .entities
            .row_mut(builder)
            .villager
            .as_mut()
            .unwrap()
            .inventory[slot] = Some(ItemStack::new(item, 8));
    }
    (state, id, uuid, handle)
}

fn certificate(state: &TickState, handle: &str) -> serde_json::Value {
    state
        .plugins
        .get_domain(OWNER, &ledger::structure_key(handle))
        .unwrap()
        .clone()
}

fn stock(state: &TickState, name: &str) -> i32 {
    let item = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse(name).unwrap())
        .unwrap();
    state.block_entities[&(1, 201, 1)]
        .slots
        .iter()
        .filter(|stack| stack.item_id == item)
        .map(|stack| stack.count)
        .sum()
}

#[test]
fn upgrade_registered_sapling_is_still_the_first_disposition() {
    let (state, id, _, _) = plan();
    let cell = builds::PlannedCell {
        portion: PortionKind::Build,
        at: [4, 201, 4],
        state: state.registry.states.stone,
        resource: Some("minecraft:cobblestone".into()),
        quantity: 1,
    };
    let lot = ScriptWorldBounds::new(cell.at, cell.at).unwrap();
    let cleared = builds::upgrade_trees_clear(
        &state,
        id,
        "house",
        cell.at,
        &[cell],
        Some(lot),
        &mut crate::settlements::forestry::Clearance::default(),
    )
    .unwrap();
    assert_eq!(cleared.len(), 1);
    assert_eq!(cleared[0].portion, PortionKind::Saplings);
}

#[test]
fn preparation_portions_follow_disposition_order_and_retire_exact_roots() {
    let (mut state, id, builder, handle) = plan();
    let snapshot = builds::status(&state, id, &handle).unwrap();
    assert_eq!(
        snapshot
            .stages
            .iter()
            .map(|stage| stage.stage.as_str())
            .collect::<Vec<_>>(),
        ["saplings", "trees", "cut", "fill", "build"]
    );
    let record = certificate(&state, &handle);
    assert_eq!(
        record["portions"][0]["roots"],
        serde_json::json!([[4, 201, 4]])
    );
    assert_eq!(
        record["portions"][1]["roots"],
        serde_json::json!([[5, 201, 4]])
    );
    for index in 0..6 {
        assert_eq!(
            builds::next_cell(&state, id, &handle).unwrap(),
            Some([4 + index, 201, 4])
        );
        builds::advance(&mut state, id, &handle, builder).unwrap();
        validate(&state).unwrap();
    }
    assert_eq!(
        builds::status(&state, id, &handle).unwrap().state,
        ScriptStructureState::Committed
    );
    assert!(state.settlements.get(id).unwrap().planted_trees.is_empty());
    assert_eq!(stock(&state, "minecraft:oak_log"), 1);
    assert_eq!(stock(&state, "minecraft:cobblestone"), 1);
}

#[test]
fn changed_later_and_mid_portion_cells_rederive_with_exact_credit_without_livelock() {
    let (mut state, id, builder, handle) = plan();
    // A later build cell changes, then changes again after this portion started.
    state
        .world
        .set_block(9, 201, 4, state.registry.states.stone);
    for _ in 0..5 {
        builds::advance(&mut state, id, &handle, builder).unwrap();
    }
    let before = builds::status(&state, id, &handle).unwrap();
    assert_eq!(
        (before.current_stage_index, before.completed_work_units),
        (4, 1)
    );
    state.world.set_block(9, 201, 4, state.registry.states.dirt);
    let evidence_before = certificate(&state, &handle);
    assert!(builds::advance(&mut state, id, &handle, builder).unwrap());
    let after = builds::status(&state, id, &handle).unwrap();
    assert_eq!(after.state, ScriptStructureState::Committed);
    assert_eq!(after.watermark, 6);
    assert_eq!(
        stock(&state, "minecraft:dirt"),
        1,
        "credit the live dirt, not the stale stone"
    );
    assert_eq!(
        stock(&state, "minecraft:cobblestone"),
        1,
        "only the original cut credits stone"
    );
    let evidence_after = certificate(&state, &handle);
    assert_eq!(
        evidence_after["portions"], evidence_before["portions"],
        "processed evidence is not rewritten"
    );
    assert_eq!(
        evidence_after["portions"][0],
        evidence_before["portions"][0]
    );
    assert_eq!(
        after
            .consumed
            .iter()
            .map(|m| (m.resource.as_str(), m.quantity))
            .collect::<Vec<_>>(),
        [("minecraft:dirt", 1), ("minecraft:oak_log", 2)]
    );
    assert_eq!(
        builds::advance(&mut state, id, &handle, builder),
        Err(Failure::Blocked)
    );
    assert_eq!(stock(&state, "minecraft:dirt"), 1);
    validate(&state).unwrap();
}

#[test]
fn outside_changed_cell_rederives_but_history_veto_publishes_nothing() {
    for protected in [false, true] {
        let (mut state, id, builder, handle) = plan();
        for _ in 0..5 {
            builds::advance(&mut state, id, &handle, builder).unwrap();
        }
        state.settlements.get_mut(id).unwrap().radius = 1;
        let grass = block(&state, "minecraft:grass_block", &[]);
        state.world.set_block(9, 201, 4, grass);
        if protected {
            state.block_log =
                crate::settlements::forestry::tests::player_history(&state, [9, 201, 4], 0);
            assert!(state.block_log.player_touched([9, 201, 4]));
        }
        let before = certificate(&state, &handle);
        let carry = state
            .entities
            .row(state.entities.slot_of_uuid(builder).unwrap())
            .villager
            .as_ref()
            .unwrap()
            .inventory
            .clone();
        if protected {
            assert_eq!(
                builds::advance(&mut state, id, &handle, builder),
                Err(Failure::Blocked)
            );
            assert_eq!(certificate(&state, &handle), before);
            assert_eq!(stock(&state, "minecraft:dirt"), 0);
            assert_eq!(state.world.block_at(9, 201, 4), Some(grass));
            assert_eq!(
                state
                    .entities
                    .row(state.entities.slot_of_uuid(builder).unwrap())
                    .villager
                    .as_ref()
                    .unwrap()
                    .inventory,
                carry
            );
        } else {
            assert!(builds::advance(&mut state, id, &handle, builder).unwrap());
            assert_eq!(stock(&state, "minecraft:dirt"), 1);
        }
    }
}

#[test]
fn missing_partial_or_corrupt_portions_refuse_startup_but_empty_world_opens() {
    let (empty, _, _) = crate::settlements::construction::tests::fixture();
    validate(&empty).unwrap();
    let (mut state, _, _, handle) = plan();
    let key = ledger::structure_key(&handle);
    let valid = certificate(&state, &handle);
    for variant in [
        "missing",
        "partial",
        "raw",
        "identity",
        "roots",
        "duplicate_root",
        "missing_roots",
        "missing_expected",
        "unknown",
    ] {
        let mut bad = valid.clone();
        match variant {
            "missing" => {
                bad.as_object_mut().unwrap().remove("portions");
            }
            "partial" => {
                bad["portions"].as_array_mut().unwrap().pop();
            }
            "raw" => bad["portions"][0]["expected"] = serde_json::json!([u32::MAX]),
            "identity" => bad["portions"][0]["expected"] = serde_json::json!([]),
            "roots" => bad["portions"][0]["roots"] = serde_json::json!([[15, 201, 15]]),
            "duplicate_root" => {
                bad["portions"][0]["roots"] = serde_json::json!([[4, 201, 4], [4, 201, 4]])
            }
            "missing_roots" => {
                bad["portions"][0].as_object_mut().unwrap().remove("roots");
            }
            "missing_expected" => {
                bad["portions"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("expected");
            }
            _ => bad["portions"][0]["extra"] = serde_json::json!(true),
        }
        state.plugins.set_domain(OWNER, &key, bad);
        assert!(
            matches!(validate(&state), Err(crate::CoreError::Recovery(_))),
            "{variant}"
        );
    }
    state.plugins.set_domain(OWNER, &key, valid);
    validate(&state).unwrap();
}

#[test]
fn resume_after_paid_progress_uses_cell_evidence_not_the_admission_box() {
    let (mut state, id, builder, handle) = plan();
    for _ in 0..5 {
        builds::advance(&mut state, id, &handle, builder).unwrap();
    }
    let before = builds::status(&state, id, &handle).unwrap();
    execute_operation(
        &mut state,
        &builds::owner(id),
        &ScriptWorldStructureOperation::PauseStructure {
            operation_id: "pause".into(),
            structure_id: handle.clone(),
            expected_revision: before.revision,
        },
    )
    .unwrap();
    let paused = builds::status(&state, id, &handle).unwrap();
    execute_operation(
        &mut state,
        &builds::owner(id),
        &ScriptWorldStructureOperation::ResumeStructure {
            operation_id: "resume".into(),
            structure_id: handle.clone(),
            expected_revision: paused.revision,
        },
    )
    .unwrap();
    assert!(builds::advance(&mut state, id, &handle, builder).unwrap());
    assert_eq!(builds::status(&state, id, &handle).unwrap().watermark, 6);
}

#[test]
fn completed_fill_repairs_and_all_rewrites_preserve_certificate_alignment() {
    let (mut state, id, builder, handle) = plan();
    let mut replica = Replica::capture(&mut state);
    for _ in 0..6 {
        builds::advance(&mut state, id, &handle, builder).unwrap();
    }
    replica.step(&mut state);
    state.world.set_block(7, 201, 4, state.registry.states.air);
    assert!(builds::recover(&mut state, id, &handle).unwrap());
    replica.step(&mut state);
    replica.assert_matches(&state);
    validate(&state).unwrap();
    assert_eq!(
        builds::status(&state, id, &handle)
            .unwrap()
            .stages
            .last()
            .unwrap()
            .stage,
        "repair"
    );
    // A second interference while repair is pending truncates and re-derives
    // that same final portion, never appending a second repair certificate.
    let mut local = ledger_mut(&mut state).unwrap();
    let record = local.structure_mut(&handle).unwrap();
    record.snapshot.state = ScriptStructureState::Paused;
    record.snapshot.pause_reason = Some("footprint_changed".into());
    save(&mut state, local);
    assert!(builds::recover(&mut state, id, &handle).unwrap());
    replica.step(&mut state);
    replica.assert_matches(&state);
    validate(&state).unwrap();
    assert_eq!(builds::status(&state, id, &handle).unwrap().stages.len(), 6);
    assert!(builds::advance(&mut state, id, &handle, builder).unwrap());
    state.world.set_block(7, 201, 4, state.registry.states.air);
    assert!(builds::reopen_emptied(&mut state, id, &handle).unwrap());
    replica.step(&mut state);
    replica.assert_matches(&state);
    validate(&state).unwrap();
    assert!(builds::advance(&mut state, id, &handle, builder).unwrap());
    let snapshot = builds::status(&state, id, &handle).unwrap();
    assert_eq!(
        snapshot
            .consumed
            .iter()
            .find(|m| m.resource == "minecraft:dirt")
            .unwrap()
            .quantity,
        3
    );
    validate(&state).unwrap();
}

#[test]
fn overlapping_portions_simulate_preceding_targets_in_the_certificate() {
    let (mut state, _, _, handle) = plan();
    let original = ledger(&state).unwrap().structures()[&handle].cells[3][0].clone();
    let mut cleared = original.clone();
    cleared.state = state.registry.states.air;
    cleared.resource = None;
    cleared.quantity = 0;
    state.world.set_block(
        original.at[0],
        original.at[1],
        original.at[2],
        state.registry.states.stone,
    );
    let portions = certify(&state, &[vec![cleared], vec![original]]).unwrap();
    assert_eq!(
        portions[0].expected,
        [identity(&state, state.registry.states.stone).unwrap()]
    );
    assert_eq!(
        portions[1].expected,
        [identity(&state, state.registry.states.air).unwrap()]
    );
}

#[test]
fn grown_certified_sapling_requeues_before_any_cut_and_keeps_its_root() {
    let (mut state, id, builder, handle) = plan();
    state.settlements.get_mut(id).unwrap().projects[0].ordered = true;
    let log = block(&state, "minecraft:oak_log", &[]);
    state.world.set_block(4, 201, 4, log);
    state.world.set_block(4, 202, 4, log);
    assert!(!builds::advance(&mut state, id, &handle, builder).unwrap());
    assert_eq!(
        builds::status(&state, id, &handle)
            .unwrap()
            .pause_reason
            .as_deref(),
        Some("footprint_changed")
    );
    assert!(builds::recover(&mut state, id, &handle).unwrap());
    assert_eq!(
        builds::status(&state, id, &handle).unwrap().state,
        ScriptStructureState::Cancelled
    );
    assert_eq!(state.world.block_at(4, 201, 4), Some(log));
    assert!(
        state
            .settlements
            .get(id)
            .unwrap()
            .planted_trees
            .contains(&[4, 201, 4])
    );
    assert!(state.settlements.get(id).unwrap().projects.is_empty());
    assert_eq!(state.settlements.get(id).unwrap().orders.len(), 1);
}
