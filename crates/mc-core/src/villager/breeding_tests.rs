//! A typical player village, driven only through the allocator + brain tick:
//! nobody is hand-placed into courtship and nobody is handed food. Food comes
//! from what the farmers harvest, exactly as in vanilla.
use super::*;

fn population(state: &TickState) -> usize {
    state
        .entities
        .slots()
        .filter(|&slot| state.entities.row(slot).villager.is_some())
        .count()
}

/// Four bound villagers (two farmers, two unemployed), six beds, a certified
/// level-2 farmer workshop whose certificate contains the mature wheat/carrot
/// field beside its two composters.
fn typical_village(state: &mut TickState) {
    state.day_time = 3000;
    let settlement = state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "village".into(),
            "owner".into(),
            [8, 201, 8],
            crate::settlements::commands::DEFAULT_RADIUS,
        )
        .unwrap();
    for (x, z) in [(3, 2), (7, 2), (3, 13), (7, 13), (11, 13), (11, 2)] {
        roofed_bed(state, x, z);
    }
    let composter = block(state, "minecraft:composter", &[]);
    state.world.set_block(12, 201, 8, composter);
    state.world.set_block(12, 201, 9, composter);
    let farmland = block(state, "minecraft:farmland", &[]);
    let wheat = block(state, "minecraft:wheat", &[("age", "7")]);
    let carrots = block(state, "minecraft:carrots", &[("age", "7")]);
    for x in 13..16 {
        for z in 5..12 {
            state.world.set_block(x, 200, z, farmland);
            state
                .world
                .set_block(x, 201, z, if x == 13 { carrots } else { wheat });
        }
    }
    state.world.fold_overlay();
    let workshop = crate::settlements::buildings::certify(
        state,
        settlement,
        crate::settlements::buildings::BuildingKind::Workshop(Profession::Farmer),
        mc_script::ScriptWorldBounds::new([10, 200, 5], [15, 203, 11]).unwrap(),
    )
    .expect("the farm has a loaded, floored workshop with composters");
    crate::settlements::buildings::upgrade(state, workshop)
        .expect("level 2 supplies a distinct slot for each farmer");
    assert_eq!(
        state.settlements.get(settlement).unwrap().buildings[&workshop]
            .work_points
            .len(),
        2,
    );
    for (index, offset) in [(0, 0.0), (1, 1.0), (2, 2.0), (3, 3.0)] {
        let slot = villager(state, [8.5 + offset, 201.0, 8.5]);
        let id = state.entities.body(slot).id;
        crate::settlements::bind(state, settlement, id).unwrap();
        if index < 2 {
            state
                .entities
                .row_mut(slot)
                .villager
                .as_mut()
                .unwrap()
                .data
                .profession = Profession::Farmer;
        }
    }
}

fn stored_food_village(beds: usize) -> (TickState, [EntitySlot; 2], u32) {
    stored_food_village_roofed(beds, true)
}

fn stored_food_village_roofed(beds: usize, roofed: bool) -> (TickState, [EntitySlot; 2], u32) {
    let mut state = test_state();
    state.day_time = 3000;
    let settlement = state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "village".into(),
            "owner".into(),
            [8, 201, 8],
            crate::settlements::commands::DEFAULT_RADIUS,
        )
        .unwrap();
    // Residents claim only roofed beds (they shelter instead of sleeping in
    // the open), so each bed gets a roof block clear of breeding headroom.
    let stone = state.registry.states.stone;
    for (x, z) in [(3, 3), (7, 3), (11, 3)].into_iter().take(beds) {
        bed(&mut state, x, z);
        if roofed {
            state.world.set_block(x, 205, z, stone);
        }
    }
    let chest = block(&state, "minecraft:chest", &[]);
    state.world.set_block(8, 201, 8, chest);
    let bread = item(&state, "minecraft:bread");
    let mut slots = vec![ItemStack::EMPTY; 27];
    slots[0] = ItemStack::new(bread, 6);
    state.block_entities.insert(
        (8, 201, 8),
        crate::tick::BlockEntityState {
            slots,
            furnace: None,
        },
    );
    state
        .settlements
        .get_mut(settlement)
        .unwrap()
        .bootstrap_storage = Some([8, 201, 8]);
    state.world.fold_overlay();
    let parents = [[6.5, 201.0, 8.5], [10.5, 201.0, 8.5]].map(|pos| {
        let slot = villager(&mut state, pos);
        let id = state.entities.body(slot).id;
        crate::settlements::bind(&mut state, settlement, id).unwrap();
        slot
    });
    (state, parents, bread)
}

