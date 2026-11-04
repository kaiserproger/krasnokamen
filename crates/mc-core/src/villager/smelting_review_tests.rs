//! Regression tests for furnace task custody and stalled loads.
use super::*;
use crate::boundary::BlockPos;
use crate::entities::EntitySlot;
use crate::villager::smelting;

fn step(state: &mut TickState, slot: EntitySlot) -> Option<Option<BlockPos>> {
    let mut worker = state.entities.row_mut(slot).villager.take().unwrap();
    let result = smelting::work(state, slot, &mut worker);
    state.entities.row_mut(slot).villager = Some(worker);
    result
}

fn legacy_load(state: &mut TickState, slot: EntitySlot) {
    // Deserialize the actual old persisted shape; do not invent a receipt for it.
    let task = serde_json::from_value(serde_json::json!({
        "furnace": [FURNACE.0, FURNACE.1, FURNACE.2],
        "phase": "Load"
    }))
    .unwrap();
    state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .smelting = Some(task);
}

fn assert_not_loading(state: &TickState, slot: EntitySlot) {
    let worker = state.entities.row(slot).villager.as_ref().unwrap();
    assert!(
        !worker
            .smelting
            .is_some_and(|task| task.phase == smelting::Phase::Load),
        "no available fuel must not leave the worker parked in Load: {:?}",
        worker.smelting
    );
}

#[test]
fn unfueled_furnace_without_fuel_does_not_park_fresh_or_legacy_task() {
    for persisted in [false, true] {
        let (mut state, worker, _) = setup();
        let slot = state.entities.slot_of(worker).unwrap();
        let raw = mining::item_id(&state, "raw_iron").unwrap();
        state.block_entities.get_mut(&CHEST).unwrap().slots[1] = ItemStack::EMPTY;
        // Move one ore from storage into the already committed furnace batch.
        state.block_entities.get_mut(&CHEST).unwrap().slots[0].count -= 1;
        state.block_entities.get_mut(&FURNACE).unwrap().slots[0] = ItemStack::new(raw, 1);
        if persisted {
            legacy_load(&mut state, slot);
        }
        state.entities.set_pos(slot, [10.5, 201.0, 3.5]);
        let before = counts(&state);
        step(&mut state, slot);
        assert_eq!(
            counts(&state),
            before,
            "stock must be conserved, persisted={persisted}"
        );
        assert_eq!(state.block_entities[&FURNACE].slots[0].count, 1);
        assert!(state.block_entities[&FURNACE].slots[1].is_empty());
        assert_not_loading(&state, slot);
    }
}

#[test]
fn legacy_load_does_not_burn_unrelated_carried_wool_or_planks() {
    for name in ["white_wool", "oak_planks"] {
        let (mut state, worker, _) = setup();
        let slot = state.entities.slot_of(worker).unwrap();
        let raw = mining::item_id(&state, "raw_iron").unwrap();
        let unrelated = mining::item_id(&state, name).unwrap();
        state.block_entities.get_mut(&CHEST).unwrap().slots[0].count -= 1;
        state.block_entities.get_mut(&FURNACE).unwrap().slots[0] = ItemStack::new(raw, 1);
        state
            .entities
            .row_mut(slot)
            .villager
            .as_mut()
            .unwrap()
            .inventory[0] = Some(ItemStack::new(unrelated, 4));
        legacy_load(&mut state, slot);
        state.entities.set_pos(slot, [10.5, 201.0, 3.5]);
        let before = counts(&state);
        step(&mut state, slot);
        assert_eq!(counts(&state), before, "stock must be conserved for {name}");
        assert!(
            state.block_entities[&FURNACE].slots[1].is_empty(),
            "legacy task must not insert unrelated {name} as fuel"
        );
        assert_not_loading(&state, slot);
    }
}

#[test]
fn fetched_ore_and_fuel_load_while_unrelated_carry_remains_untouched() {
    let (mut state, worker, _) = setup();
    let slot = state.entities.slot_of(worker).unwrap();
    let raw = mining::item_id(&state, "raw_iron").unwrap();
    let coal = mining::item_id(&state, "coal").unwrap();
    // Held shears satisfy this fixture's entire iron demand.
    let diamond = mining::item_id(&state, "diamond").unwrap();
    state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .inventory[0] = Some(ItemStack::new(diamond, 1));
    let before = counts(&state);
    assert_eq!(step(&mut state, slot), Some(None), "fetch ore");
    assert_eq!(state.block_entities[&CHEST].slots[0].count, 7);
    assert_eq!(step(&mut state, slot), Some(None), "fetch fuel");
    assert_eq!(state.block_entities[&CHEST].slots[1].count, 2);
    state.entities.set_pos(slot, [10.5, 201.0, 3.5]);
    assert_eq!(step(&mut state, slot), Some(None), "load furnace");
    let furnace = &state.block_entities[&FURNACE].slots;
    assert_eq!((furnace[0].item_id, furnace[0].count), (raw, 2));
    assert_eq!((furnace[1].item_id, furnace[1].count), (coal, 1));
    let held = &state
        .entities
        .row(slot)
        .villager
        .as_ref()
        .unwrap()
        .inventory[0];
    assert_eq!(
        held.as_ref().map(|stack| (stack.item_id, stack.count)),
        Some((diamond, 1))
    );
    assert_eq!(
        counts(&state),
        before,
        "transfers must conserve all physical stock"
    );
}

#[test]
fn fresh_task_inserts_only_fetched_fuel_not_other_carried_fuel() {
    let (mut state, worker, _) = setup();
    let slot = state.entities.slot_of(worker).unwrap();
    let diamond = mining::item_id(&state, "diamond").unwrap();
    let wool = mining::item_id(&state, "white_wool").unwrap();
    let coal = mining::item_id(&state, "coal").unwrap();
    // Occupy the first slot so the actual ore and coal receipts refer to
    // later slots. Add unrelated wool before the load, not to the receipt.
    state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .inventory[0] = Some(ItemStack::new(diamond, 1));
    assert_eq!(step(&mut state, slot), Some(None));
    assert_eq!(step(&mut state, slot), Some(None));
    let inventory = &mut state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .inventory;
    inventory[3] = inventory[0].take();
    inventory[0] = Some(ItemStack::new(wool, 4));
    state.entities.set_pos(slot, [10.5, 201.0, 3.5]);
    let before = counts(&state);
    assert_eq!(step(&mut state, slot), Some(None));
    let fuel = &state.block_entities[&FURNACE].slots[1];
    assert_eq!((fuel.item_id, fuel.count), (coal, 1));
    let held = &state
        .entities
        .row(slot)
        .villager
        .as_ref()
        .unwrap()
        .inventory[0];
    assert_eq!(
        held.as_ref().map(|stack| (stack.item_id, stack.count)),
        Some((wool, 4))
    );
    assert_eq!(
        counts(&state),
        before,
        "transfers must conserve all physical stock"
    );
}
