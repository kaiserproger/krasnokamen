use super::VillagerProfession;

#[test]
fn builder_profession_has_native_identity_and_mason_appearance() {
    let builder = VillagerProfession::from_name("solaris:builder")
        .expect("settlement builders have a native persisted profession");
    assert_eq!(builder.name(), "builder");
    assert_eq!(
        builder.profession_id(),
        VillagerProfession::Mason.profession_id()
    );
    assert_eq!(builder.title_name(), "mason");
    assert!(VillagerProfession::from_name("minecraft:builder").is_none());
}

#[test]
fn farmer_delivers_surplus_but_keeps_breeding_ration() {
    let (mut state, slot, mut v) =
        super::mining_tests::fixture(crate::settlements::buildings::BuildingKind::Mine);
    super::mining_tests::storage(&mut state, &v, [4, 201, 3]);
    v.data.profession = VillagerProfession::Farmer;
    let bread = super::mining::item_id(&state, "bread").unwrap();
    v.inventory[0] = Some(mc_data::ItemStack::new(bread, 7));
    assert!(super::harvest::deposit(&mut state, slot, &mut v).is_none());
    assert_eq!(
        v.inventory
            .iter()
            .flatten()
            .map(|stack| stack.count)
            .sum::<i32>(),
        3
    );
    assert_eq!(
        state.block_entities[&(4, 201, 3)]
            .slots
            .iter()
            .map(|stack| stack.count)
            .sum::<i32>(),
        4
    );
    assert!(super::harvest::deposit(&mut state, slot, &mut v).is_none());
    assert_eq!(
        state.block_entities[&(4, 201, 3)]
            .slots
            .iter()
            .map(|stack| stack.count)
            .sum::<i32>(),
        4
    );
}

#[test]
fn bound_farmer_keeps_raw_wheat_once_food_reserve_is_full() {
    let (mut state, slot, mut v) = super::mining_tests::fixture(
        crate::settlements::buildings::BuildingKind::Workshop(VillagerProfession::Farmer),
    );
    v.data.profession = VillagerProfession::Farmer;
    let bread = super::mining::item_id(&state, "bread").unwrap();
    let wheat = super::mining::item_id(&state, "wheat").unwrap();
    v.inventory[0] = Some(mc_data::ItemStack::new(bread, 3));
    v.inventory[1] = Some(mc_data::ItemStack::new(wheat, 2));
    let crop =
        crate::settlements::allocator::tests::block(&state, "minecraft:wheat", &[("age", "7")]);
    state.world.set_block(4, 201, 3, crop);
    let mut changes = crate::boundary::ChangeSet::default();
    for _ in 0..8 {
        super::harvest::work(&mut state, slot, &mut v, &mut changes);
        state.tick += 40;
    }
    assert_eq!(
        v.inventory
            .iter()
            .flatten()
            .filter(|stack| stack.item_id == bread)
            .map(|stack| stack.count)
            .sum::<i32>(),
        3
    );
    assert_eq!(
        v.inventory
            .iter()
            .flatten()
            .filter(|stack| stack.item_id == wheat)
            .map(|stack| stack.count)
            .sum::<i32>(),
        3
    );
    let seeds = super::mining::item_id(&state, "wheat_seeds").unwrap();
    assert_eq!(
        v.inventory
            .iter()
            .flatten()
            .filter(|stack| stack.item_id == seeds)
            .map(|stack| stack.count)
            .sum::<i32>(),
        1
    );
}

#[test]
fn farmer_refuses_harvest_if_net_seed_and_grain_do_not_both_fit() {
    let (mut state, slot, mut v) = super::mining_tests::fixture(
        crate::settlements::buildings::BuildingKind::Workshop(VillagerProfession::Farmer),
    );
    v.data.profession = VillagerProfession::Farmer;
    let cobble = super::mining::item_id(&state, "cobblestone").unwrap();
    let wheat = super::mining::item_id(&state, "wheat").unwrap();
    v.inventory = std::array::from_fn(|_| Some(mc_data::ItemStack::new(cobble, 64)));
    v.inventory[7] = Some(mc_data::ItemStack::new(wheat, 63));
    let before = v.inventory.clone();
    let crop =
        crate::settlements::allocator::tests::block(&state, "minecraft:wheat", &[("age", "7")]);
    state.world.set_block(4, 201, 3, crop);
    let mut changes = crate::boundary::ChangeSet::default();
    super::harvest::work(&mut state, slot, &mut v, &mut changes);
    assert_eq!(v.inventory, before);
    assert_eq!(state.world.block_at(4, 201, 3), Some(crop));
    assert!(changes.blocks.is_empty());
}

