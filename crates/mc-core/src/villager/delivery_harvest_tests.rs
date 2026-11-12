//! Actual certified harvest::work and native production/store/cancel paths.
//! Unit setup is explicit; no graphical or synthetic-receipt claim.
use super::*;
use crate::settlements::{commands::EconomyDomain, economy_commands};
use mc_data::{Identifier, ItemStack};

fn item(state: &crate::tick::TickState, name: &str) -> u32 {
    state
        .registry
        .items
        .id_of(&Identifier::parse(format!("minecraft:{name}")).unwrap())
        .unwrap()
}

fn count(v: &super::super::VillagerState, id: u32) -> i32 {
    v.inventory
        .iter()
        .flatten()
        .filter(|stack| stack.item_id == id)
        .map(|stack| stack.count)
        .sum()
}

fn produced(
    recipe: &str,
    paused: bool,
) -> (
    crate::tick::TickState,
    crate::entities::EntitySlot,
    Box<super::super::VillagerState>,
) {
    let (mut state, slot, mut v) = fixture();
    crate::plugin_startup::apply(
        std::sync::Arc::make_mut(&mut state.registry),
        &mc_plugin_host::DeploymentContribution::default(),
        None,
        &[],
    )
    .unwrap();
    let settlement = v.settlement.unwrap();
    let building = state
        .settlements
        .get(settlement)
        .unwrap()
        .buildings
        .values()
        .find(|building| building.kind == BuildingKind::Workshop(VillagerProfession::Farmer))
        .unwrap()
        .id;
    let table = crate::villager::mining_tests::block(&state, "crafting_table");
    state.world.set_block(3, 201, 3, table);
    state.world.fold_overlay();
    if recipe == "bread" {
        v.inventory[0] = Some(ItemStack::new(item(&state, "bread"), 1));
        v.inventory[1] = Some(ItemStack::new(item(&state, "wheat"), 6));
    } else {
        v.inventory[0] = Some(ItemStack::new(item(&state, "hay_block"), 1));
    }
    state.entities.set_pos(slot, [3.5, 201.0, 4.5]);
    state.entities.row_mut(slot).villager = Some(v);
    state.entities.row_mut(slot).controls.no_ai = true;
    let worker = state.entities.row(slot).uuid;
    economy_commands::execute(
        &mut state,
        settlement,
        EconomyDomain::Production,
        &[
            "start".into(),
            "farm-output".into(),
            worker.to_string(),
            format!("krasnokamen-settlements:{recipe}"),
            "1".into(),
            building.0.to_string(),
            "3,201,3".into(),
            "3,201,4".into(),
        ],
    )
    .unwrap();
    state.tick = state.tick.max(
        state
            .settlements
            .get(settlement)
            .unwrap()
            .economy
            .production
            .jobs["farm-output"]
            .next_due,
    );
    let _ = crate::tick::run_tick(&mut state, &[]);
    assert_eq!(
        state
            .settlements
            .get(settlement)
            .unwrap()
            .economy
            .production
            .jobs["farm-output"]
            .done,
        1
    );
    if paused {
        economy_commands::execute(
            &mut state,
            settlement,
            EconomyDomain::Production,
            &["pause".into(), "farm-output".into()],
        )
        .unwrap();
    }
    let mut v = state.entities.row_mut(slot).villager.take().unwrap();
    v.sleeping = false;
    v.activity = Activity::Work;
    v.harvest_at = 0;
    v.harvest_progress = Default::default();
    (state, slot, v)
}

/// This farm fixture already certified building sequence 2 after clearing the
/// mining fixture's sequence-1 building. mining_tests::storage hard-codes 2;
/// using it here would silently replace the farmer's workplace with storage.
fn storage_preserving_farm(
    state: &mut crate::tick::TickState,
    v: &super::super::VillagerState,
    position: [i32; 3],
) {
    let farm = buildings::building_for_job(state, v)
        .expect("certified farm before storage")
        .clone();
    let farm_image = serde_json::to_value(&farm).unwrap();
    assert_eq!(
        farm.kind,
        BuildingKind::Workshop(VillagerProfession::Farmer)
    );
    let settlement = v.settlement.unwrap();
    let sequence = state
        .settlements
        .get(settlement)
        .unwrap()
        .building_sequence
        .checked_add(1)
        .unwrap();
    assert!(sequence <= u64::from(u32::MAX));
    let id = buildings::BuildingId((settlement.0 << 32) | sequence);
    assert_ne!(id, farm.id);
    assert!(
        !state
            .settlements
            .get(settlement)
            .unwrap()
            .buildings
            .contains_key(&id)
    );
    let chest = crate::villager::mining_tests::block(state, "chest");
    state
        .world
        .set_block(position[0], position[1], position[2], chest);
    state.block_entities.insert(
        (position[0], position[1], position[2]),
        crate::tick::BlockEntityState {
            slots: vec![ItemStack::EMPTY; 27],
            furnace: None,
        },
    );
    let record = state.settlements.get_mut(settlement).unwrap();
    record.building_sequence = sequence;
    assert!(
        record
            .buildings
            .insert(
                id,
                buildings::Building {
                    id,
                    settlement,
                    kind: BuildingKind::Storage,
                    level: 1,
                    bounds: mc_script::ScriptWorldBounds::new(
                        position,
                        [position[0] + 2, position[1] + 2, position[2] + 2]
                    )
                    .unwrap(),
                    work_points: vec![position],
                    revision: 1,
                    natural_zone: false,
                }
            )
            .is_none()
    );
    state.world.fold_overlay();
    let still_farm =
        buildings::building_for_job(state, v).expect("storage must preserve farmer workplace");
    assert_eq!(serde_json::to_value(still_farm).unwrap(), farm_image);
}

