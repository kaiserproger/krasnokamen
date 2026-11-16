//! VILLAGER-IDLE-STROLL-UNBOUNDED-DRIFT: residents stroll inside their village
//! and swim up out of deep water (vanilla VillageBoundRandomStroll and Swim).
//! The tests drive the brain only through symbols that predate the fix, so
//! they still build with the fix reversed.

use super::*;

fn manhattan(a: BlockPos, b: BlockPos) -> u32 {
    a.x.abs_diff(b.x) + a.z.abs_diff(b.z)
}

/// Seven by seven loaded chunks of flat stone around a bell at (8, 201, 8):
/// 56 open blocks on every side of the meeting point.
fn village_state() -> (TickState, BlockPos) {
    let mut state = test_state();
    let chunks: Vec<_> = (-3..=3)
        .flat_map(|x| (-3..=3).map(move |z| crate::boundary::ChunkPos { x, z }))
        .collect();
    let want = FxHashMap::from_iter(chunks.iter().map(|&chunk| (chunk, 0)));
    state.world.ensure_blocking(&want);
    for x in -48..64 {
        for z in -48..64 {
            state
                .world
                .set_block(x, 200, z, state.registry.states.stone);
            for y in 201..206 {
                state.world.set_block(x, y, z, state.registry.states.air);
            }
        }
    }
    let meeting = BlockPos { x: 8, y: 201, z: 8 };
    let bell = block(&state, "minecraft:bell", &[]);
    state.world.set_block(meeting.x, meeting.y, meeting.z, bell);
    state.world.fold_overlay();
    for chunk in chunks {
        state
            .tickets
            .insert(format!("village-{}-{}", chunk.x, chunk.z), (chunk, 0));
    }
    state.refresh_active_chunks(0);
    state.gamerules.insert("spawn_mobs".into(), "false".into());
    (state, meeting)
}

#[test]
fn idle_strolls_stay_bound_to_the_village_over_many_decisions() {
    let (mut state, meeting) = village_state();
    state.day_time = 11_500;
    let slot = villager(&mut state, [10.5, 201.0, 8.5]);
    state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .meeting_point = Some(meeting);
    let mut decisions = 0;
    let mut last = None;
    let mut farthest = 0;
    for _ in 0..12_000 {
        step(&mut state);
        let cell = at(state.entities.pos(slot));
        farthest = farthest.max(cell.x.abs_diff(meeting.x).max(cell.z.abs_diff(meeting.z)));
        let v = state.entities.row(slot).villager.as_ref().unwrap();
        if v.navigation_target != last {
            last = v.navigation_target;
            decisions += 1;
        }
    }
    let v = state.entities.row(slot).villager.as_ref().unwrap();
    assert_eq!(v.activity, Activity::Idle, "setup: the resident idles");
    assert_eq!(
        v.meeting_point,
        Some(meeting),
        "setup: the meeting point stays"
    );
    assert!(decisions >= 200, "setup: only {decisions} stroll decisions");
    // In-village cells lie within one section of the meeting point's section
    // (at most 31 blocks per axis); one stroll step adds at most 6, walking
    // one more cell.
    assert!(
        farthest <= 31 + 6 + 1,
        "{decisions} strolls carried the resident {farthest} blocks from the meeting point"
    );
}

#[test]
fn idle_resident_outside_the_village_strolls_back_toward_it() {
    let mut state = test_state();
    state.day_time = 11_500;
    let slot = villager(&mut state, [8.5, 201.0, 8.5]);
    // Four sections east, in an unloaded chunk: an unloaded cell is not
    // evidence that the bell vanished, so the memory stays.
    let meeting = BlockPos {
        x: 72,
        y: 201,
        z: 8,
    };
    state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .meeting_point = Some(meeting);
    let mut decisions = Vec::new();
    let mut last = None;
    for _ in 0..400 {
        let from = at(state.entities.pos(slot));
        step(&mut state);
        let v = state.entities.row(slot).villager.as_ref().unwrap();
        assert_eq!(
            v.meeting_point,
            Some(meeting),
            "setup: the meeting point stays"
        );
        if v.navigation_target != last {
            last = v.navigation_target;
            if let Some(target) = last {
                assert_eq!(v.activity, Activity::Idle, "setup: the resident idles");
                decisions.push((from, target));
            }
        }
    }
    assert!(!decisions.is_empty(), "setup: the idle resident strolls");
    for (from, target) in decisions {
        assert!(
            manhattan(target, meeting) < manhattan(from, meeting),
            "stroll from {from:?} to {target:?} leads away from the meeting point {meeting:?}"
        );
    }
}

