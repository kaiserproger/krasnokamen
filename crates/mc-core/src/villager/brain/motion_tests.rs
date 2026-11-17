use super::*;

#[test]
fn a_lower_committed_waypoint_does_not_reverse_horizontal_motion_while_falling() {
    for activity in [Activity::Work, Activity::Idle] {
        for side in [-1.0, 1.0] {
            let (mut state, _, slot) = crate::settlements::construction::tests::fixture();
            let goal = BlockPos { x: 8, y: 201, z: 8 };
            let initial = [8.5 + side * 0.03, 202.1, 8.5 + side * 0.04];
            state.entities.set_pos(slot, initial);
            state.entities.set_vel(slot, [0.0; 3]);
            state.entities.set_ground(slot, false);
            let mut v = state.entities.row_mut(slot).villager.take().unwrap();
            v.activity = activity;
            v.navigation_target = Some(goal);
            v.navigation_path = vec![goal];
            let mut yaw = None;
            for _ in 0..12 {
                move_committed(&mut state, slot, &mut v, Activity::Work);
                let position = state.entities.pos(slot);
                let current_yaw = state.entities.rot(slot)[0];
                assert_eq!(*yaw.get_or_insert(current_yaw), current_yaw, "no yaw flip");
                for axis in [0, 2] {
                    assert!(
                        (position[axis] - 8.5) * side >= -1e-10,
                        "{activity:?} crossed its unchanged lower waypoint: {position:?}"
                    );
                    assert!(
                        (position[axis] - 8.5).abs() <= (initial[axis] - 8.5).abs() + 1e-10,
                        "the approach must not move away from the waypoint"
                    );
                }
                assert_eq!(v.navigation_target, Some(goal));
                state.tick += 1;
            }
            assert_eq!(state.entities.pos(slot)[1], 201.0, "gravity still lands");
            assert!(
                v.navigation_path.is_empty(),
                "vertical arrival releases the waypoint"
            );
            assert_eq!(state.entities.vel(slot), [0.0; 3]);
        }
    }
}

#[test]
fn an_idle_resident_descends_a_ledge_at_every_start_phase_without_livelock() {
    let (mut state, _, slot) = crate::settlements::construction::tests::fixture();
    for x in 9..16 {
        for z in 0..16 {
            state
                .world
                .set_block(x, 201, z, state.registry.states.stone);
        }
    }
    state.world.fold_overlay();
    let goal = BlockPos { x: 8, y: 201, z: 8 };
    let mut v = state.entities.row_mut(slot).villager.take().unwrap();
    v.activity = Activity::Idle;
    for phase in (0..25).map(|i| f64::from(i) * 0.01).chain([0.2439]) {
        state.entities.set_pos(slot, [9.0 + phase, 202.0, 8.5]);
        state.entities.set_vel(slot, [0.0; 3]);
        state.entities.set_ground(slot, true);
        v.navigation_target = Some(goal);
        v.navigation_path = vec![goal];
        for _ in 0..24 {
            let before = state.entities.pos(slot);
            move_committed(&mut state, slot, &mut v, Activity::Idle);
            let after = state.entities.pos(slot);
            assert!(
                (after[0] - before[0]).abs() <= (8.5 - before[0]).abs() + 1e-10,
                "step exceeded the remaining approach at phase {phase}: {before:?} -> {after:?}"
            );
            assert!(after[0] <= before[0] + 1e-10, "reversal at phase {phase}");
            state.tick += 1;
        }
        assert_eq!(
            state.entities.pos(slot)[1],
            201.0,
            "descent at phase {phase}"
        );
        assert!(v.navigation_path.is_empty(), "arrival at phase {phase}");
    }
}

#[test]
fn a_far_committed_waypoint_keeps_full_work_and_idle_walking_speed() {
    for (activity, speed) in [(Activity::Work, 3.0), (Activity::Idle, 5.0)] {
        let (mut state, _, slot) = crate::settlements::construction::tests::fixture();
        state.entities.set_pos(slot, [8.5, 201.0, 8.5]);
        state.entities.set_vel(slot, [0.0; 3]);
        state.entities.set_ground(slot, true);
        let mut v = state.entities.row_mut(slot).villager.take().unwrap();
        v.activity = activity;
        let goal = BlockPos {
            x: 14,
            y: 201,
            z: 8,
        };
        v.navigation_target = Some(goal);
        v.navigation_path = vec![goal];
        for _ in 0..20 {
            let before = state.entities.pos(slot);
            move_committed(&mut state, slot, &mut v, Activity::Work);
            assert!(
                (state.entities.pos(slot)[0] - before[0] - speed * mc_physics::TICK_SECONDS).abs()
                    < 1e-10
            );
            state.tick += 1;
        }
        assert!((state.entities.pos(slot)[0] - 8.5 - speed).abs() < 1e-10);
    }
}

#[test]
fn waypoint_approach_keeps_far_speed_and_does_not_clamp_external_impulse() {
    let (mut state, _, slot) = crate::settlements::construction::tests::fixture();
    state.entities.set_pos(slot, [8.5, 201.0, 8.5]);
    state.entities.set_vel(slot, [0.0; 3]);
    state.entities.set_ground(slot, true);
    let v = state.entities.row(slot).villager.as_ref().unwrap();
    let far = resolve_body_motion(
        &state,
        slot,
        Some([10.5, 201.0, 8.5]),
        3.0,
        v,
        &mut MotionGeometryCache::default(),
        false,
    )
    .unwrap();
    assert!((far.position[0] - 8.65).abs() < 1e-10);
    assert_eq!(far.velocity[0], 3.0);

    state.entities.set_brain_impulse(slot, [2.0, 0.0]);
    let v = state.entities.row(slot).villager.as_ref().unwrap();
    let near = resolve_body_motion(
        &state,
        slot,
        Some([8.53, 201.0, 8.5]),
        3.0,
        v,
        &mut MotionGeometryCache::default(),
        false,
    )
    .unwrap();
    assert!((near.position[0] - (8.53 + 2.0 * mc_physics::TICK_SECONDS)).abs() < 1e-10);
    assert!(
        near.impulse[0] > 0.0,
        "physical impulse still decays normally"
    );
}
