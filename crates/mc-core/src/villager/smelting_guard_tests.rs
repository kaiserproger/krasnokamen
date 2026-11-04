use super::*;
use crate::boundary::BlockPos;
use crate::entities::EntitySlot;
use crate::villager::{Activity, smelting};

fn step(state: &mut TickState, slot: EntitySlot) -> Option<Option<BlockPos>> {
    let mut worker = state.entities.row_mut(slot).villager.take().unwrap();
    let result = smelting::work(state, slot, &mut worker);
    state.entities.row_mut(slot).villager = Some(worker);
    result
}

fn carried(state: &TickState, slot: EntitySlot, item: u32) -> i32 {
    state
        .entities
        .row(slot)
        .villager
        .as_ref()
        .unwrap()
        .inventory
        .iter()
        .flatten()
        .filter(|stack| stack.item_id == item)
        .map(|stack| stack.count)
        .sum()
}

fn stored(state: &TickState, item: u32) -> i32 {
    state.block_entities[&CHEST]
        .slots
        .iter()
        .filter(|stack| stack.item_id == item)
        .map(|stack| stack.count)
        .sum()
}

fn town(state: &TickState, slot: EntitySlot) -> crate::settlements::SettlementId {
    state
        .entities
        .row(slot)
        .villager
        .as_ref()
        .unwrap()
        .settlement
        .unwrap()
}

fn at_chest(state: &mut TickState, slot: EntitySlot) {
    state.entities.set_pos(slot, [3.5, 201.0, 3.5]);
}

fn at_furnace(state: &mut TickState, slot: EntitySlot) {
    state.entities.set_pos(slot, [10.5, 201.0, 3.5]);
}

#[test]
fn satisfied_bill_does_not_withdraw_or_load_iron() {
    let (mut state, worker, _) = setup();
    let slot = state.entities.slot_of(worker).unwrap();
    let id = town(&state, slot);
    let wool = mining::item_id(&state, "white_wool").unwrap();
    state.block_entities.get_mut(&CHEST).unwrap().slots[3] = ItemStack::new(wool, 3);
    assert_eq!(
        crate::settlements::construction::iron_demand(&state, id, None),
        0
    );
    let before = counts(&state);
    let chest = state.block_entities[&CHEST].slots.clone();
    let furnace = state.block_entities[&FURNACE].slots.clone();
    assert!(step(&mut state, slot).is_none());
    assert_eq!(state.block_entities[&CHEST].slots, chest);
    assert_eq!(state.block_entities[&FURNACE].slots, furnace);
    assert_eq!(counts(&state), before);
}

#[test]
fn uncertified_furnace_is_not_touched_or_supplied() {
    let (mut state, worker, _) = setup();
    let slot = state.entities.slot_of(worker).unwrap();
    let id = town(&state, slot);
    let record = state.settlements.get_mut(id).unwrap();
    record.bootstrap_storage = Some([CHEST.0, CHEST.1, CHEST.2]);
    record.buildings.clear();
    assert_eq!(
        crate::settlements::construction::iron_demand(&state, id, None),
        2
    );
    assert!(smelting::owned_furnaces(&state, id).next().is_none());
    let before = counts(&state);
    let chest = state.block_entities[&CHEST].slots.clone();
    let furnace = state.block_entities[&FURNACE].slots.clone();
    assert!(step(&mut state, slot).is_none());
    assert_eq!(state.block_entities[&CHEST].slots, chest);
    assert_eq!(state.block_entities[&FURNACE].slots, furnace);
    assert_eq!(counts(&state), before);
}

