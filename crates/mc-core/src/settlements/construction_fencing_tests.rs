use super::tests::{attempt, complete, fixture, fund, start};
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
        assert!(
            state
                .settlements
                .get(id)
                .unwrap()
                .projects
                .first()
                .is_some()
        );
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
        assert!(
            state
                .settlements
                .get(id)
                .unwrap()
                .projects
                .first()
                .is_some()
        );
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
            let ledger = state
                .plugins
                .get_domain("@native-structures", "ledger")
                .unwrap()
                .clone();
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
            restored
                .plugins
                .set_domain("@native-structures", "ledger", ledger);
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
        state
            .settlements
            .get(id)
            .unwrap()
            .projects
            .first()
            .is_some(),
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

#[test]
fn foreign_plank_replacement_pauses_without_debit_then_next_admission_resumes_and_completes() {
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
    assert!(
        state
            .settlements
            .get(id)
            .unwrap()
            .projects
            .first()
            .is_some()
    );
    let before = executor::status(&state, id, &project.structure).unwrap();
    let writes = state.plugin_block_writes.len();
    state
        .world
        .set_block(plank[0], plank[1], plank[2], state.registry.states.stone);
    // The executor still refuses to place over a changed footprint.
    let mut paused = before.clone();
    for _ in 0..8 {
        attempt(&mut state, id, slot);
        paused = executor::status(&state, id, &project.structure).unwrap();
        if paused.state == ScriptStructureState::Paused {
            break;
        }
    }
    assert_eq!(paused.state, ScriptStructureState::Paused);
    assert_eq!(paused.pause_reason.as_deref(), Some("footprint_changed"));
    assert_eq!(paused.completed_work_units, before.completed_work_units);
    assert_eq!(paused.consumed, before.consumed);
    assert_eq!(state.plugin_block_writes.len(), writes);
    // The next construction admission re-snapshots the footprint and resumes.
    attempt(&mut state, id, slot);
    let resumed = executor::status(&state, id, &project.structure).unwrap();
    assert_ne!(resumed.state, ScriptStructureState::Paused);
    assert_eq!(resumed.pause_reason, None);
    assert_eq!(resumed.completed_work_units, before.completed_work_units);
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
        state
            .settlements
            .get(id)
            .unwrap()
            .projects
            .first()
            .is_some(),
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
    assert!(
        state
            .settlements
            .get(id)
            .unwrap()
            .projects
            .first()
            .is_some()
    );
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