#[test]
fn resident_in_deep_water_swims_up_to_the_surface() {
    let mut state = test_state();
    state.day_time = 11_500;
    let water = block(&state, "minecraft:water", &[]);
    for x in 0..16 {
        for z in 0..16 {
            for y in 201..=205 {
                state.world.set_block(x, y, z, water);
            }
        }
    }
    state.world.fold_overlay();
    // On the floor of five-deep water, like the seed-24 nitwit.
    let slot = villager(&mut state, [8.5, 201.0, 8.5]);
    let mut lowest_late = f64::MAX;
    for tick in 0..200 {
        step(&mut state);
        if tick >= 160 {
            lowest_late = lowest_late.min(state.entities.pos(slot)[1]);
        }
    }
    assert!(
        lowest_late > 205.0,
        "resident stayed under water: lowest feet y {lowest_late} over the last 40 ticks"
    );
    assert!(
        state.entities.pos(slot)[1] < 206.5,
        "swimming must not lift the resident out of the water: feet y {}",
        state.entities.pos(slot)[1]
    );
}

/// VILLAGER-STRANDED-IN-DEEP-WATER: a 7x7 pond dug into a stone pad (banks
/// level with the water top) with no bank cell next to the centre. Vanilla
/// `setCanFloat` plus `WalkNodeEvaluator` path a floating villager over the
/// surface to the bank, and `jumpOutOfFluid` lifts it out.
#[test]
fn resident_in_a_pond_with_no_adjacent_bank_swims_to_the_bank() {
    for depth in [2, 5] {
        let mut state = test_state();
        // The meeting hour: the resident walks to its bell.
        state.day_time = 9_500;
        let water = block(&state, "minecraft:water", &[]);
        for x in 0..16 {
            for z in 0..16 {
                for y in 201..=205 {
                    let pond = (4..=10).contains(&x) && (4..=10).contains(&z) && y > 205 - depth;
                    let id = if pond {
                        water
                    } else {
                        state.registry.states.stone
                    };
                    state.world.set_block(x, y, z, id);
                }
            }
        }
        let meeting = BlockPos { x: 1, y: 206, z: 7 };
        let bell = block(&state, "minecraft:bell", &[]);
        state.world.set_block(meeting.x, meeting.y, meeting.z, bell);
        state.world.fold_overlay();
        // On the pond floor: the nearest bank cell is four blocks away.
        let slot = villager(&mut state, [7.5, f64::from(206 - depth), 7.5]);
        state
            .entities
            .row_mut(slot)
            .villager
            .as_mut()
            .unwrap()
            .meeting_point = Some(meeting);
        let mut arrived = None;
        for tick in 0..900 {
            step(&mut state);
            let pos = state.entities.pos(slot);
            if arrived.is_none()
                && (pos[0] - 1.5).powi(2) + (pos[2] - 7.5).powi(2) < 2.5 * 2.5
                && pos[1] >= 206.0
                && state.entities.ground(slot)
            {
                arrived = Some(tick);
            }
        }
        assert!(
            arrived.is_some(),
            "{depth}-deep pond: the resident never reached the bell on the bank, ended at {:?}",
            state.entities.pos(slot)
        );
    }
}

/// VILLAGER-AFTER-HOURS-CLAIM-NOT-EMPLOYED: a workstation placed after work
/// hours, 45 blocks from a jobless resident. Vanilla GoToPotentialJobSite runs
/// in the Idle package too, so the claimer walks there and takes the
/// profession instead of strolling inside the village jobless until the next
/// work period.
#[test]
fn jobless_villager_walks_to_an_after_hours_claim_and_is_employed() {
    // Idle: day ticks 10..2000 and 11000..12000.
    for time in [1_000, 11_000, 11_500] {
        let (mut state, _) = village_state();
        state.day_time = time;
        let composter = block(&state, "minecraft:composter", &[]);
        let site = BlockPos {
            x: 40,
            y: 201,
            z: 8,
        };
        state.world.set_block(site.x, site.y, site.z, composter);
        state.world.fold_overlay();
        let slot = villager(&mut state, [-5.5, 201.0, 8.5]);
        let mut employed_at = None;
        for tick in 0..600 {
            step(&mut state);
            let v = state.entities.row(slot).villager.as_ref().unwrap();
            if v.data.profession == Profession::Farmer {
                employed_at = Some(tick);
                break;
            }
        }
        let v = state.entities.row(slot).villager.as_ref().unwrap();
        assert_eq!(v.job_site, Some(site), "day time {time}: the claim");
        assert!(
            employed_at.is_some(),
            "day time {time}: claimed but still {:?} at {:?}",
            v.data.profession,
            state.entities.pos(slot)
        );
    }
}