fn work(
    state: &mut crate::tick::TickState,
    slot: crate::entities::EntitySlot,
    v: &mut super::super::VillagerState,
    changes: &mut ChangeSet,
) {
    let farm = buildings::building_for_job(state, v)
        .expect("real farm work requires its original certificate");
    assert_eq!(
        farm.kind,
        BuildingKind::Workshop(VillagerProfession::Farmer)
    );
    for _ in 0..20 {
        state.tick += 40;
        harvest::work(state, slot, v, changes);
    }
}

#[test]
fn pending_bread_survives_farmer_autobake_and_unrelated_work_resumes_after_cancel_or_delivery() {
    for paused in [false, true] {
        for cancel in [false, true] {
            let (mut state, slot, mut v) = produced("bread", paused);
            let settlement = v.settlement.unwrap();
            let bread = item(&state, "bread");
            let wheat = item(&state, "wheat");
            let carrot = item(&state, "carrot");
            assert_eq!((count(&v, bread), count(&v, wheat)), (2, 3));
            let owed = state
                .settlements
                .get(settlement)
                .unwrap()
                .economy
                .production
                .jobs["farm-output"]
                .delivery
                .clone();
            let mature = crop(&mut state, 4, 4);
            state.world.fold_overlay();
            state.entities.set_pos(slot, [4.5, 201.0, 4.5]);
            let mut changes = ChangeSet::default();
            work(&mut state, slot, &mut v, &mut changes);
            assert_ne!(
                state.world.block_at(4, 201, 4),
                Some(mature),
                "unrelated carrot harvest still commits"
            );
            assert!(count(&v, carrot) >= 3);
            assert_eq!(
                (count(&v, bread), count(&v, wheat)),
                (2, 3),
                "defer bread conversion before adding bread or debiting wheat"
            );
            assert_eq!(
                state
                    .settlements
                    .get(settlement)
                    .unwrap()
                    .economy
                    .production
                    .jobs["farm-output"]
                    .delivery,
                owed
            );
            assert!(!owed.as_ref().unwrap().invalidated);
            if cancel {
                state.entities.row_mut(slot).villager = Some(v);
                economy_commands::execute(
                    &mut state,
                    settlement,
                    EconomyDomain::Production,
                    &["cancel".into(), "farm-output".into()],
                )
                .unwrap();
                v = state.entities.row_mut(slot).villager.take().unwrap();
            } else {
                storage_preserving_farm(&mut state, &v, [6, 201, 4]);
                state.entities.set_pos(slot, [6.5, 201.0, 5.5]);
                state.entities.row_mut(slot).villager = Some(v);
                if paused {
                    economy_commands::execute(
                        &mut state,
                        settlement,
                        EconomyDomain::Production,
                        &["resume".into(), "farm-output".into()],
                    )
                    .unwrap();
                }
                state.tick = state.tick.max(
                    state
                        .settlements
                        .get(settlement)
                        .unwrap()
                        .economy
                        .production
                        .jobs["farm-output"]
                        .next_due,
                );
                let _ = crate::tick::run_tick(&mut state, &[]);
                assert!(
                    state
                        .settlements
                        .get(settlement)
                        .unwrap()
                        .economy
                        .production
                        .jobs["farm-output"]
                        .delivery_complete()
                );
                let stored: i32 = state.block_entities[&(6, 201, 4)]
                    .slots
                    .iter()
                    .filter(|stack| stack.item_id == bread)
                    .map(|stack| stack.count)
                    .sum();
                assert_eq!(stored, 1);
                v = state.entities.row_mut(slot).villager.take().unwrap();
                v.sleeping = false;
                v.activity = Activity::Work;
            }
            let before = count(&v, bread);
            crop(&mut state, 5, 4);
            state.world.fold_overlay();
            state.entities.set_pos(slot, [5.5, 201.0, 4.5]);
            v.harvest_at = 0;
            v.harvest_progress = Default::default();
            work(&mut state, slot, &mut v, &mut ChangeSet::default());
            assert_eq!(
                count(&v, bread),
                before + 1,
                "ordinary bake resumes once output reservation is released"
            );
            assert_eq!(count(&v, wheat), 0);
            if cancel {
                assert_eq!(
                    state
                        .settlements
                        .get(settlement)
                        .unwrap()
                        .economy
                        .production
                        .jobs["farm-output"]
                        .delivery,
                    owed,
                    "cancel does not pay or increase the old debt"
                );
            }
        }
    }
}