#[test]
fn production_inputs_survive_farmer_work_and_deposit_until_completion_or_cancel() {
    use crate::settlements::{buildings::BuildingKind, commands::EconomyDomain, economy_commands};
    use mc_data::ItemStack;

    let (mut state, slot, mut villager) =
        super::mining_tests::fixture(BuildingKind::Workshop(VillagerProfession::Farmer));
    crate::plugin_startup::apply(
        std::sync::Arc::make_mut(&mut state.registry),
        &mc_plugin_host::DeploymentContribution::default(),
        None,
        &[],
    )
    .unwrap();
    villager.data.profession = VillagerProfession::Farmer;
    // The allocator validates the Farmer POI during the production tick.
    // A crafting table is a recipe station, not a Farmer's composter job site.
    let composter = super::mining_tests::block(&state, "composter");
    let table = super::mining_tests::block(&state, "crafting_table");
    state.world.set_block(3, 201, 3, composter);
    state.world.set_block(2, 201, 3, table);
    let settlement = villager.settlement.unwrap();
    let building = (settlement.0 << 32) | 1;
    super::mining_tests::storage(&mut state, &villager, [4, 201, 3]);
    let wheat = super::mining::item_id(&state, "wheat").unwrap();
    let cobble = super::mining::item_id(&state, "cobblestone").unwrap();
    villager.inventory[0] = Some(ItemStack::new(wheat, 5));
    villager.inventory[1] = Some(ItemStack::new(cobble, 1));
    let worker = state.entities.row(slot).uuid;
    state.entities.row_mut(slot).villager = Some(villager);
    let start = |world: &mut crate::tick::TickState, name: &str| {
        economy_commands::execute(
            world,
            settlement,
            EconomyDomain::Production,
            &[
                "start".into(),
                name.into(),
                worker.to_string(),
                "solaris-settlements:bread".into(),
                "1".into(),
                building.to_string(),
                "2,201,3".into(),
                "3,201,4".into(),
            ],
        )
        .unwrap();
    };
    start(&mut state, "bread-input");
    let mut villager = state.entities.row_mut(slot).villager.take().unwrap();
    let crop =
        crate::settlements::allocator::tests::block(&state, "minecraft:wheat", &[("age", "7")]);
    state.world.set_block(4, 201, 4, crop);
    let mut changes = crate::boundary::ChangeSet::default();
    for _ in 0..8 {
        super::harvest::work(&mut state, slot, &mut villager, &mut changes);
        state.tick += 40;
    }
    let wheat_in_carry = |v: &super::VillagerState| {
        v.inventory
            .iter()
            .flatten()
            .filter(|s| s.item_id == wheat)
            .map(|s| s.count)
            .sum::<i32>()
    };
    assert_eq!(
        wheat_in_carry(&villager),
        3,
        "only surplus wheat may be baked"
    );
    super::harvest::deposit(&mut state, slot, &mut villager);
    assert_eq!(
        wheat_in_carry(&villager),
        3,
        "job wheat must not be deposited"
    );
    assert!(
        state.block_entities[&(4, 201, 3)]
            .slots
            .iter()
            .any(|s| s.item_id == cobble && s.count == 1)
    );
    state.entities.row_mut(slot).villager = Some(villager);
    state.tick += 1000;
    state.tick_budget.reset(state.tick);
    crate::settlements::tick(&mut state, &mut changes);
    let job = economy_commands::execute(
        &mut state,
        settlement,
        EconomyDomain::Production,
        &["job".into(), "bread-input".into()],
    )
    .unwrap();
    let job: serde_json::Value = serde_json::from_str(&job).unwrap();
    assert_eq!(job["done"], 1);
    assert_eq!(job["receipts"].as_array().unwrap().len(), 1);
    let current = state.entities.row(slot).villager.as_ref().unwrap();
    assert_eq!(
        current.data.profession,
        VillagerProfession::Farmer,
        "valid Farmer POI survives allocation"
    );
    // A finished portion still owns its worker until the output is stored.
    state.entities.set_pos(slot, [4.5, 201.0, 3.5]);
    state.tick += 1000;
    state.tick_budget.reset(state.tick);
    crate::settlements::tick(&mut state, &mut changes);
    assert!(
        state
            .settlements
            .get(settlement)
            .unwrap()
            .economy
            .production
            .jobs["bread-input"]
            .delivery_complete()
    );
    let mut villager = state.entities.row_mut(slot).villager.take().unwrap();
    assert_eq!(wheat_in_carry(&villager), 0);
    villager.inventory[0] = Some(ItemStack::new(wheat, 3));
    villager.inventory[1] = Some(ItemStack::new(
        super::mining::item_id(&state, "bread").unwrap(),
        3,
    ));
    state.entities.row_mut(slot).villager = Some(villager);
    start(&mut state, "cancel-input");
    economy_commands::execute(
        &mut state,
        settlement,
        EconomyDomain::Production,
        &["cancel".into(), "cancel-input".into()],
    )
    .unwrap();
    let mut villager = state.entities.row_mut(slot).villager.take().unwrap();
    super::harvest::deposit(&mut state, slot, &mut villager);
    assert_eq!(wheat_in_carry(&villager), 0, "cancel releases the input");
    assert!(
        state.block_entities[&(4, 201, 3)]
            .slots
            .iter()
            .any(|s| s.item_id == wheat && s.count == 3)
    );
}