#[test]
fn two_tenders_share_one_two_iron_demand() {
    let (mut state, first, _) = setup();
    let first = state.entities.slot_of(first).unwrap();
    let id = town(&state, first);
    let villager_type = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse("minecraft:villager").unwrap())
        .unwrap();
    let second_id = state.spawn_mob(villager_type, [3.5, 201.0, 4.5], 20.0);
    crate::settlements::bind(&mut state, id, second_id).unwrap();
    let second = state.entities.slot_of(second_id).unwrap();
    let resident = state.entities.row_mut(second).villager.as_mut().unwrap();
    resident.data.profession = VillagerProfession::None;
    resident.activity = Activity::Work;
    let raw = mining::item_id(&state, "raw_iron").unwrap();
    assert_eq!(
        crate::settlements::construction::iron_demand(&state, id, None),
        2
    );
    let before = counts(&state);
    assert_eq!(step(&mut state, first), Some(None));
    assert_eq!(carried(&state, first, raw), 2);
    assert!(step(&mut state, second).is_none());
    assert_eq!(carried(&state, second, raw), 0);
    assert_eq!(stored(&state, raw), 7);
    assert_eq!(counts(&state), before);
    assert_eq!(state.smelting_cache.snapshots, 1);
    assert_eq!(state.smelting_cache.demand_scans, 1);
}

#[test]
fn real_furnace_output_is_carried_to_storage_without_tender_crafting() {
    let (mut state, worker, _) = setup();
    let slot = state.entities.slot_of(worker).unwrap();
    let raw = mining::item_id(&state, "raw_iron").unwrap();
    let iron = mining::item_id(&state, "iron_ingot").unwrap();
    let shears = mining::item_id(&state, "shears").unwrap();
    assert_eq!(step(&mut state, slot), Some(None)); // withdraw ore
    assert_eq!(carried(&state, slot, raw), 2);
    assert_eq!(step(&mut state, slot), Some(None)); // withdraw fuel
    at_furnace(&mut state, slot);
    assert_eq!(step(&mut state, slot), Some(None)); // load furnace
    assert_eq!(state.block_entities[&FURNACE].slots[0].count, 2);
    assert_eq!(step(&mut state, slot), Some(None)); // finish empty delivery
    let registry = std::sync::Arc::clone(&state.registry);
    let rules = registry.item_rules();
    for _ in 0..400 {
        let furnace = state.block_entities.get_mut(&FURNACE).unwrap();
        furnace.furnace.as_mut().unwrap().tick(
            &rules,
            mc_data::recipes::CookingKind::Furnace,
            &mut furnace.slots,
        );
    }
    assert_eq!(state.block_entities[&FURNACE].slots[2].item_id, iron);
    assert_eq!(state.block_entities[&FURNACE].slots[2].count, 2);
    let before = counts(&state);
    assert_eq!(step(&mut state, slot), Some(None)); // take actual output
    assert_eq!(carried(&state, slot, iron), 2);
    assert_eq!(stored(&state, iron), 0);
    let destination = step(&mut state, slot)
        .expect("delivery owns the turn")
        .expect("storage is distant");
    assert_eq!((destination.x, destination.y, destination.z), CHEST);
    assert_eq!(carried(&state, slot, iron), 2);
    assert_eq!(carried(&state, slot, shears), 0);
    at_chest(&mut state, slot);
    assert_eq!(step(&mut state, slot), Some(None));
    assert_eq!(stored(&state, iron), 2);
    assert_eq!(carried(&state, slot, iron), 0);
    assert_eq!(carried(&state, slot, shears), 0);
    assert_eq!(counts(&state), before);
}

#[test]
fn revoked_furnace_certificate_returns_loaded_carry_to_storage() {
    let (mut state, worker, _) = setup();
    let slot = state.entities.slot_of(worker).unwrap();
    let id = town(&state, slot);
    let raw = mining::item_id(&state, "raw_iron").unwrap();
    let coal = mining::item_id(&state, "coal").unwrap();
    let before = counts(&state);
    assert_eq!(step(&mut state, slot), Some(None));
    assert_eq!(step(&mut state, slot), Some(None));
    assert_eq!(carried(&state, slot, raw), 2);
    assert_eq!(carried(&state, slot, coal), 1);
    let record = state.settlements.get_mut(id).unwrap();
    record.bootstrap_storage = Some([CHEST.0, CHEST.1, CHEST.2]);
    record.buildings.clear();
    at_furnace(&mut state, slot);
    let destination = step(&mut state, slot)
        .expect("delivery owns the turn")
        .expect("storage is distant");
    assert_eq!((destination.x, destination.y, destination.z), CHEST);
    assert_eq!(carried(&state, slot, raw), 2);
    assert_eq!(carried(&state, slot, coal), 1);
    assert!(
        state.block_entities[&FURNACE]
            .slots
            .iter()
            .all(ItemStack::is_empty)
    );
    at_chest(&mut state, slot);
    assert_eq!(step(&mut state, slot), Some(None));
    assert_eq!(stored(&state, raw), 9);
    assert_eq!(stored(&state, coal), 3);
    assert_eq!(carried(&state, slot, raw), 0);
    assert_eq!(carried(&state, slot, coal), 0);
    assert_eq!(counts(&state), before);
}

