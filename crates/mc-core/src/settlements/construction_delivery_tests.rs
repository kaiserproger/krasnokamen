use super::*;
use crate::settlements::{economy, production};

#[test]
fn production_and_builder_assignment_exclude_each_other_and_paused_debt_cannot_be_debited() {
    let (mut state, id, slot) = fixture();
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
        .block(&mc_data::Identifier::parse("minecraft:crafting_table").unwrap())
        .unwrap()
        .default
        .0;
    state.world.set_block(10, 201, 10, table);
    state.world.fold_overlay();
    let building = BuildingId((id.0 << 32) | 99);
    let kind = BuildingKind::Workshop(Profession::Farmer);
    state.settlements.get_mut(id).unwrap().buildings.insert(
        building,
        buildings::Building {
            id: building,
            settlement: id,
            kind,
            level: 1,
            bounds: ScriptWorldBounds::new([9, 200, 9], [12, 204, 12]).unwrap(),
            work_points: vec![[10, 201, 10]],
            revision: 1,
            natural_zone: false,
        },
    );
    let reference = mc_script::ScriptSettlementBuildingRef::new(
        "building".into(),
        building.0.to_string(),
        format!("krasnokamen:{}", kind.name()),
    );
    let uuid = state.entities.row(slot).uuid;
    let request = || production::Request::Start {
        job: "builder-exclusive".into(),
        worker: uuid,
        recipe: "krasnokamen-settlements:bread".into(),
        count: 1,
        station: [10, 201, 10],
        stand_at: [10, 201, 11],
        building: reference.clone(),
    };
    assert_eq!(
        state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .unwrap()
            .data
            .profession,
        Profession::Builder
    );
    assert!(economy::execute(&mut state, id, economy::Request::Production(request())).is_err());
    state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .data
        .profession = Profession::None;
    economy::execute(&mut state, id, economy::Request::Production(request())).unwrap();
    assign_builder(&mut state, id);
    assert_eq!(
        state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .unwrap()
            .data
            .profession,
        Profession::None
    );
    economy::execute(
        &mut state,
        id,
        economy::Request::Production(production::Request::Pause {
            job: "builder-exclusive".into(),
        }),
    )
    .unwrap();
    assign_builder(&mut state, id);
    assert_eq!(
        state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .unwrap()
            .data
            .profession,
        Profession::None
    );
    let wheat = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:wheat").unwrap())
        .unwrap();
    state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .inventory[0] = Some(mc_data::ItemStack::new(wheat, 3));
    state.entities.set_pos(slot, [10.5, 201.0, 11.5]);
    economy::execute(
        &mut state,
        id,
        economy::Request::Production(production::Request::Resume {
            job: "builder-exclusive".into(),
        }),
    )
    .unwrap();
    state.tick =
        state.settlements.get(id).unwrap().economy.production.jobs["builder-exclusive"].next_due;
    economy::tick(&mut state);
    economy::execute(
        &mut state,
        id,
        economy::Request::Production(production::Request::Pause {
            job: "builder-exclusive".into(),
        }),
    )
    .unwrap();
    assert!(
        state.settlements.get(id).unwrap().economy.production.jobs["builder-exclusive"]
            .delivery_pending()
    );
    assign_builder(&mut state, id);
    assert_eq!(
        state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .unwrap()
            .data
            .profession,
        Profession::None
    );
    // Stale/foreign reassignment cannot bypass the actual behavior/transaction guards.
    state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .data
        .profession = Profession::Builder;
    let before = state
        .entities
        .row(slot)
        .villager
        .as_ref()
        .unwrap()
        .inventory
        .clone();
    assert_eq!(
        crate::plugin_structures::settlement_builds::advance(&mut state, id, "no-project", uuid),
        Err(mc_script::ScriptOperationFailure::Busy)
    );
    assert_eq!(
        crate::plugin_structures::settlement_builds::fetch(&mut state, uuid, [1, 201, 1], wheat),
        Err(mc_script::ScriptOperationFailure::Busy)
    );
    crate::plugin_structures::settlement_builds::return_carry(&mut state, id);
    let mut v = state.entities.row_mut(slot).villager.take().unwrap();
    assert!(
        crate::villager::building::work(
            &mut state,
            slot,
            &mut v,
            &mut crate::boundary::ChangeSet::default()
        )
        .is_none()
    );
    assert_eq!(v.inventory, before);
    state.entities.row_mut(slot).villager = Some(v);
    economy::execute(
        &mut state,
        id,
        economy::Request::Production(production::Request::Cancel {
            job: "builder-exclusive".into(),
        }),
    )
    .unwrap();
    assert!(!economy::production_reserved(&state, Some(id), uuid));
    assert_eq!(
        state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .unwrap()
            .inventory,
        before
    );
}
