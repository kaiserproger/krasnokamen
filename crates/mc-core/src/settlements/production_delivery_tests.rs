//! Candidate unit regressions; fixture moves are NOT graphical route proof.
use super::*;

fn owed(policy: &State, name: &str) -> i32 {
    policy.jobs[name]
        .delivery
        .as_ref()
        .unwrap()
        .lots
        .iter()
        .map(|lot| lot.stack.count)
        .sum()
}

fn stored(world: &TickState, chest: [i32; 3], name: &str) -> i32 {
    world.block_entities[&(chest[0], chest[1], chest[2])]
        .slots
        .iter()
        .filter(|stack| resource(world, stack) == Some(name))
        .map(|stack| stack.count)
        .sum()
}

fn due(world: &mut TickState, town: SettlementId, policy: &mut State, name: &str) {
    world.tick = policy.jobs[name].next_due;
    assert!(tick(world, town, policy));
}

fn beside(world: &mut TickState, body: EntityId, chest: [i32; 3]) {
    at(
        world,
        body,
        [
            f64::from(chest[0]) + 0.5,
            f64::from(chest[1]),
            f64::from(chest[2]) + 1.5,
        ],
    );
}

#[test]
fn terminal_preserves_three_bread_ration_and_unstorable_unrelated_cargo() {
    let (mut world, town, body, worker, table, stand, worksite) = setup();
    let chest = supply_chest(&mut world, town, 0);
    let cobble = item(&world, "minecraft:cobblestone", 64);
    let old_bread = item(&world, "minecraft:bread", 63);
    let mut slots = vec![cobble; 27];
    slots[0] = old_bread;
    world
        .block_entities
        .get_mut(&(chest[0], chest[1], chest[2]))
        .unwrap()
        .slots = slots;
    give(
        vec![
            item(&world, "minecraft:bread", 3),
            item(&world, "minecraft:wheat", 3),
            item(&world, "minecraft:coal", 64),
        ],
        &mut world,
        body,
    );
    let mut policy = bread_order(&mut world, town, worker, table, stand, &worksite);
    due(&mut world, town, &mut policy, "supply");
    assert_eq!(owed(&policy, "supply"), 1);
    assert_eq!(
        policy.jobs["supply"].delivery.as_ref().unwrap().lots[0].retained,
        3
    );
    assert_eq!(stock_total(&world, body, "minecraft:bread"), 4);
    due(&mut world, town, &mut policy, "supply");
    assert_eq!(
        stored(&world, chest, "minecraft:bread"),
        63,
        "remote intention is not a store"
    );
    beside(&mut world, body, chest);
    due(&mut world, town, &mut policy, "supply");
    assert_eq!(owed(&policy, "supply"), 0);
    assert!(policy.jobs["supply"].delivery_complete());
    assert!(!active(&policy));
    assert_eq!(stock_total(&world, body, "minecraft:bread"), 3);
    assert_eq!(stock_total(&world, body, "minecraft:coal"), 64);
    assert_eq!(stored(&world, chest, "minecraft:bread"), 64);
    assert_eq!(stored(&world, chest, "minecraft:coal"), 0);
    assert_eq!(policy.completed, 1);
    assert_eq!(policy.jobs["supply"].delivery.as_ref().unwrap().sequence, 1);
    let before = carry(&world, body);
    world.tick += 100;
    assert!(!tick(&mut world, town, &mut policy));
    assert_eq!(carry(&world, body), before);
    assert_eq!(stored(&world, chest, "minecraft:bread"), 64);
}