#[test]
fn another_towns_ingots_do_not_satisfy_our_iron_pipeline() {
    let (mut state, worker, _) = setup();
    let slot = state.entities.slot_of(worker).unwrap();
    let other_town = state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "other".into(),
            "other-owner".into(),
            [8, 201, 8],
            12,
        )
        .unwrap();
    let villager = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse("minecraft:villager").unwrap())
        .unwrap();
    let other = state.spawn_mob(villager, [5.5, 201.0, 5.5], 20.0);
    crate::settlements::bind(&mut state, other_town, other).unwrap();
    let other = state.entities.slot_of(other).unwrap();
    let iron = mining::item_id(&state, "iron_ingot").unwrap();
    let raw = mining::item_id(&state, "raw_iron").unwrap();
    state
        .entities
        .row_mut(other)
        .villager
        .as_mut()
        .unwrap()
        .inventory[0] = Some(ItemStack::new(iron, 2));
    let before = counts(&state);
    assert_eq!(step(&mut state, slot), Some(None));
    assert_eq!(carried(&state, slot, raw), 2);
    assert_eq!(carried(&state, other, iron), 2);
    assert_eq!(counts(&state), before);
}

#[test]
fn reserved_ore_and_fuel_in_first_storage_do_not_block_a_free_second_source() {
    use mc_script::*;
    let (mut state, worker, _) = setup();
    let slot = state.entities.slot_of(worker).unwrap();
    let id = town(&state, slot);
    let raw = mining::item_id(&state, "raw_iron").unwrap();
    let coal = mining::item_id(&state, "coal").unwrap();
    let second = [4, 201, 7];
    state.world.set_block(
        second[0],
        second[1],
        second[2],
        mining_tests::block(&state, "chest"),
    );
    let mut slots = vec![ItemStack::EMPTY; 27];
    slots[0] = ItemStack::new(raw, 2);
    slots[1] = ItemStack::new(coal, 1);
    state.block_entities.insert(
        (second[0], second[1], second[2]),
        crate::tick::BlockEntityState {
            slots,
            furnace: None,
        },
    );
    let record = state.settlements.get_mut(id).unwrap();
    let mut building = record.buildings.values().next().unwrap().clone();
    building.id = crate::settlements::buildings::BuildingId((id.0 << 32) | 3);
    building.bounds = ScriptWorldBounds::new(second, second).unwrap();
    building.work_points = vec![second];
    record.buildings.insert(building.id, building);
    record.building_sequence = 3;
    state.world.fold_overlay();
    let endpoint = ScriptInventoryEndpoint::Container {
        dimension: state.dimension.name().into(),
        position: ScriptBlockPosition::new(CHEST.0, CHEST.1, CHEST.2),
    };
    let fence = crate::plugin_inventories::load(&state, "owner", &endpoint)
        .unwrap()
        .fence;
    let request = ScriptOperationRequest::try_new(
        "smelt-reserve",
        ScriptOperation::Inventory {
            operation: ScriptOwnedInventoryOperation::Reserve {
                operation_id: "smelt-reserve".into(),
                endpoint,
                resource_plan: ScriptInventoryResourcePlan::new(vec![
                    ScriptInventoryWorkPortion::new(
                        1,
                        vec![
                            ScriptInventoryMaterial::new("minecraft:raw_iron".into(), 9),
                            ScriptInventoryMaterial::new("minecraft:coal".into(), 3),
                        ],
                    ),
                ]),
                expected_revision: fence,
            },
        },
    )
    .unwrap();
    assert_eq!(
        crate::plugin_inventories::execute(&mut state, "owner", &request).failure(),
        None
    );
    let before = counts(&state);
    let first_stock = state.block_entities[&CHEST].slots.clone();
    let destination = step(&mut state, slot).unwrap().unwrap();
    assert_eq!([destination.x, destination.y, destination.z], second);
    state.entities.set_pos(slot, [4.5, 201.0, 6.5]);
    assert_eq!(step(&mut state, slot), Some(None));
    assert_eq!(step(&mut state, slot), Some(None));
    assert_eq!(carried(&state, slot, raw), 2);
    assert_eq!(carried(&state, slot, coal), 1);
    assert_eq!(state.block_entities[&CHEST].slots, first_stock);
    assert_eq!(counts(&state), before);
}