/// Settlement policy: a resident's home is a roofed bed (it shelters instead of
/// sleeping in the open), and rations follow the home. A settlement whose only
/// beds stand in the open gives its residents no home, so no stored rations
/// and no births, while the same village with roofed beds breeds.
#[test]
fn open_air_only_settlement_gives_residents_no_home_rations_or_births() {
    let (mut state, parents, bread) = stored_food_village_roofed(3, false);
    for _ in 0..6_000 {
        step(&mut state);
    }
    assert_eq!(population(&state), 2, "no birth without a roofed bed");
    for slot in parents {
        assert_eq!(
            state.entities.row(slot).villager.as_ref().unwrap().home,
            None
        );
        assert_eq!(food_count(&state, slot, bread), 0, "no ration drawn");
    }
    assert_eq!(state.block_entities[&(8, 201, 8)].slots[0].count, 6);
}

#[test]
fn bound_workers_breed_from_real_bootstrap_chest_food() {
    let (mut state, parents, bread) = stored_food_village(3);
    for _ in 0..6_000 {
        step(&mut state);
        if population(&state) > 2 {
            assert_eq!(state.block_entities[&(8, 201, 8)].slots[0].count, 0);
            for slot in parents {
                let parent = state.entities.row(slot).villager.as_ref().unwrap();
                assert_eq!(food_count(&state, slot, bread), 0);
                assert_eq!(parent.food_level, 0);
            }
            return;
        }
    }
    panic!(
        "no birth with three beds and six stored loaves; chest={:?}, parents={:?}",
        state.block_entities[&(8, 201, 8)].slots[0],
        parents.map(|slot| state.entities.row(slot).villager.as_ref().unwrap())
    );
}

/// The seed-24 builder could see the new house's bed near its site, but not
/// from the founding chest. Rechecking the local bed scan on arrival turned
/// the ration trip back into work before any bread could enter carry.
#[test]
fn ration_trip_completes_when_storage_is_beyond_the_spare_bed_scan() {
    let mut state = test_state();
    let chunks = (0..5)
        .map(|x| (crate::boundary::ChunkPos { x, z: 0 }, 0))
        .collect();
    state.world.ensure_blocking(&chunks);
    for x in 0..5 {
        state.tickets.insert(
            format!("ration-trip-{x}"),
            (crate::boundary::ChunkPos { x, z: 0 }, 0),
        );
    }
    state.refresh_active_chunks(0);
    for x in 0..80 {
        for z in 0..16 {
            state
                .world
                .set_block(x, 200, z, state.registry.states.stone);
            for y in 201..206 {
                state.world.set_block(x, y, z, state.registry.states.air);
            }
        }
    }
    let home = bed(&mut state, 44, 3);
    let spare = bed(&mut state, 70, 3);
    // Residents claim only roofed beds; roof both clear of breeding headroom.
    for head in [home, spare] {
        state
            .world
            .set_block(head.x, 205, head.z, state.registry.states.stone);
    }
    let settlement = state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "ration-trip".into(),
            "owner".into(),
            [40, 201, 8],
            96,
        )
        .unwrap();
    let chest = block(&state, "minecraft:chest", &[]);
    let bread = item(&state, "minecraft:bread");
    let logs = item(&state, "minecraft:birch_log");
    state.world.set_block(8, 201, 8, chest);
    let mut slots = vec![ItemStack::EMPTY; 27];
    slots[0] = ItemStack::new(bread, 6);
    state.block_entities.insert(
        (8, 201, 8),
        crate::tick::BlockEntityState {
            slots,
            furnace: None,
        },
    );
    state
        .settlements
        .get_mut(settlement)
        .unwrap()
        .bootstrap_storage = Some([8, 201, 8]);
    let slot = villager(&mut state, [40.5, 201.0, 8.5]);
    let entity = state.entities.body(slot).id;
    crate::settlements::bind(&mut state, settlement, entity).unwrap();
    let resident = state.entities.row_mut(slot).villager.as_mut().unwrap();
    resident.home = Some(home);
    resident.inventory[0] = Some(ItemStack::new(logs, 32));
    state.day_time = 3000;
    state.world.fold_overlay();
    for _ in 0..1_200 {
        step(&mut state);
        if food_count(&state, slot, bread) == 3 {
            assert!(distance(state.entities.pos(slot), centre(spare)) > 48.0 * 48.0);
            assert_eq!(state.block_entities[&(8, 201, 8)].slots[0].count, 3);
            assert_eq!(food_count(&state, slot, logs), 32);
            assert!(
                state
                    .entities
                    .row(slot)
                    .villager
                    .as_ref()
                    .unwrap()
                    .ration_target
                    .is_none()
            );
            return;
        }
    }
    panic!(
        "admitted ration trip never withdrew real bread: pos={:?}, resident={:?}",
        state.entities.pos(slot),
        state.entities.row(slot).villager
    );
}

