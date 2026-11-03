use super::*;
use crate::settlements::{buildings, economy, production};

fn workshop(f: &mut Fixture) -> mc_script::ScriptSettlementBuildingRef {
    let table = block(&f.state, "minecraft:crafting_table", &[]);
    f.state.world.set_block(8, 201, 20, table);
    f.state.world.fold_overlay();
    let bid = buildings::BuildingId((f.settlement.0 << 32) | 99);
    let kind = buildings::BuildingKind::Workshop(crate::villager::VillagerProfession::Farmer);
    f.state
        .settlements
        .get_mut(f.settlement)
        .unwrap()
        .buildings
        .insert(
            bid,
            buildings::Building {
                id: bid,
                settlement: f.settlement,
                kind,
                level: 1,
                bounds: mc_script::ScriptWorldBounds::new([7, 200, 19], [10, 204, 23]).unwrap(),
                work_points: vec![[8, 201, 20]],
                revision: 1,
                natural_zone: false,
            },
        );
    mc_script::ScriptSettlementBuildingRef::new(
        "building".into(),
        bid.0.to_string(),
        format!("solaris:{}", kind.name()),
    )
}

fn production_request(
    f: &Fixture,
    building: mc_script::ScriptSettlementBuildingRef,
) -> production::Request {
    production::Request::Start {
        job: "caravan-exclusive".into(),
        worker: f.carrier_uuid,
        recipe: "solaris-settlements:bread".into(),
        count: 1,
        station: [8, 201, 20],
        stand_at: [8, 201, 21],
        building,
    }
}

#[test]
fn caravan_then_production_and_paused_production_then_caravan_both_refuse() {
    let mut f = fixture();
    let building = workshop(&mut f);
    stock(&mut f.state, SRC_CHEST, "minecraft:bread", 3);
    let request = open(&f, "reserved-caravan", 1, None);
    economy::execute(
        &mut f.state,
        f.settlement,
        economy::Request::Caravan(request),
    )
    .unwrap();
    let request = production_request(&f, building.clone());
    assert!(
        economy::execute(
            &mut f.state,
            f.settlement,
            economy::Request::Production(request)
        )
        .is_err()
    );
    economy::execute(
        &mut f.state,
        f.settlement,
        economy::Request::Caravan(Request::Cancel {
            id: "reserved-caravan".into(),
        }),
    )
    .unwrap();
    let request = production_request(&f, building);
    economy::execute(
        &mut f.state,
        f.settlement,
        economy::Request::Production(request),
    )
    .unwrap();
    economy::execute(
        &mut f.state,
        f.settlement,
        economy::Request::Production(production::Request::Pause {
            job: "caravan-exclusive".into(),
        }),
    )
    .unwrap();
    assert_eq!(
        economy::target(&f.state, Some(f.settlement), f.carrier_uuid),
        None
    );
    let request = open(&f, "competing-caravan", 1, None);
    assert!(
        economy::execute(
            &mut f.state,
            f.settlement,
            economy::Request::Caravan(request)
        )
        .is_err()
    );
}

#[test]
fn paused_output_blocks_direct_caravan_admission_and_actual_load_unload() {
    let mut f = fixture();
    let building = workshop(&mut f);
    stock(&mut f.state, SRC_CHEST, "minecraft:bread", 3);
    // Old/stale shipment exists outside the policy owner as adversarial setup.
    // Its actual load/unload functions must still refuse the active debt.
    let mut stale = State::default();
    let request = open(&f, "stale", 1, None);
    run(&mut f.state, f.settlement, &mut stale, request);
    let slot = f.state.entities.slot_of(f.carrier).unwrap();
    let wheat = f
        .state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:wheat").unwrap())
        .unwrap();
    f.state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .inventory[0] = Some(ItemStack::new(wheat, 3));
    f.state.entities.set_pos(slot, [8.5, 201.0, 21.5]);
    let request = production_request(&f, building);
    economy::execute(
        &mut f.state,
        f.settlement,
        economy::Request::Production(request),
    )
    .unwrap();
    f.state.tick = f
        .state
        .settlements
        .get(f.settlement)
        .unwrap()
        .economy
        .production
        .jobs["caravan-exclusive"]
        .next_due;
    economy::tick(&mut f.state);
    economy::execute(
        &mut f.state,
        f.settlement,
        economy::Request::Production(production::Request::Pause {
            job: "caravan-exclusive".into(),
        }),
    )
    .unwrap();
    assert!(
        f.state
            .settlements
            .get(f.settlement)
            .unwrap()
            .economy
            .production
            .jobs["caravan-exclusive"]
            .delivery_pending()
    );
    let request = open(&f, "direct", 1, None);
    assert_eq!(
        execute(
            &mut f.state,
            f.settlement,
            OWNER,
            &mut State::default(),
            request
        ),
        Err(Failure::Busy)
    );
    let shipment = stale.shipments.get_mut("stale").unwrap();
    let carry_before = load(&f.state, &carry_endpoint(f.carrier_uuid)).unwrap();
    let source_before = load(&f.state, &container_endpoint(&f.source)).unwrap();
    assert_eq!(
        try_pickup(&mut f.state, f.settlement, shipment),
        Err(Failure::Busy)
    );
    assert_eq!(
        try_unload(&mut f.state, f.settlement, shipment),
        Err(Failure::Busy)
    );
    let carry_after = load(&f.state, &carry_endpoint(f.carrier_uuid)).unwrap();
    let source_after = load(&f.state, &container_endpoint(&f.source)).unwrap();
    assert_eq!(
        (carry_after.slots, carry_after.fence),
        (carry_before.slots, carry_before.fence)
    );
    assert_eq!(
        (source_after.slots, source_after.fence),
        (source_before.slots, source_before.fence)
    );
}

#[test]
fn builder_and_soldier_cannot_be_admitted_as_native_caravan_carriers() {
    for profession in [
        crate::villager::VillagerProfession::Builder,
        crate::villager::VillagerProfession::Soldier,
    ] {
        let mut f = fixture();
        stock(&mut f.state, SRC_CHEST, "minecraft:bread", 3);
        let slot = f.state.entities.slot_of(f.carrier).unwrap();
        f.state
            .entities
            .row_mut(slot)
            .villager
            .as_mut()
            .unwrap()
            .data
            .profession = profession;
        let request = open(&f, "role-conflict", 1, None);
        assert_eq!(
            execute(
                &mut f.state,
                f.settlement,
                OWNER,
                &mut State::default(),
                request
            ),
            Err(Failure::Busy)
        );
    }
}
