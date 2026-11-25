use super::tests::{attempt, complete, fixture, fund, open_plan, start};
use super::*;
use crate::entities::EntitySlot;

#[test]
fn broken_placed_house_cell_is_rebuilt_before_completion_even_after_restart() {
    for restart in [false, true] {
        let (mut state, id, slot) = fixture();
        start(&mut state, id, "house", 1, None);
        fund(&mut state, "house", 1);
        let project = state.settlements.get(id).unwrap().projects[0].clone();
        let (cell, expected) = first_cell(&state, "house", "oak_log");
        for _ in 0..4096 {
            if state.world.block_at(cell[0], cell[1], cell[2]) == Some(expected) {
                break;
            }
            attempt(&mut state, id, slot);
        }
        assert_eq!(
            state.world.block_at(cell[0], cell[1], cell[2]),
            Some(expected)
        );
        assert!(!state.settlements.get(id).unwrap().projects.is_empty());
        let log = state
            .registry
            .items
            .id_of(&mc_data::Identifier::parse("minecraft:oak_log").unwrap())
            .unwrap();
        let slots = &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots;
        let spare = slots.iter_mut().find(|stack| stack.is_empty()).unwrap();
        *spare = mc_data::ItemStack::new(log, 1);
        state
            .world
            .set_block(cell[0], cell[1], cell[2], state.registry.states.air);
        for _ in 0..4096 {
            let snapshot = executor::status(&state, id, &project.structure).unwrap();
            if snapshot
                .stages
                .get(snapshot.current_stage_index as usize)
                .is_some_and(|stage| stage.stage == "repair")
            {
                break;
            }
            attempt(&mut state, id, slot);
        }
        let snapshot = executor::status(&state, id, &project.structure).unwrap();
        assert_eq!(
            snapshot
                .stages
                .get(snapshot.current_stage_index as usize)
                .map(|stage| stage.stage.as_str()),
            Some("repair"),
            "the broken cell must be re-queued before the project completes"
        );
        assert_eq!(
            executor::next_cell(&state, id, &project.structure).unwrap(),
            Some(cell)
        );
        crate::plugin_structures::validate(&state).unwrap();
        // With no replacement log, the existing shortage path reports the
        // repair and does not place for free.
        let saved_stock = state.block_entities[&(1, 201, 1)].slots.clone();
        let saved_carry = state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .unwrap()
            .inventory
            .clone();
        for stack in &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots {
            if stack.item_id == log {
                *stack = mc_data::ItemStack::EMPTY;
            }
        }
        for stack in &mut state
            .entities
            .row_mut(slot)
            .villager
            .as_mut()
            .unwrap()
            .inventory
        {
            if stack.as_ref().is_some_and(|item| item.item_id == log) {
                *stack = None;
            }
        }
        assert_eq!(
            executor::missing(&state, id, &project.structure)
                .unwrap()
                .get("minecraft:oak_log"),
            Some(&1)
        );
        attempt(&mut state, id, slot);
        assert_eq!(
            state.world.block_at(cell[0], cell[1], cell[2]),
            Some(state.registry.states.air)
        );
        assert!(!state.settlements.get(id).unwrap().projects.is_empty());
        state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots = saved_stock;
        state
            .entities
            .row_mut(slot)
            .villager
            .as_mut()
            .unwrap()
            .inventory = saved_carry;

        if restart {
            let rows = super::super::persistence::checkpoint_rows(&state, state.tick).unwrap();
            let ledger: Vec<(String, serde_json::Value)> = state
                .plugins
                .domain_entries("@native-structures")
                .map(|(key, value)| (key.to_owned(), value.clone()))
                .collect();
            let carry = state.entities.row(slot).villager.clone();
            let (mut restored, restored_id, restored_slot) = fixture();
            assert_eq!(restored_id, id);
            restored.settlements = super::super::Registry::default();
            super::super::persistence::hydrate_dimension(
                &mut restored,
                &rows,
                &crate::journal::Replay::default(),
                id.0,
            )
            .unwrap();
            for (key, value) in ledger {
                restored
                    .plugins
                    .set_domain("@native-structures", &key, value);
            }
            restored.entities.row_mut(restored_slot).villager = carry;
            crate::settlements::rebuild_counts(&mut restored);
            restored.block_entities.get_mut(&(1, 201, 1)).unwrap().slots =
                state.block_entities[&(1, 201, 1)].slots.clone();
            for (position, block) in &state.plugin_block_writes {
                restored
                    .world
                    .set_block(position.x, position.y, position.z, *block);
            }
            restored
                .world
                .set_block(cell[0], cell[1], cell[2], restored.registry.states.air);
            state = restored;
        }
        let stock = |state: &TickState| -> i32 {
            state.block_entities[&(1, 201, 1)]
                .slots
                .iter()
                .chain(
                    state
                        .entities
                        .row(slot)
                        .villager
                        .as_ref()
                        .unwrap()
                        .inventory
                        .iter()
                        .flatten(),
                )
                .filter(|stack| stack.item_id == log)
                .map(|stack| stack.count)
                .sum()
        };
        let before = stock(&state);
        assert!(before >= 1);
        for _ in 0..4096 {
            if state.world.block_at(cell[0], cell[1], cell[2]) == Some(expected) {
                break;
            }
            attempt(&mut state, id, slot);
        }
        assert_eq!(stock(&state), before - 1, "repair must debit one real log");
        complete(&mut state, id, slot);
        assert_eq!(
            state.world.block_at(cell[0], cell[1], cell[2]),
            Some(expected),
            "restart={restart}"
        );
        assert!(state.settlements.get(id).unwrap().projects.is_empty());
    }
}

