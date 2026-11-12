//! Real production admission/portion plus actual social consumption functions.
//! Authored test recipe stresses reservation size; not live bread-production proof.
use super::*;

#[test]
fn paused_output_obligation_survives_sharing_and_courtship_consumption() {
    use crate::settlements::{buildings::BuildingKind, commands::EconomyDomain, economy_commands};
    let (mut state, slot, mut v) =
        crate::villager::mining_tests::fixture(BuildingKind::Workshop(Profession::Farmer));
    crate::plugin_startup::apply(
        Arc::make_mut(&mut state.registry),
        &mc_plugin_host::DeploymentContribution::default(),
        None,
        &[],
    )
    .unwrap();
    let mut recipe = state
        .registry
        .item_facts
        .work_recipes()
        .find(|recipe| recipe.id.as_str() == "krasnokamen-settlements:bread")
        .unwrap()
        .clone();
    recipe.id = mc_data::Identifier::parse("test:delivery_food").unwrap();
    recipe.outputs[0].count = 40;
    recipe.duration_ticks = 1;
    let registry = Arc::make_mut(&mut state.registry);
    registry.item_facts = registry
        .item_facts
        .clone()
        .with_work_recipes([recipe], &registry.items)
        .unwrap();
    let settlement = v.settlement.unwrap();
    let worker = state.entities.row(slot).uuid;
    let bread = item(&state, "minecraft:bread");
    let wheat = item(&state, "minecraft:wheat");
    let carrot = item(&state, "minecraft:carrot");
    v.inventory[0] = Some(ItemStack::new(bread, 3));
    v.inventory[1] = Some(ItemStack::new(wheat, 3));
    v.inventory[2] = Some(ItemStack::new(carrot, 16));
    v.age = 0;
    v.food_level = 0;
    state.entities.row_mut(slot).villager = Some(v);
    state.entities.row_mut(slot).controls.no_ai = true;
    economy_commands::execute(
        &mut state,
        settlement,
        EconomyDomain::Production,
        &[
            "start".into(),
            "food-delivery".into(),
            worker.to_string(),
            "test:delivery_food".into(),
            "1".into(),
            ((settlement.0 << 32) | 1).to_string(),
            "3,201,3".into(),
            "3,201,4".into(),
        ],
    )
    .unwrap();
    for _ in 0..20 {
        let _ = crate::tick::run_tick(&mut state, &[]);
        if state
            .settlements
            .get(settlement)
            .unwrap()
            .economy
            .production
            .jobs["food-delivery"]
            .done
            == 1
        {
            break;
        }
    }
    assert_eq!(
        state
            .settlements
            .get(settlement)
            .unwrap()
            .economy
            .production
            .jobs["food-delivery"]
            .done,
        1
    );
    economy_commands::execute(
        &mut state,
        settlement,
        EconomyDomain::Production,
        &["pause".into(), "food-delivery".into()],
    )
    .unwrap();
    assert_eq!(
        crate::settlements::economy::target(&state, Some(settlement), worker),
        None
    );
    let _recipient = villager(&mut state, [4.0, 201.0, 3.0]);
    let mut food = FoodTable::default();
    food.refresh(&state.registry.items);
    state.tick = 20;
    let mut changes = ChangeSet::default();
    let mut v = state.entities.row_mut(slot).villager.take().unwrap();
    let protected = crate::settlements::economy::protected_carry(&state, slot, &v);
    assert_eq!(
        protected[0], 43,
        "same-component pool remains reserved even with no route"
    );
    assert_eq!(
        protected[2], 0,
        "unrelated food is still available for ordinary birth"
    );
    share_food(&mut state, &food, slot, &mut v, &mut changes);
    assert_eq!(v.inventory[0].as_ref().unwrap().count, 43);
    eat_for_birth(&food, &mut v, &protected);
    assert_eq!(v.inventory[0].as_ref().unwrap().count, 43);
    assert_eq!(v.inventory[2].as_ref().unwrap().count, 4);
    assert_eq!(v.food_level, 0);
    state.entities.row_mut(slot).villager = Some(v);
    let job = &state
        .settlements
        .get(settlement)
        .unwrap()
        .economy
        .production
        .jobs["food-delivery"];
    assert_eq!(job.delivery.as_ref().unwrap().lots[0].stack.count, 40);
    assert_eq!(job.delivery.as_ref().unwrap().lots[0].retained, 3);
    assert_eq!(job.delivery.as_ref().unwrap().sequence, 0);
    assert!(!job.delivery_complete());
    economy_commands::execute(
        &mut state,
        settlement,
        EconomyDomain::Production,
        &["resume".into(), "food-delivery".into()],
    )
    .unwrap();
    let _ = crate::tick::run_tick(&mut state, &[]);
    assert_eq!(food_count(&state, slot, bread), 43);
    assert!(
        !state
            .settlements
            .get(settlement)
            .unwrap()
            .economy
            .production
            .jobs["food-delivery"]
            .delivery_complete()
    );
}
