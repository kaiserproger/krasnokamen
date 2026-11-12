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
        if at(pos) == target && state.entities.ground(slot) {
            break;
        }
    }
    let pos = state.entities.pos(slot);
    assert!(wet, "setup: the route wades the channel");
    assert_eq!(
        at(pos),
        target,
        "resident must cross the channel onto the far bank, stopped at {pos:?}"
    );
}