#[test]
fn bound_village_uses_reachable_food_when_nearest_chest_is_blocked() {
    let (mut state, parents, bread) = stored_food_village(3);
    let settlement = state
        .entities
        .row(parents[0])
        .villager
        .as_ref()
        .unwrap()
        .settlement
        .unwrap();
    let chest = block(&state, "minecraft:chest", &[]);
    let blocked = state.block_entities.remove(&(8, 201, 8)).unwrap();
    state.world.set_block(8, 201, 8, state.registry.states.air);
    state.world.set_block(5, 201, 8, chest);
    state.block_entities.insert((5, 201, 8), blocked);
    state
        .settlements
        .get_mut(settlement)
        .unwrap()
        .bootstrap_storage = Some([5, 201, 8]);
    let mut slots = vec![ItemStack::EMPTY; 27];
    slots[0] = ItemStack::new(bread, 6);
    state.world.set_block(13, 201, 8, chest);
    state.block_entities.insert(
        (13, 201, 8),
        crate::tick::BlockEntityState {
            slots,
            furnace: None,
        },
    );
    for x in 3..=7 {
        for z in 6..=10 {
            if x == 3 || x == 7 || z == 6 || z == 10 {
                for y in 201..=204 {
                    state.world.set_block(x, y, z, state.registry.states.stone);
                }
            }
        }
    }
    state.world.fold_overlay();
    crate::settlements::buildings::certify(
        &mut state,
        settlement,
        crate::settlements::buildings::BuildingKind::Storage,
        mc_script::ScriptWorldBounds::new([12, 200, 7], [14, 203, 9]).unwrap(),
    )
    .unwrap();
    for (slot, z) in parents.into_iter().zip([8.5, 9.5]) {
        state.entities.set_pos(slot, [1.5, 201.0, z]);
    }
    for _ in 0..6_000 {
        step(&mut state);
        if population(&state) > 2 {
            assert_eq!(state.block_entities[&(5, 201, 8)].slots[0].count, 6);
            assert_eq!(state.block_entities[&(13, 201, 8)].slots[0].count, 0);
            return;
        }
    }
    panic!(
        "no birth with inaccessible nearest chest and reachable stocked storage: parents={:?}, chests={:?} {:?}",
        parents.map(|slot| state.entities.row(slot).villager.as_ref().unwrap()),
        state.block_entities[&(5, 201, 8)].slots[0],
        state.block_entities[&(13, 201, 8)].slots[0]
    );
}