#[test]
fn resident_wading_a_sunken_ford_climbs_out_onto_the_far_bank() {
    let mut state = test_state();
    let water = block(&state, "minecraft:water", &[]);
    // A foot-deep channel across x = 6..=7: floor at y = 199, water at y = 200,
    // banks at y = 201 on both sides (vanilla jumpOutOfFluid lifts swimmers out).
    for z in 0..16 {
        for x in 6..=7 {
            state
                .world
                .set_block(x, 199, z, state.registry.states.stone);
            state.world.set_block(x, 200, z, water);
        }
    }
    state.world.fold_overlay();
    let slot = villager(&mut state, [4.5, 201.0, 8.5]);
    let target = BlockPos {
        x: 10,
        y: 201,
        z: 8,
    };
    let mut v = VillagerState::default();
    let mut cache = PoiCache::default();
    let mut wet = false;
    for _ in 0..400 {
        move_to(&mut state, &mut cache, slot, &mut v, Some(target), false);
        state.tick += 1;
        let pos = state.entities.pos(slot);
        wet |= pos[1] < 201.0;
        // Past the channel's far edge (x = 8) by more than the half width,
        // standing on the bank; the route ends short of the target cell.
        if pos[0] > 8.3 && pos[1] >= 201.0 && state.entities.ground(slot) {
            break;
        }
    }
    let pos = state.entities.pos(slot);
    assert!(wet, "setup: the route wades the channel");
    assert!(
        pos[0] > 8.3 && pos[1] >= 201.0 && state.entities.ground(slot),
        "resident must climb out onto the far bank, stopped at {pos:?}"
    );
}

#[test]
fn short_stroll_rejects_a_roofed_well_but_keeps_a_dry_neighbor() {
    let (mut state, _) = village_state();
    let wood = block(&state, "minecraft:acacia_wood", &[]);
    let leaves = block(&state, "minecraft:acacia_leaves", &[]);
    let water = block(&state, "minecraft:water", &[]);
    for x in 7..=9 {
        for z in 7..=9 {
            state.world.set_block(x, 201, z, wood);
            state.world.set_block(x, 204, z, leaves);
        }
    }
    state.world.set_block(8, 200, 8, wood);
    state.world.set_block(8, 201, 8, water);
    state.world.fold_overlay();
    let slot = villager(&mut state, [7.5, 202.0, 8.5]);
    let mut v = VillagerState::default();
    wander_to(
        &mut state,
        slot,
        &mut v,
        Some(BlockPos { x: 9, y: 202, z: 8 }),
        Activity::Idle,
    );
    assert!(
        v.navigation_path.is_empty(),
        "a land stroll must not descend through well water: {:?}",
        v.navigation_path
    );
    wander_to(
        &mut state,
        slot,
        &mut v,
        Some(BlockPos { x: 6, y: 202, z: 8 }),
        Activity::Idle,
    );
    assert!(
        !v.navigation_path.is_empty(),
        "ordinary dry stroll still works"
    );
}

#[test]
fn work_route_avoids_roofed_well_and_reaches_the_opposite_rim() {
    let (mut state, _) = village_state();
    let wood = block(&state, "minecraft:acacia_wood", &[]);
    let leaves = block(&state, "minecraft:acacia_leaves", &[]);
    let water = block(&state, "minecraft:water", &[]);
    for x in 7..=9 {
        for z in 7..=9 {
            state.world.set_block(x, 201, z, wood);
            state.world.set_block(x, 204, z, leaves);
        }
    }
    state.world.set_block(8, 200, 8, wood);
    state.world.set_block(8, 201, 8, water);
    state.world.fold_overlay();
    let slot = villager(&mut state, [7.5, 202.0, 8.5]);
    let mut v = VillagerState::default();
    let mut cache = PoiCache::default();
    let target = BlockPos {
        x: 10,
        y: 202,
        z: 8,
    };
    for _ in 0..400 {
        begin_brain_tick(&mut state);
        move_to(&mut state, &mut cache, slot, &mut v, Some(target), false);
        let pos = state.entities.pos(slot);
        assert!(
            !(pos[0] > 8.0 && pos[0] < 9.0 && pos[2] > 8.0 && pos[2] < 9.0),
            "work routing entered the roofed well instead of its dry rim: {pos:?}, path={:?}",
            v.navigation_path
        );
        if pos[0] > 9.0 {
            return;
        }
        state.tick += 1;
    }
    panic!(
        "worker never reached the far rim: {:?}, path={:?}",
        state.entities.pos(slot),
        v.navigation_path
    );
}