fn first_cell(state: &TickState, blueprint: &str, path: &str) -> ([i32; 3], u32) {
    let level = blueprints::get(&state.registry, blueprint)
        .unwrap()
        .level(1)
        .unwrap()
        .rotated(Rotation::from_quarter_turns(0));
    let (offset, block) = level
        .blocks
        .iter()
        .find(|(_, block)| state.registry.blocks.by_id(*block).unwrap().block.id.path() == path)
        .unwrap();
    (level.world([3, 200, 3], *offset), block.0)
}

#[test]
fn farmland_drying_and_wheat_growth_during_native_work_do_not_pause_construction() {
    let (mut state, id, slot) = fixture();
    start(&mut state, id, "farm", 1, None);
    fund(&mut state, "farm", 1);
    let project = state.settlements.get(id).unwrap().projects[0].clone();
    let (soil, soil_state) = first_cell(&state, "farm", "farmland");
    let (crop, crop_state) = first_cell(&state, "farm", "wheat");
    for _ in 0..4096 {
        if state.world.block_at(soil[0], soil[1], soil[2]) == Some(soil_state)
            && state.world.block_at(crop[0], crop[1], crop[2]) == Some(crop_state)
        {
            break;
        }
        attempt(&mut state, id, slot);
    }
    assert!(
        !state.settlements.get(id).unwrap().projects.is_empty(),
        "evolution occurs mid-build"
    );
    assert_eq!(
        state.world.block_at(crop[0], crop[1], crop[2]),
        Some(crop_state)
    );
    let dry =
        crate::farming::with_prop(&state.registry.blocks, soil_state, "moisture", "0").unwrap();
    let grown = crate::farming::with_prop(&state.registry.blocks, crop_state, "age", "1").unwrap();
    state.world.set_block(soil[0], soil[1], soil[2], dry);
    state.world.set_block(crop[0], crop[1], crop[2], grown);
    let before = executor::status(&state, id, &project.structure).unwrap();
    attempt(&mut state, id, slot);
    attempt(&mut state, id, slot);
    let after = executor::status(&state, id, &project.structure).unwrap();
    assert_ne!(after.state, ScriptStructureState::Paused);
    assert!(
        after.revision > before.revision,
        "canonical work advances after state evolution"
    );
    complete(&mut state, id, slot);
    crate::plugin_structures::validate(&state).unwrap();
}

/// Seed-24 gate 20261124T195404: the farm's own harvest emptied a crop cell, the
/// completion pass reopened a repair stage of 12 wheat_seeds that no stock could
/// fund (structure `settlement-1-229`, watermark 215), and the project held the
/// settlement's only construction slot for 39 000 ticks, so no House was ever
/// admitted. A harvested crop is the farm's work, not construction damage.
#[test]
fn a_harvested_farm_crop_does_not_reopen_a_repair_stage() {
    let (mut state, id, slot) = fixture();
    start(&mut state, id, "farm", 1, None);
    fund(&mut state, "farm", 1);
    let (crop, crop_state) = first_cell(&state, "farm", "wheat");
    for _ in 0..4096 {
        if state.world.block_at(crop[0], crop[1], crop[2]) == Some(crop_state) {
            break;
        }
        attempt(&mut state, id, slot);
    }
    assert_eq!(
        state.world.block_at(crop[0], crop[1], crop[2]),
        Some(crop_state),
        "the farm's crops are built"
    );
    // The farmer harvests the crop before the completion pass.
    state
        .world
        .set_block(crop[0], crop[1], crop[2], state.registry.states.air);
    complete(&mut state, id, slot);
    assert!(
        state.settlements.get(id).unwrap().projects.is_empty(),
        "a harvested crop must not keep construction open"
    );
    assert_eq!(
        state.world.block_at(crop[0], crop[1], crop[2]),
        Some(state.registry.states.air),
        "construction never re-plants a crop"
    );
    crate::plugin_structures::validate(&state).unwrap();
}