#[test]
fn partial_output_storage_preserves_tool_and_releases_without_shipping_it() {
    let (mut world, town, body, worker, table, stand_at, worksite) = setup();
    let chest = supply_chest(&mut world, town, 0);
    let cobble = item(&world, "minecraft:cobblestone", 64);
    let mut slots = vec![cobble; 27];
    slots[0] = item(&world, "minecraft:oak_planks", 63);
    world
        .block_entities
        .get_mut(&(chest[0], chest[1], chest[2]))
        .unwrap()
        .slots = slots;
    give(
        vec![
            item(&world, "minecraft:oak_log", 1),
            item(&world, "minecraft:wooden_axe", 1),
        ],
        &mut world,
        body,
    );
    let mut policy = State::default();
    start(
        &mut world,
        town,
        &mut policy,
        Start {
            job: "planks".into(),
            worker,
            recipe_name: "krasnokamen-settlements:oak_planks".into(),
            count: 1,
            station: table,
            stand_at,
            building: worksite_ref(&worksite),
        },
    )
    .unwrap();
    due(&mut world, town, &mut policy, "planks");
    let axe = carry(&world, body)
        .into_iter()
        .find(|stack| resource(&world, stack) == Some("minecraft:wooden_axe"))
        .unwrap();
    assert_eq!(axe.damage, Some(1));
    assert_eq!(owed(&policy, "planks"), 4);
    beside(&mut world, body, chest);
    due(&mut world, town, &mut policy, "planks");
    assert_eq!(owed(&policy, "planks"), 3);
    assert_eq!(stock_total(&world, body, "minecraft:oak_planks"), 3);
    assert_eq!(stored(&world, chest, "minecraft:oak_planks"), 64);
    assert_eq!(stored(&world, chest, "minecraft:wooden_axe"), 0);
    assert!(active(&policy));
    let receipts = policy.jobs["planks"].receipts.clone();
    due(&mut world, town, &mut policy, "planks");
    assert_eq!(owed(&policy, "planks"), 3);
    assert_eq!(
        policy.jobs["planks"].paused.as_deref(),
        Some("no output storage")
    );
    // Ordinary storage-owner mutation in the unit fixture, not an output spawn.
    // Live QA must free space by a real graphical inventory transfer.
    world
        .block_entities
        .get_mut(&(chest[0], chest[1], chest[2]))
        .unwrap()
        .slots[1] = ItemStack::EMPTY;
    due(&mut world, town, &mut policy, "planks");
    assert_eq!(owed(&policy, "planks"), 0);
    assert_eq!(stored(&world, chest, "minecraft:oak_planks"), 67);
    assert!(carry(&world, body).iter().any(|stack| stack == &axe));
    assert_eq!(policy.jobs["planks"].receipts, receipts);
    assert_eq!(policy.jobs["planks"].delivery.as_ref().unwrap().sequence, 2);
    assert_eq!(policy.completed, 1);
    assert!(!active(&policy));
    // Preserved carry tool actually supports a subsequent ordinary job.
    let mut next_carry = carry(&world, body);
    let empty = next_carry.iter().position(ItemStack::is_empty).unwrap();
    next_carry[empty] = item(&world, "minecraft:oak_log", 1);
    give(next_carry, &mut world, body);
    start(
        &mut world,
        town,
        &mut policy,
        Start {
            job: "next-planks".into(),
            worker,
            recipe_name: "krasnokamen-settlements:oak_planks".into(),
            count: 1,
            station: table,
            stand_at,
            building: worksite_ref(&worksite),
        },
    )
    .unwrap();
    at(&mut world, body, [7.5, 201.0, 8.5]);
    due(&mut world, town, &mut policy, "next-planks");
    assert_eq!(policy.jobs["next-planks"].done, 1);
    assert_eq!(policy.jobs["next-planks"].receipts[0].tool_wear[0].wear, 1);
}

#[test]
fn missing_output_invalidates_obligation_even_if_same_components_are_refilled() {
    let (mut world, town, body, worker, table, stand, worksite) = setup();
    let chest = supply_chest(&mut world, town, 0);
    give(
        vec![
            item(&world, "minecraft:bread", 3),
            item(&world, "minecraft:wheat", 3),
        ],
        &mut world,
        body,
    );
    let mut policy = bread_order(&mut world, town, worker, table, stand, &worksite);
    due(&mut world, town, &mut policy, "supply");
    let slot = world.entities.slot_of(body).unwrap();
    // Adversarial unit corruption: delete just the newly owed quantity from
    // the indistinguishable pool. The retained three may not pay the debt.
    world
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .inventory[0]
        .as_mut()
        .unwrap()
        .count -= 1;
    beside(&mut world, body, chest);
    due(&mut world, town, &mut policy, "supply");
    assert!(policy.jobs["supply"].delivery.as_ref().unwrap().invalidated);
    assert_eq!(owed(&policy, "supply"), 1);
    assert!(!policy.jobs["supply"].delivery_complete());
    assert_eq!(stored(&world, chest, "minecraft:bread"), 0);
    world
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .inventory[0]
        .as_mut()
        .unwrap()
        .count += 1;
    due(&mut world, town, &mut policy, "supply");
    assert_eq!(owed(&policy, "supply"), 1);
    assert_eq!(stored(&world, chest, "minecraft:bread"), 0);
    assert_eq!(policy.jobs["supply"].delivery.as_ref().unwrap().sequence, 0);
    let before = carry(&world, body);
    execute(
        &mut world,
        town,
        "owner",
        &mut policy,
        Request::Cancel {
            job: "supply".into(),
        },
    )
    .unwrap();
    assert_eq!(
        owed(&policy, "supply"),
        1,
        "cancel records unfulfilled debt, never pays it"
    );
    assert!(policy.jobs["supply"].cancelled);
    assert!(!policy.jobs["supply"].delivery_complete());
    assert!(!active(&policy));
    assert_eq!(carry(&world, body), before);
    validate(&policy).unwrap();
}