#[test]
fn stored_breeding_food_respects_spare_beds_and_stock_reservations() {
    use mc_script::{
        ScriptBlockPosition, ScriptInventoryEndpoint, ScriptInventoryMaterial,
        ScriptInventoryResourcePlan, ScriptInventoryWorkPortion, ScriptOperation,
        ScriptOperationRequest, ScriptOwnedInventoryOperation,
    };

    let (mut no_spare, parents, bread) = stored_food_village(2);
    let settlement = no_spare
        .entities
        .row(parents[0])
        .villager
        .as_ref()
        .unwrap()
        .settlement
        .unwrap();
    no_spare
        .settlements
        .get_mut(settlement)
        .unwrap()
        .bootstrap_storage = None;
    for _ in 0..200 {
        step(&mut no_spare);
    }
    assert!(parents.into_iter().all(|slot| {
        no_spare
            .entities
            .row(slot)
            .villager
            .as_ref()
            .unwrap()
            .home
            .is_some()
    }));
    no_spare
        .settlements
        .get_mut(settlement)
        .unwrap()
        .bootstrap_storage = Some([8, 201, 8]);
    for _ in 0..1_000 {
        step(&mut no_spare);
    }
    assert_eq!(no_spare.block_entities[&(8, 201, 8)].slots[0].count, 6);
    assert!(
        parents
            .into_iter()
            .all(|slot| food_count(&no_spare, slot, bread) == 0)
    );

    let (mut state, parents, bread) = stored_food_village(3);
    let endpoint = ScriptInventoryEndpoint::Container {
        dimension: state.dimension.name().into(),
        position: ScriptBlockPosition::new(8, 201, 8),
    };
    let fence = crate::plugin_inventories::load(&state, "breeding-test", &endpoint)
        .unwrap()
        .fence;
    let request = ScriptOperationRequest::try_new(
        "breeding-reservation",
        ScriptOperation::Inventory {
            operation: ScriptOwnedInventoryOperation::Reserve {
                operation_id: "breeding-reservation".into(),
                endpoint,
                resource_plan: ScriptInventoryResourcePlan::new(vec![
                    ScriptInventoryWorkPortion::new(
                        1,
                        vec![ScriptInventoryMaterial::new("minecraft:bread".into(), 3)],
                    ),
                ]),
                expected_revision: fence,
            },
        },
    )
    .unwrap();
    assert_eq!(
        crate::plugin_inventories::execute(&mut state, "breeding-test", &request).failure(),
        None
    );
    for _ in 0..1_000 {
        step(&mut state);
    }
    assert_eq!(population(&state), 2);
    assert_eq!(state.block_entities[&(8, 201, 8)].slots[0].count, 3);
    assert_eq!(
        parents
            .into_iter()
            .map(|slot| food_count(&state, slot, bread))
            .sum::<i32>(),
        3
    );
}