#[test]
fn failed_storage_approach_selects_and_keeps_an_alternate_source() {
    let (mut state, worker, _) = setup();
    let slot = state.entities.slot_of(worker).unwrap();
    let id = town(&state, slot);
    let raw = mining::item_id(&state, "raw_iron").unwrap();
    let second = [8, 201, 5];
    state.world.set_block(
        second[0],
        second[1],
        second[2],
        mining_tests::block(&state, "chest"),
    );
    let mut slots = vec![ItemStack::EMPTY; 27];
    slots[0] = ItemStack::new(raw, 2);
    slots[1] = ItemStack::new(mining::item_id(&state, "coal").unwrap(), 1);
    state.block_entities.insert(
        (second[0], second[1], second[2]),
        crate::tick::BlockEntityState {
            slots,
            furnace: None,
        },
    );
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .buildings
        .values_mut()
        .next()
        .unwrap()
        .work_points
        .push(second);
    state.entities.set_pos(slot, [1.5, 201.0, 10.5]);
    let resident = state.entities.row_mut(slot).villager.as_mut().unwrap();
    resident.navigation_target = Some(BlockPos {
        x: CHEST.0,
        y: CHEST.1,
        z: CHEST.2,
    });
    resident.navigation_search_failed = true;
    let before = counts(&state);
    let destination = step(&mut state, slot).unwrap().unwrap();
    assert_eq!([destination.x, destination.y, destination.z], second);
    let resident = state.entities.row_mut(slot).villager.as_mut().unwrap();
    resident.navigation_target = Some(destination);
    resident.navigation_search_failed = false;
    assert_eq!(
        step(&mut state, slot),
        Some(Some(destination)),
        "do not oscillate back to the first source"
    );
    assert_eq!(counts(&state), before);
}

#[test]
fn failed_furnace_approach_keeps_real_carry_and_selects_another_owned_furnace() {
    let (mut state, worker, _) = setup();
    let slot = state.entities.slot_of(worker).unwrap();
    assert_eq!(step(&mut state, slot), Some(None));
    assert_eq!(step(&mut state, slot), Some(None));
    let alternate = [9, 201, 5];
    state.world.set_block(
        alternate[0],
        alternate[1],
        alternate[2],
        mining_tests::block(&state, "furnace"),
    );
    state.block_entities.insert(
        (alternate[0], alternate[1], alternate[2]),
        crate::tick::BlockEntityState {
            slots: vec![ItemStack::EMPTY; 3],
            furnace: Some(Default::default()),
        },
    );
    let resident = state.entities.row_mut(slot).villager.as_mut().unwrap();
    resident.navigation_target = Some(BlockPos {
        x: FURNACE.0,
        y: FURNACE.1,
        z: FURNACE.2,
    });
    resident.navigation_search_failed = true;
    let before = counts(&state);
    let destination = step(&mut state, slot).unwrap().unwrap();
    assert_eq!([destination.x, destination.y, destination.z], alternate);
    let resident = state.entities.row(slot).villager.as_ref().unwrap();
    assert_eq!(resident.smelting.unwrap().furnace, alternate);
    assert_eq!(counts(&state), before);
}
