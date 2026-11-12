//! Actual production admissions and derived reservation lifecycle.
use super::*;
use crate::settlements::economy;
use crate::villager::VillagerProfession as Profession;

fn publish_policy(world: &mut TickState, town: SettlementId, policy: State) {
    let revision = world
        .settlements
        .get(town)
        .unwrap()
        .revision
        .checked_add(1)
        .unwrap();
    world.settlements.economy_for_tick(town).production = policy;
    world.settlements.finish_economy(town, revision, true);
}

#[test]
fn exclusive_worker_roles_and_furnace_carry_refuse_native_production_start() {
    for (profession, tending) in [
        (Profession::Builder, false),
        (Profession::Soldier, false),
        (Profession::None, true),
    ] {
        let (mut world, town, body, worker, table, stand_at, worksite) = setup();
        let slot = world.entities.slot_of(body).unwrap();
        world
            .entities
            .row_mut(slot)
            .villager
            .as_mut()
            .unwrap()
            .data
            .profession = profession;
        if tending {
            world
                .entities
                .row_mut(slot)
                .villager
                .as_mut()
                .unwrap()
                .smelting = Some(crate::villager::smelting::Task {
                furnace: [8, 201, 8],
                phase: crate::villager::smelting::Phase::Load,
                carried: Default::default(),
            });
        }
        let before = carry(&world, body);
        let mut policy = State::default();
        let start_request = || Request::Start {
            job: "exclusive".into(),
            worker,
            recipe: "krasnokamen-settlements:bread".into(),
            count: 1,
            station: table,
            stand_at,
            building: worksite_ref(&worksite),
        };
        assert_eq!(
            execute(&mut world, town, "owner", &mut policy, start_request()),
            Err(Failure::Busy)
        );
        assert!(policy.jobs.is_empty());
        assert!(
            economy::execute(
                &mut world,
                town,
                economy::Request::Production(start_request())
            )
            .is_err()
        );
        assert_eq!(carry(&world, body), before);
    }
}

#[test]
fn paused_debt_is_indexed_without_a_route_and_cancel_releases_only_that_job() {
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
    execute(
        &mut world,
        town,
        "owner",
        &mut policy,
        Request::Pause {
            job: "supply".into(),
        },
    )
    .unwrap();
    publish_policy(&mut world, town, policy);
    assert_eq!(economy::target(&world, Some(town), worker), None);
    assert!(economy::production_reserved(&world, Some(town), worker));
    let other = resident(&mut world, town, [9.5, 201.0, 8.5]);
    assert!(!economy::production_reserved(
        &world,
        Some(town),
        uuid_of(&world, other)
    ));
    let slot = world.entities.slot_of(body).unwrap();
    let protected = economy::protected_carry(
        &world,
        slot,
        world.entities.row(slot).villager.as_ref().unwrap(),
    );
    assert_eq!(protected.iter().sum::<u32>(), 4);
    let before = carry(&world, body);
    economy::execute(
        &mut world,
        town,
        economy::Request::Production(Request::Cancel {
            job: "supply".into(),
        }),
    )
    .unwrap();
    let old = world.settlements.get(town).unwrap().economy.production.jobs["supply"].clone();
    assert_eq!(old.delivery.as_ref().unwrap().lots[0].stack.count, 1);
    assert!(!economy::production_reserved(&world, Some(town), worker));
    assert_eq!(carry(&world, body), before);
    // Fresh input in the UNIT fixture only; old debt is never reassigned.
    let mut inputs = carry(&world, body);
    inputs[1] = item(&world, "minecraft:wheat", 3);
    give(inputs, &mut world, body);
    economy::execute(
        &mut world,
        town,
        economy::Request::Production(Request::Start {
            job: "new".into(),
            worker,
            recipe: "krasnokamen-settlements:bread".into(),
            count: 1,
            station: table,
            stand_at: stand,
            building: worksite_ref(&worksite),
        }),
    )
    .unwrap();
    let mut policy = std::mem::take(&mut world.settlements.economy_for_tick(town).production);
    at(&mut world, body, [7.5, 201.0, 8.5]);
    due(&mut world, town, &mut policy, "new");
    beside(&mut world, body, chest);
    due(&mut world, town, &mut policy, "new");
    assert!(policy.jobs["new"].delivery_complete());
    assert_eq!(
        policy.jobs["supply"], old,
        "new store never pays the old cancelled debt"
    );
    assert_eq!(stock_total(&world, body, "minecraft:bread"), 4);
    assert_eq!(stored(&world, chest, "minecraft:bread"), 1);
    validate(&policy).unwrap();
}

#[test]
fn resumed_paused_input_job_keeps_indexed_worker_ownership_but_releases_food_hold() {
    let (mut world, town, body, worker, table, stand, worksite) = setup();
    give(vec![item(&world, "minecraft:wheat", 3)], &mut world, body);
    let mut policy = bread_order(&mut world, town, worker, table, stand, &worksite);
    execute(
        &mut world,
        town,
        "owner",
        &mut policy,
        Request::Pause {
            job: "supply".into(),
        },
    )
    .unwrap();
    publish_policy(&mut world, town, policy);
    let slot = world.entities.slot_of(body).unwrap();
    assert!(economy::production_reserved(&world, Some(town), worker));
    assert_eq!(
        economy::protected_carry(
            &world,
            slot,
            world.entities.row(slot).villager.as_ref().unwrap()
        ),
        [0; 8]
    );
    economy::execute(
        &mut world,
        town,
        economy::Request::Production(Request::Resume {
            job: "supply".into(),
        }),
    )
    .unwrap();
    assert_eq!(
        economy::protected_carry(
            &world,
            slot,
            world.entities.row(slot).villager.as_ref().unwrap()
        )
        .iter()
        .sum::<u32>(),
        3
    );
}