/// Seed-24 civilization-seed-qa (ISSUE-1-17 births): the founding bread ran
/// out on courtships that found no bed, because a courtship spends its food
/// even when no baby follows, so the town had no food left for the birth its
/// new house allowed. Two fed residents whose beds are both taken keep their
/// bread instead of courting.
#[test]
fn fed_residents_with_no_spare_bed_keep_their_bread() {
    let (mut state, parents, bread) = stored_food_village(2);
    let settlement = state
        .entities
        .row(parents[0])
        .villager
        .as_ref()
        .unwrap()
        .settlement
        .unwrap();
    state
        .settlements
        .get_mut(settlement)
        .unwrap()
        .bootstrap_storage = None;
    for _ in 0..200 {
        step(&mut state);
    }
    for slot in parents {
        let parent = state.entities.row_mut(slot).villager.as_mut().unwrap();
        assert!(
            parent.home.is_some(),
            "each parent holds one of the two beds"
        );
        parent.inventory[0] = Some(ItemStack::new(bread, 3));
    }
    for _ in 0..3_000 {
        step(&mut state);
    }
    assert_eq!(population(&state), 2);
    for slot in parents {
        assert_eq!(
            food_count(&state, slot, bread),
            3,
            "a courtship without a bed must not spend the bread"
        );
    }
}
#[test]
fn bound_miner_keeps_chest_ration_through_deposit_and_breeds() {
    let (mut state, parents, bread) = stored_food_village(3);
    let settlement = state
        .entities
        .row(parents[0])
        .villager
        .as_ref()
        .unwrap()
        .settlement
        .unwrap();
    for x in 0..16 {
        for z in 0..16 {
            for y in 180..=200 {
                state.world.set_block(x, y, z, state.registry.states.stone);
            }
        }
    }
    state.world.fold_overlay();
    crate::settlements::buildings::certify_zone(
        &mut state,
        settlement,
        crate::settlements::buildings::BuildingKind::Mine,
        mc_script::ScriptWorldBounds::new([1, 198, 10], [5, 202, 14]).unwrap(),
        [3, 201, 12],
    )
    .unwrap();
    state
        .entities
        .row_mut(parents[0])
        .villager
        .as_mut()
        .unwrap()
        .data
        .profession = Profession::Miner;
    state
        .entities
        .row_mut(parents[1])
        .villager
        .as_mut()
        .unwrap()
        .data
        .profession = Profession::Nitwit;
    state
        .settlements
        .get_mut(settlement)
        .unwrap()
        .bootstrap_storage = None;
    for _ in 0..1_000 {
        step(&mut state);
        if state
            .entities
            .row(parents[0])
            .villager
            .as_ref()
            .unwrap()
            .job_site
            .is_some()
        {
            break;
        }
    }
    assert!(
        state
            .entities
            .row(parents[0])
            .villager
            .as_ref()
            .unwrap()
            .job_site
            .is_some(),
        "miner has allocated mine"
    );
    state.day_time = 11_000;
    state
        .settlements
        .get_mut(settlement)
        .unwrap()
        .bootstrap_storage = Some([8, 201, 8]);
    let mut assigned = false;
    for _ in 0..6_000 {
        step(&mut state);
        assigned |= state
            .entities
            .row(parents[0])
            .villager
            .as_ref()
            .unwrap()
            .job_site
            .is_some();
        if population(&state) > 2 {
            assert!(assigned, "miner must run with an allocated mine");
            assert_eq!(state.block_entities[&(8, 201, 8)].slots[0].count, 0);
            assert_eq!(food_count(&state, parents[0], bread), 0);
            state.day_time = 3_000;
            for _ in 0..6_000 {
                if !step(&mut state).blocks.is_empty() {
                    return;
                }
            }
            panic!("miner never resumed extraction after its ration and birth");
        }
    }
    panic!(
        "miner did not breed; assigned={assigned}, miner={:?}, chest={:?}",
        state.entities.row(parents[0]).villager.as_ref().unwrap(),
        state.block_entities[&(8, 201, 8)].slots[0]
    );
}

#[test]
fn bound_village_farmers_feed_the_village_and_it_has_births() {
    let mut state = test_state();
    typical_village(&mut state);
    let before = population(&state);
    let mut reduced_cadence = false;
    for _ in 0..6_000 {
        step(&mut state);
        reduced_cadence |= state
            .villager_pois
            .resident_data
            .iter()
            .any(|brain| brain.cadence.period > 1);
        if population(&state) > before {
            assert!(
                reduced_cadence,
                "no player is near: the village runs on a reduced brain cadence"
            );
            return;
        }
    }
    let residents: Vec<_> = state
        .entities
        .slots()
        .filter_map(|slot| {
            let v = state.entities.row(slot).villager.as_ref()?;
            Some((
                state.entities.body(slot).id,
                state.entities.pos(slot),
                v.data.profession,
                v.job_site,
                v.home,
                v.activity,
                &v.inventory,
                (v.food_level, v.age, v.partner, v.courtship_until),
            ))
        })
        .collect();
    panic!(
        "no birth in 6000 working-hour ticks of a four-villager village with six beds: {residents:?}"
    );
}

