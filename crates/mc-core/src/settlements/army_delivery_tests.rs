use super::*;
use crate::settlements::{economy, production};

fn production_building(
    state: &mut TickState,
    id: SettlementId,
) -> mc_script::ScriptSettlementBuildingRef {
    crate::plugin_startup::apply(
        std::sync::Arc::make_mut(&mut state.registry),
        &mc_plugin_host::DeploymentContribution::default(),
        None,
        &[],
    )
    .unwrap();
    let table = state
        .registry
        .blocks
        .block(&Identifier::parse("minecraft:crafting_table").unwrap())
        .unwrap()
        .default
        .0;
    state.world.set_block(3, 201, 3, table);
    state.world.fold_overlay();
    let bid = BuildingId((id.0 << 32) | 3);
    let kind = BuildingKind::Workshop(VillagerProfession::Farmer);
    state.settlements.get_mut(id).unwrap().buildings.insert(
        bid,
        buildings::Building {
            id: bid,
            settlement: id,
            kind,
            level: 1,
            bounds: mc_script::ScriptWorldBounds::new([1, 200, 1], [5, 204, 5]).unwrap(),
            work_points: vec![[3, 201, 3]],
            revision: 1,
            natural_zone: false,
        },
    );
    mc_script::ScriptSettlementBuildingRef::new(
        "building".into(),
        bid.0.to_string(),
        format!("krasnokamen:{}", kind.name()),
    )
}

fn start(
    state: &mut TickState,
    id: SettlementId,
    uuid: Uuid,
    building: mc_script::ScriptSettlementBuildingRef,
) -> Result<(), mc_script::ScriptOperationFailure> {
    economy::execute(
        state,
        id,
        economy::Request::Production(production::Request::Start {
            job: "army-exclusive".into(),
            worker: uuid,
            recipe: "krasnokamen-settlements:bread".into(),
            count: 1,
            station: [3, 201, 3],
            stand_at: [3, 201, 4],
            building,
        }),
    )
}

#[test]
fn production_then_recruit_and_soldier_then_production_are_symmetric_refusals() {
    let (mut state, id, slot) = fixture();
    let building = production_building(&mut state, id);
    let uuid = state.entities.row(slot).uuid;
    start(&mut state, id, uuid, building.clone()).unwrap();
    assert!(recruit(&mut state, id).is_err());
    economy::execute(
        &mut state,
        id,
        economy::Request::Production(production::Request::Pause {
            job: "army-exclusive".into(),
        }),
    )
    .unwrap();
    assert!(
        recruit(&mut state, id).is_err(),
        "pause has no route but still owns the worker"
    );
    economy::execute(
        &mut state,
        id,
        economy::Request::Production(production::Request::Cancel {
            job: "army-exclusive".into(),
        }),
    )
    .unwrap();
    recruit(&mut state, id).unwrap();
    assert!(start(&mut state, id, uuid, building).is_err());
    assert_eq!(
        state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .unwrap()
            .data
            .profession,
        VillagerProfession::Soldier
    );
}

#[test]
fn paused_output_cannot_be_recruited_or_redirected_by_a_stale_soldier_role() {
    let (mut state, id, slot) = fixture();
    let building = production_building(&mut state, id);
    let uuid = state.entities.row(slot).uuid;
    let wheat = item(&state, "wheat");
    state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .inventory[0] = Some(ItemStack::new(wheat, 3));
    state.entities.set_pos(slot, [3.5, 201.0, 4.5]);
    start(&mut state, id, uuid, building).unwrap();
    state.tick =
        state.settlements.get(id).unwrap().economy.production.jobs["army-exclusive"].next_due;
    economy::tick(&mut state);
    economy::execute(
        &mut state,
        id,
        economy::Request::Production(production::Request::Pause {
            job: "army-exclusive".into(),
        }),
    )
    .unwrap();
    assert!(
        state.settlements.get(id).unwrap().economy.production.jobs["army-exclusive"]
            .delivery_pending()
    );
    assert!(recruit(&mut state, id).is_err());
    // Adversarial stale role: actual soldier order/goal paths must not steal
    // the resident even when a conflicting role survived an old world image.
    let v = state.entities.row_mut(slot).villager.as_mut().unwrap();
    v.data.profession = VillagerProfession::Soldier;
    v.soldier = Some(Soldier {
        barracks: BuildingId((id.0 << 32) | 1),
        order: Order::Patrol,
        next_attack: 0,
    });
    let before = v.inventory.clone();
    assert!(order(&mut state, id, Some(uuid), Order::Return).is_err());
    assert!(dismiss(&mut state, id, uuid).is_err());
    let mut v = state.entities.row_mut(slot).villager.take().unwrap();
    assert!(crate::villager::soldier::work(&mut state, slot, &mut v).is_none());
    assert_eq!(v.inventory, before);
    state.entities.row_mut(slot).villager = Some(v);
    assert!(
        !state.settlements.get(id).unwrap().economy.production.jobs["army-exclusive"]
            .delivery_complete()
    );
}