#[test]
fn authored_remainders_and_different_components_are_not_delivery_obligations() {
    let (mut world, town, body, worker, table, stand_at, worksite) = setup();
    let chest = supply_chest(&mut world, town, 0);
    give(cake_inputs(&world), &mut world, body);
    let mut policy = State::default();
    start(
        &mut world,
        town,
        &mut policy,
        Start {
            job: "cake".into(),
            worker,
            recipe_name: "krasnokamen-settlements:cake".into(),
            count: 1,
            station: table,
            stand_at,
            building: worksite_ref(&worksite),
        },
    )
    .unwrap();
    due(&mut world, town, &mut policy, "cake");
    assert_eq!(owed(&policy, "cake"), 1);
    assert_eq!(stock_total(&world, body, "minecraft:bucket"), 3);
    beside(&mut world, body, chest);
    due(&mut world, town, &mut policy, "cake");
    assert_eq!(stored(&world, chest, "minecraft:cake"), 1);
    assert_eq!(stored(&world, chest, "minecraft:bucket"), 0);
    assert_eq!(stock_total(&world, body, "minecraft:bucket"), 3);

    let mut named = item(&world, "minecraft:bread", 3);
    named.custom_name = Some("old ration".into());
    give(
        vec![named.clone(), item(&world, "minecraft:wheat", 3)],
        &mut world,
        body,
    );
    at(&mut world, body, [7.5, 201.0, 8.5]);
    let mut next = bread_order(&mut world, town, worker, table, stand_at, &worksite);
    due(&mut world, town, &mut next, "supply");
    assert_eq!(
        next.jobs["supply"].delivery.as_ref().unwrap().lots[0].retained,
        0
    );
    beside(&mut world, body, chest);
    due(&mut world, town, &mut next, "supply");
    assert!(carry(&world, body).iter().any(|stack| stack == &named));
    assert!(
        world.block_entities[&(chest[0], chest[1], chest[2])]
            .slots
            .iter()
            .all(|stack| stack.custom_name.as_deref() != Some("old ration"))
    );
}

#[test]
fn missing_storage_remains_unfulfilled_without_stealing_the_ration() {
    let (mut world, town, body, worker, table, stand, worksite) = setup();
    give(
        vec![
            item(&world, "minecraft:bread", 3),
            item(&world, "minecraft:wheat", 3),
        ],
        &mut world,
        body,
    );
    let mut policy = bread_order(&mut world, town, worker, table, stand, &worksite);
    due(&mut world, town, &mut policy, "supply");
    let carry_before = carry(&world, body);
    due(&mut world, town, &mut policy, "supply");
    assert_eq!(
        policy.jobs["supply"].paused.as_deref(),
        Some("no output storage")
    );
    assert_eq!(carry(&world, body), carry_before);
    assert_eq!(owed(&policy, "supply"), 1);
    assert!(!policy.jobs["supply"].delivery_complete());
    let chest = supply_chest(&mut world, town, 0);
    beside(&mut world, body, chest);
    due(&mut world, town, &mut policy, "supply");
    assert_eq!(stock_total(&world, body, "minecraft:bread"), 3);
    assert_eq!(stored(&world, chest, "minecraft:bread"), 1);
}

#[path = "production_delivery_disk_tests.rs"]
mod disk;

#[path = "production_ownership_tests.rs"]
mod ownership;