#[test]
fn pending_wheat_defers_crop_before_mutation_but_farmer_selects_unrelated_crop() {
    for paused in [false, true] {
        for cancel in [false, true] {
            let (mut state, slot, mut v) = produced("open_hay", paused);
            let settlement = v.settlement.unwrap();
            let wheat = item(&state, "wheat");
            let seeds = item(&state, "wheat_seeds");
            let carrot = item(&state, "carrot");
            let owed = state
                .settlements
                .get(settlement)
                .unwrap()
                .economy
                .production
                .jobs["farm-output"]
                .delivery
                .clone();
            let mature = crate::settlements::allocator::tests::block(
                &state,
                "minecraft:wheat",
                &[("age", "7")],
            );
            let farmland = crate::villager::mining_tests::block(&state, "farmland");
            state.world.set_block(4, 200, 4, farmland);
            state.world.set_block(4, 201, 4, mature);
            crop(&mut state, 5, 4);
            state.world.fold_overlay();
            state.entities.set_pos(slot, [4.5, 201.0, 4.5]);
            // Seed a stale selected matching target. Selection must release it,
            // not stare at it forever and starve otherwise permitted carrot work.
            let building = state
                .settlements
                .get(settlement)
                .unwrap()
                .buildings
                .values()
                .find(|building| {
                    building.kind == BuildingKind::Workshop(VillagerProfession::Farmer)
                })
                .unwrap();
            v.harvest_progress.building = building.id.0;
            v.harvest_progress.revision = building.revision;
            v.harvest_progress.bounds = Some(building.bounds);
            v.harvest_progress.target = Some(BlockPos { x: 4, y: 201, z: 4 });
            let before_wheat = count(&v, wheat);
            let before_seeds = count(&v, seeds);
            let mut changes = ChangeSet::default();
            work(&mut state, slot, &mut v, &mut changes);
            assert_eq!(state.world.block_at(4, 201, 4), Some(mature));
            assert!(
                !changes
                    .blocks
                    .iter()
                    .any(|change| change.position == BlockPos { x: 4, y: 201, z: 4 })
            );
            assert_eq!(count(&v, wheat), before_wheat);
            assert_eq!(count(&v, seeds), before_seeds);
            assert!(
                count(&v, carrot) >= 3,
                "matching deferral must not disable unrelated farm work"
            );
            assert_eq!(
                state
                    .settlements
                    .get(settlement)
                    .unwrap()
                    .economy
                    .production
                    .jobs["farm-output"]
                    .delivery,
                owed
            );
            if cancel {
                state.entities.row_mut(slot).villager = Some(v);
                economy_commands::execute(
                    &mut state,
                    settlement,
                    EconomyDomain::Production,
                    &["cancel".into(), "farm-output".into()],
                )
                .unwrap();
            } else {
                storage_preserving_farm(&mut state, &v, [6, 201, 4]);
                state.entities.set_pos(slot, [6.5, 201.0, 5.5]);
                state.entities.row_mut(slot).villager = Some(v);
                if paused {
                    economy_commands::execute(
                        &mut state,
                        settlement,
                        EconomyDomain::Production,
                        &["resume".into(), "farm-output".into()],
                    )
                    .unwrap();
                }
                state.tick = state.tick.max(
                    state
                        .settlements
                        .get(settlement)
                        .unwrap()
                        .economy
                        .production
                        .jobs["farm-output"]
                        .next_due,
                );
                let _ = crate::tick::run_tick(&mut state, &[]);
                assert!(
                    state
                        .settlements
                        .get(settlement)
                        .unwrap()
                        .economy
                        .production
                        .jobs["farm-output"]
                        .delivery_complete()
                );
                assert_eq!(
                    state.block_entities[&(6, 201, 4)]
                        .slots
                        .iter()
                        .filter(|stack| stack.item_id == wheat)
                        .map(|stack| stack.count)
                        .sum::<i32>(),
                    before_wheat
                );
            }
            let mut v = state.entities.row_mut(slot).villager.take().unwrap();
            v.sleeping = false;
            v.activity = Activity::Work;
            state.entities.set_pos(slot, [4.5, 201.0, 4.5]);
            v.harvest_at = 0;
            v.harvest_progress = Default::default();
            work(&mut state, slot, &mut v, &mut ChangeSet::default());
            assert_ne!(
                state.world.block_at(4, 201, 4),
                Some(mature),
                "released reservation restores ordinary matching harvest"
            );
            assert_eq!(count(&v, seeds), before_seeds + 1);
            if cancel {
                assert_eq!(
                    state
                        .settlements
                        .get(settlement)
                        .unwrap()
                        .economy
                        .production
                        .jobs["farm-output"]
                        .delivery,
                    owed
                );
            } else {
                assert!(
                    state
                        .settlements
                        .get(settlement)
                        .unwrap()
                        .economy
                        .production
                        .jobs["farm-output"]
                        .delivery_complete()
                );
            }
        }
    }
}