#[test]
fn working_farmer_harvests_replants_and_bakes_bread() {
    let mut state = test_state();
    state.day_time = 3000;
    let farmland = block(&state, "minecraft:farmland", &[]);
    let wheat = block(&state, "minecraft:wheat", &[("age", "7")]);
    let young = block(&state, "minecraft:wheat", &[("age", "0")]);
    for x in 9..12 {
        state.world.set_block(x, 200, 8, farmland);
        state.world.set_block(x, 201, 8, wheat);
    }
    state.world.fold_overlay();
    let slot = villager(&mut state, [8.5, 201.0, 8.5]);
    let v = state.entities.row_mut(slot).villager.as_mut().unwrap();
    v.data.profession = Profession::Farmer;
    v.activity = Activity::Work;
    let mut changes = ChangeSet::default();
    for tick in [7, 47, 87] {
        state.tick = tick;
        let mut v = state.entities.row_mut(slot).villager.take().unwrap();
        crate::villager::harvest::work(&mut state, slot, &mut v, &mut changes);
        state.entities.row_mut(slot).villager = Some(v);
    }
    let bread = item(&state, "minecraft:bread");
    assert_eq!(food_count(&state, slot, bread), 1);
    assert_eq!(food_count(&state, slot, item(&state, "minecraft:wheat")), 0);
    assert_eq!(changes.blocks.len(), 3);
    for x in 9..12 {
        assert_eq!(state.world.block_at(x, 201, 8), Some(young));
    }
}

#[test]
fn farmer_with_partial_room_leaves_the_crop_standing() {
    let mut state = test_state();
    state.day_time = 3000;
    let farmland = block(&state, "minecraft:farmland", &[]);
    let carrots = block(&state, "minecraft:carrots", &[("age", "7")]);
    state.world.set_block(9, 200, 8, farmland);
    state.world.set_block(9, 201, 8, carrots);
    state.world.fold_overlay();
    let slot = villager(&mut state, [8.5, 201.0, 8.5]);
    let carrot = item(&state, "minecraft:carrot");
    let potato = item(&state, "minecraft:potato");
    let v = state.entities.row_mut(slot).villager.as_mut().unwrap();
    v.data.profession = Profession::Farmer;
    v.activity = Activity::Work;
    v.inventory = std::array::from_fn(|_| Some(ItemStack::new(potato, 64)));
    v.inventory[0] = Some(ItemStack::new(carrot, 62));
    let mut changes = ChangeSet::default();
    let mut v = state.entities.row_mut(slot).villager.take().unwrap();
    crate::villager::harvest::work(&mut state, slot, &mut v, &mut changes);
    state.entities.row_mut(slot).villager = Some(v);
    assert_eq!(food_count(&state, slot, carrot), 62);
    assert_eq!(state.world.block_at(9, 201, 8), Some(carrots));
    assert!(changes.blocks.is_empty());
}

#[test]
fn bound_farmer_waits_for_a_certified_job_before_harvesting() {
    let mut state = test_state();
    state.day_time = 3000;
    let settlement = state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "farm".into(),
            "owner".into(),
            [8, 201, 8],
            48,
        )
        .unwrap();
    let farmland = block(&state, "minecraft:farmland", &[]);
    let carrots = block(&state, "minecraft:carrots", &[("age", "7")]);
    state.world.set_block(9, 200, 8, farmland);
    state.world.set_block(9, 201, 8, carrots);
    state.world.fold_overlay();
    let slot = villager(&mut state, [8.5, 201.0, 8.5]);
    let entity = state.entities.body(slot).id;
    crate::settlements::bind(&mut state, settlement, entity).unwrap();
    let mut v = state.entities.row_mut(slot).villager.take().unwrap();
    v.data.profession = Profession::Farmer;
    v.activity = Activity::Work;
    let mut changes = ChangeSet::default();
    crate::villager::harvest::work(&mut state, slot, &mut v, &mut changes);
    state.entities.row_mut(slot).villager = Some(v);
    assert_eq!(
        state.world.block_at(9, 201, 8),
        Some(carrots),
        "saved profession alone is not a settlement production slot"
    );
    assert!(changes.blocks.is_empty());
    assert_eq!(
        food_count(&state, slot, item(&state, "minecraft:carrot")),
        0
    );
}