#[test]
fn foreign_replacement_of_a_completed_cell_does_not_pause_pending_portions() {
    let (mut state, id, slot) = fixture();
    start(&mut state, id, "house", 1, None);
    fund(&mut state, "house", 1);
    let project = state.settlements.get(id).unwrap().projects[0].clone();
    let (plank, plank_state) = first_cell(&state, "house", "oak_planks");
    for _ in 0..4096 {
        if state.world.block_at(plank[0], plank[1], plank[2]) == Some(plank_state) {
            break;
        }
        attempt(&mut state, id, slot);
    }
    assert!(!state.settlements.get(id).unwrap().projects.is_empty());
    let before = executor::status(&state, id, &project.structure).unwrap();
    let writes = state.plugin_block_writes.len();
    state
        .world
        .set_block(plank[0], plank[1], plank[2], state.registry.states.stone);
    // Evidence is local to pending cells; a replaced completed cell neither
    // rewinds payment nor freezes unrelated work.
    for _ in 0..8 {
        attempt(&mut state, id, slot);
        assert_ne!(
            executor::status(&state, id, &project.structure)
                .unwrap()
                .state,
            ScriptStructureState::Paused
        );
    }
    let continued = executor::status(&state, id, &project.structure).unwrap();
    assert!(continued.watermark > before.watermark);
    assert!(state.plugin_block_writes.len() > writes);
    complete(&mut state, id, slot);
    assert_eq!(
        state.world.block_at(plank[0], plank[1], plank[2]),
        Some(state.registry.states.stone),
        "recovery never rewrites cells that were already placed"
    );
    crate::plugin_structures::validate(&state).unwrap();
}

fn block_id(state: &TickState, name: &str) -> u32 {
    state
        .registry
        .blocks
        .block(&mc_data::Identifier::parse(name).unwrap())
        .unwrap()
        .default
        .0
}

fn item_id(state: &TickState, name: &str) -> u32 {
    state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse(name).unwrap())
        .unwrap()
}

/// Items of one kind in the settlement chest and the Builder's carry.
fn stock(state: &TickState, slot: EntitySlot, item: u32) -> i64 {
    let chest = state.block_entities.get(&(1, 201, 1)).unwrap().slots.iter();
    let carry = state
        .entities
        .row(slot)
        .villager
        .as_ref()
        .unwrap()
        .inventory
        .iter()
        .flatten();
    chest
        .chain(carry)
        .filter(|stack| stack.item_id == item)
        .map(|stack| i64::from(stack.count))
        .sum()
}

fn add_spare(state: &mut TickState, item: u32, count: i32) {
    let slots = &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots;
    let free = slots.iter().position(|stack| stack.is_empty()).unwrap();
    slots[free] = mc_data::ItemStack::new(item, count);
}

/// Run the project to its end. The project closing (the certificate) while a
/// `(cell, authored block)` pair holds anything else fails the test.
fn complete_authored(
    state: &mut TickState,
    id: SettlementId,
    slot: EntitySlot,
    cells: &[([i32; 3], u32)],
) {
    for _ in 0..8192 {
        if state.settlements.get(id).unwrap().projects.is_empty() {
            for (at, authored) in cells {
                assert_eq!(
                    state.world.block_at(at[0], at[1], at[2]),
                    Some(*authored),
                    "the project closed while {at:?} holds a foreign block"
                );
            }
            return;
        }
        attempt(state, id, slot);
    }
    panic!(
        "construction did not finish: {:?}",
        project_status(state, id)
    );
}

