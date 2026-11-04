//! A town without a furnace must not plan iron once per resident decision.
use super::*;
use crate::villager::smelting;

#[test]
fn admitted_decisions_share_town_slice_and_skip_demand_without_furnaces() {
    let (mut state, worker, _) = setup();
    let slot = state.entities.slot_of(worker).unwrap();
    state.block_entities.remove(&FURNACE).unwrap();
    state
        .world
        .set_block(FURNACE.0, FURNACE.1, FURNACE.2, state.registry.states.air);
    let before = counts(&state);
    for _ in 0..16 {
        let mut resident = state.entities.row_mut(slot).villager.take().unwrap();
        assert!(smelting::work(&mut state, slot, &mut resident).is_none());
        state.entities.row_mut(slot).villager = Some(resident);
    }
    assert_eq!(state.smelting_cache.snapshots, 1);
    assert_eq!(state.smelting_cache.demand_scans, 0);
    assert_eq!(counts(&state), before);
    state.tick += 1;
    let mut resident = state.entities.row_mut(slot).villager.take().unwrap();
    assert!(smelting::work(&mut state, slot, &mut resident).is_none());
    state.entities.row_mut(slot).villager = Some(resident);
    assert_eq!(state.smelting_cache.snapshots, 2);
    assert_eq!(state.smelting_cache.demand_scans, 0);
}

#[test]
fn planner_demand_is_shared_for_the_construction_cadence_not_every_tick() {
    let (mut state, worker, _) = setup();
    let slot = state.entities.slot_of(worker).unwrap();
    let before = counts(&state);
    let cadence = crate::settlements::construction::PLAN_CADENCE;
    for tick in 0..cadence {
        state.tick = tick;
        let mut resident = state.entities.row_mut(slot).villager.take().unwrap();
        assert!(smelting::work(&mut state, slot, &mut resident).is_some());
        state.entities.row_mut(slot).villager = Some(resident);
    }
    assert_eq!(state.smelting_cache.snapshots, cadence as usize);
    assert_eq!(
        state.smelting_cache.demand_scans, 2,
        "one cadence plan plus fuel-debit revalidation, not one plan per idle/navigation tick"
    );
    assert_eq!(counts(&state), before);
    state.tick = cadence;
    let mut resident = state.entities.row_mut(slot).villager.take().unwrap();
    assert!(smelting::work(&mut state, slot, &mut resident).is_some());
    state.entities.row_mut(slot).villager = Some(resident);
    assert_eq!(state.smelting_cache.demand_scans, 3);
    assert_eq!(counts(&state), before);
}

fn step(
    state: &mut TickState,
    slot: crate::entities::EntitySlot,
) -> Option<Option<crate::boundary::BlockPos>> {
    let mut resident = state.entities.row_mut(slot).villager.take().unwrap();
    let result = smelting::work(state, slot, &mut resident);
    state.entities.row_mut(slot).villager = Some(resident);
    result
}

#[test]
fn new_wool_supply_cancels_loading_before_the_next_fuel_debit() {
    let (mut state, worker, _) = setup();
    let slot = state.entities.slot_of(worker).unwrap();
    let wool = mining::item_id(&state, "white_wool").unwrap();
    assert_eq!(step(&mut state, slot), Some(None), "fetch the promised ore");
    state.block_entities.get_mut(&CHEST).unwrap().slots[3] = ItemStack::new(wool, 3);
    let before = counts(&state);
    state.tick += 1;
    assert_eq!(
        step(&mut state, slot),
        Some(None),
        "return ore instead of fetching fuel"
    );
    assert!(
        state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .unwrap()
            .smelting
            .is_none()
    );
    assert_eq!(state.block_entities[&CHEST].slots[0].count, 9);
    assert_eq!(state.block_entities[&CHEST].slots[1].count, 3);
    assert!(
        state.block_entities[&FURNACE]
            .slots
            .iter()
            .all(ItemStack::is_empty)
    );
    assert_eq!(counts(&state), before);
}

#[test]
fn new_wool_supply_cancels_loading_before_furnace_insertion() {
    let (mut state, worker, _) = setup();
    let slot = state.entities.slot_of(worker).unwrap();
    let wool = mining::item_id(&state, "white_wool").unwrap();
    assert_eq!(step(&mut state, slot), Some(None));
    state.tick += 1;
    assert_eq!(step(&mut state, slot), Some(None));
    state.block_entities.get_mut(&CHEST).unwrap().slots[3] = ItemStack::new(wool, 3);
    state.entities.set_pos(slot, [10.5, 201.0, 3.5]);
    let before = counts(&state);
    state.tick += 1;
    assert!(
        step(&mut state, slot).unwrap().is_some(),
        "return supplies to distant storage"
    );
    assert!(
        state.block_entities[&FURNACE]
            .slots
            .iter()
            .all(ItemStack::is_empty)
    );
    state.entities.set_pos(slot, [3.5, 201.0, 3.5]);
    state.tick += 1;
    assert_eq!(step(&mut state, slot), Some(None));
    assert_eq!(state.block_entities[&CHEST].slots[0].count, 9);
    assert_eq!(state.block_entities[&CHEST].slots[1].count, 3);
    assert_eq!(counts(&state), before);
}