#[test]
fn meeting_route_stays_on_the_dry_rim_of_a_roofed_well() {
    for start in [[9.5, 202.0, 8.5], [15.5, 201.0, 8.5]] {
        let (mut state, _) = village_state();
        let wood = block(&state, "minecraft:acacia_wood", &[]);
        let water = block(&state, "minecraft:water", &[]);
        let leaves = block(&state, "minecraft:acacia_leaves", &[]);
        let fence = block(&state, "minecraft:acacia_fence", &[]);
        let slab = block(&state, "minecraft:acacia_slab", &[]);
        let bell = block(&state, "minecraft:bell", &[]);
        let chest = block(&state, "minecraft:chest", &[]);
        for x in 5..=11 {
            for z in 3..=12 {
                state
                    .world
                    .set_block(x, 201, z, state.registry.states.stone);
            }
        }
        for x in 7..=9 {
            for z in [4, 5, 7, 8, 9] {
                state.world.set_block(x, 201, z, wood);
            }
            for z in 5..=7 {
                state.world.set_block(x, 204, z, slab);
            }
            for z in 8..=9 {
                state.world.set_block(x, 204, z, leaves);
            }
        }
        for x in [7, 9] {
            for z in [5, 7] {
                for y in 202..=203 {
                    state.world.set_block(x, y, z, fence);
                }
            }
        }
        state.world.set_block(8, 201, 4, water);
        state.world.set_block(8, 201, 8, water);
        state.world.set_block(8, 200, 8, wood);
        let meeting = BlockPos { x: 8, y: 203, z: 6 };
        state.world.set_block(meeting.x, meeting.y, meeting.z, bell);
        state.world.set_block(9, 202, 6, chest);
        state.world.fold_overlay();
        state.day_time = 10_000;
        let slot = villager(&mut state, start);
        state
            .entities
            .row_mut(slot)
            .villager
            .as_mut()
            .unwrap()
            .meeting_point = Some(meeting);
        let mut arrived = false;
        for _ in 0..400 {
            step(&mut state);
            let pos = state.entities.pos(slot);
            let v = state.entities.row(slot).villager.as_ref().unwrap();
            assert_eq!(v.activity, Activity::Meet);
            assert!(
                !(pos[0] > 8.0 && pos[0] < 9.0 && pos[2] > 8.0 && pos[2] < 9.0),
                "MEET enters the well at {pos:?}; target={:?}, path={:?}",
                v.navigation_target,
                v.navigation_path
            );
            let here = at(pos);
            arrived |= here.x.abs_diff(meeting.x)
                + here.y.abs_diff(meeting.y)
                + here.z.abs_diff(meeting.z)
                <= 6;
        }
        assert!(
            arrived,
            "MEET from {start:?} never reached the bell's gathering area"
        );
    }
}

#[test]
fn meeting_arrival_stops_committed_motion_but_not_a_different_delivery() {
    let mut state = test_state();
    let meeting = BlockPos { x: 8, y: 201, z: 8 };
    let bell = block(&state, "minecraft:bell", &[]);
    state.world.set_block(meeting.x, meeting.y, meeting.z, bell);
    state.world.fold_overlay();
    let slot = villager(&mut state, [10.5, 201.0, 8.5]);
    let mut v = VillagerState {
        meeting_point: Some(meeting),
        navigation_target: Some(meeting),
        navigation_path: vec![BlockPos { x: 9, y: 201, z: 8 }],
        ..Default::default()
    };
    move_committed(&mut state, slot, &mut v, Activity::Meet);
    assert!(
        v.navigation_path.is_empty(),
        "a committed meeting route must stop on arrival"
    );
    assert_eq!(state.entities.pos(slot)[0], 10.5);
    let delivery = BlockPos {
        x: 14,
        y: 201,
        z: 8,
    };
    v.navigation_target = Some(delivery);
    v.navigation_path.push(delivery);
    move_committed(&mut state, slot, &mut v, Activity::Meet);
    assert!(
        state.entities.pos(slot)[0] > 10.5,
        "a different destination must still be approached"
    );
}