#[test]
fn a_solid_foreign_block_in_a_placed_cell_keeps_its_place_and_the_certificate() {
    let (mut state, id, slot) = fixture();
    start(&mut state, id, "house", 1, None);
    fund(&mut state, "house", 1);
    let (log, log_state) = first_cell(&state, "house", "oak_log");
    for _ in 0..4096 {
        if state.world.block_at(log[0], log[1], log[2]) == Some(log_state) {
            break;
        }
        attempt(&mut state, id, slot);
    }
    let diamond = block_id(&state, "minecraft:diamond_block");
    state.world.set_block(log[0], log[1], log[2], diamond);
    complete(&mut state, id, slot);
    assert_eq!(
        state.world.block_at(log[0], log[1], log[2]),
        Some(diamond),
        "a player's solid block is never replaced"
    );
    assert_eq!(state.settlements.get(id).unwrap().buildings.len(), 1);
}

#[test]
fn a_placed_cell_broken_to_air_is_replaced_before_the_house_certifies() {
    let (mut state, id, slot) = fixture();
    start(&mut state, id, "house", 1, None);
    fund(&mut state, "house", 1);
    let (log, log_state) = first_cell(&state, "house", "oak_log");
    let log_item = item_id(&state, "minecraft:oak_log");
    add_spare(&mut state, log_item, 2);
    let before = stock(&state, slot, log_item);
    for _ in 0..4096 {
        if state.world.block_at(log[0], log[1], log[2]) == Some(log_state) {
            break;
        }
        attempt(&mut state, id, slot);
    }
    assert!(
        !state.settlements.get(id).unwrap().projects.is_empty(),
        "the log is placed mid-build"
    );
    state
        .world
        .set_block(log[0], log[1], log[2], state.registry.states.air);
    complete_authored(&mut state, id, slot, &[(log, log_state)]);
    assert_eq!(
        state.settlements.get(id).unwrap().buildings.len(),
        1,
        "the repaired house is certified"
    );
    let spent = before - stock(&state, slot, log_item);
    let bill = blueprints::get(&state.registry, "house")
        .unwrap()
        .level(1)
        .unwrap()
        .materials
        .iter()
        .find(|(item, _)| *item == log_item)
        .map_or(0, |(_, count)| i64::from(*count));
    assert_eq!(
        spent,
        bill + 1,
        "the bill plus one log for the broken cell: nothing is created or lost"
    );
    crate::plugin_structures::validate(&state).unwrap();
}

#[test]
fn a_cell_broken_after_the_last_placement_is_replaced_instead_of_stalling_the_project() {
    // The breaker's stall: the project held 250/250 with the cell air, never finishing.
    let (mut state, id, slot) = fixture();
    start(&mut state, id, "house", 1, None);
    fund(&mut state, "house", 1);
    let project = state.settlements.get(id).unwrap().projects[0].clone();
    let (log, log_state) = first_cell(&state, "house", "oak_log");
    let log_item = item_id(&state, "minecraft:oak_log");
    add_spare(&mut state, log_item, 2);
    let air = state.registry.states.air;
    for _ in 0..8192 {
        let status = executor::status(&state, id, &project.structure).unwrap();
        let stage = status.stages.get(status.current_stage_index as usize);
        if stage.is_some_and(|stage| {
            status.current_stage_index as usize + 1 == status.stages.len()
                && status.completed_work_units + 1 == stage.work_units
        }) {
            // The next placement is the last one: the project is about to be committed.
            assert_eq!(
                state.world.block_at(log[0], log[1], log[2]),
                Some(log_state)
            );
            state.world.set_block(log[0], log[1], log[2], air);
            break;
        }
        attempt(&mut state, id, slot);
    }
    assert!(!state.settlements.get(id).unwrap().projects.is_empty());
    complete_authored(&mut state, id, slot, &[(log, log_state)]);
    assert_eq!(state.settlements.get(id).unwrap().buildings.len(), 1);
    crate::plugin_structures::validate(&state).unwrap();
}

#[test]
fn a_second_break_during_the_repair_requeues_the_repair_stage() {
    let (mut state, id, slot) = fixture();
    start(&mut state, id, "house", 1, None);
    fund(&mut state, "house", 1);
    let project = state.settlements.get(id).unwrap().projects[0].clone();
    let (log, log_state) = first_cell(&state, "house", "oak_log");
    let (plank, plank_state) = first_cell(&state, "house", "oak_planks");
    for name in ["minecraft:oak_log", "minecraft:oak_planks"] {
        let item = item_id(&state, name);
        add_spare(&mut state, item, 4);
    }
    for _ in 0..4096 {
        if state.world.block_at(log[0], log[1], log[2]) == Some(log_state)
            && state.world.block_at(plank[0], plank[1], plank[2]) == Some(plank_state)
        {
            break;
        }
        attempt(&mut state, id, slot);
    }
    let air = state.registry.states.air;
    state.world.set_block(log[0], log[1], log[2], air);
    for _ in 0..4096 {
        attempt(&mut state, id, slot);
        let status = executor::status(&state, id, &project.structure).unwrap();
        if status
            .stages
            .last()
            .is_some_and(|stage| stage.stage == "repair")
        {
            assert_eq!(status.state, ScriptStructureState::Running);
            break;
        }
    }
    // The repair cell is not placed yet; break another, already placed cell.
    state.world.set_block(plank[0], plank[1], plank[2], air);
    complete_authored(
        &mut state,
        id,
        slot,
        &[(log, log_state), (plank, plank_state)],
    );
    let stages = executor::status(&state, id, &project.structure)
        .unwrap()
        .stages;
    assert_eq!(
        stages
            .iter()
            .filter(|stage| stage.stage == "repair")
            .count(),
        1,
        "one repair stage, requeued"
    );
    crate::plugin_structures::validate(&state).unwrap();
}

#[test]
fn an_upgrade_repairs_an_inherited_perimeter_cell_emptied_mid_build() {
    // Gate 20261124T231255 (house.L2_upgrade): the level-2 certificate inspects the whole level
    // box, and its min-x wall is an *unchanged level-1 wall* — not in the level-2 plan's diff. A
    // survival player emptied exactly such a cell mid-upgrade, so `finish_project` refused
    // `missing=enclosure: 0/1` every tick and the project never closed. The repair scan must cover
    // the level image the previous level authored, not only this record's own cells.
    let (mut state, id, slot) = fixture();
    open_plan(&mut state);
    fund(&mut state, "house", 1);
    start(&mut state, id, "house", 1, None);
    complete(&mut state, id, slot);
    let building = *state
        .settlements
        .get(id)
        .unwrap()
        .buildings
        .keys()
        .next()
        .unwrap();
    assert_eq!(
        state.settlements.get(id).unwrap().buildings[&building].level,
        1
    );
    fund(&mut state, "house", 2);
    start(&mut state, id, "house", 2, Some(building));
    let project = state.settlements.get(id).unwrap().projects[0].clone();
    assert_eq!(project.level, 2, "the upgrade is planned before the break");
    let blueprint = blueprints::get(&state.registry, "house").expect("house blueprint");
    let bounds = blueprint
        .level(2)
        .expect("level 2")
        .rotated(Rotation::from_quarter_turns(project.rotation))
        .bounds(project.origin);
    // Level 1's corner log stands on the level-2 certificate's min-x wall at its own origin, and
    // level 2's image keeps that block, so no cell of the level-2 increment owns it.
    let at = [project.origin[0], project.origin[1] + 1, project.origin[2]];
    assert_eq!(
        (at[0], at[2]),
        (bounds.min[0], bounds.min[2]),
        "the inherited cell is on the level-2 certificate wall"
    );
    let (_, planned) = planned_cells(&state, &project, "oak", &mut Default::default())
        .expect("the level-2 increment plans");
    assert!(
        !planned.iter().any(|cell| cell.at == at),
        "the level-2 diff does not place the inherited wall cell"
    );
    let expected = state
        .world
        .block_at(at[0], at[1], at[2])
        .expect("level 1 placed the inherited wall cell");
    assert_eq!(
        state
            .registry
            .blocks
            .by_id(mc_world::BlockStateId(expected))
            .expect("known block")
            .block
            .id
            .path(),
        "oak_log",
        "the inherited cell is the level-1 corner log"
    );
    // The repair pays the cell's material again: one spare log keeps the repair fundable.
    let log = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:oak_log").unwrap())
        .unwrap();
    let slots = &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots;
    *slots.iter_mut().find(|stack| stack.is_empty()).unwrap() = mc_data::ItemStack::new(log, 2);
    state
        .world
        .set_block(at[0], at[1], at[2], state.registry.states.air);
    state.world.fold_overlay();
    assert!(
        crate::settlements::buildings::inspect(&state, BuildingKind::House, 2, bounds).is_err(),
        "the emptied inherited cell is what the level-2 certificate inspects"
    );
    complete_authored(&mut state, id, slot, &[(at, expected)]);
    assert_eq!(
        state.settlements.get(id).unwrap().buildings[&building].level,
        2,
        "the repaired level-2 house certifies"
    );
}
