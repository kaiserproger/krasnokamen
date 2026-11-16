use super::*;
use crate::settlements::construction::tests::{attempt, fixture, fund, start};

#[test]
fn test_acceleration_scales_canonical_builder_steps() {
    for factor in [1.0, 4.0] {
        let (mut state, town, builder) = fixture();
        state.test_acceleration.construction_speed = factor;
        start(&mut state, town, "house", 1, None);
        fund(&mut state, "house", 1);
        let handle = state
            .settlements
            .get(town)
            .unwrap()
            .project
            .as_ref()
            .unwrap()
            .structure
            .clone();
        let work = |state: &TickState| {
            let status = executor::status(state, town, &handle).unwrap();
            status
                .stages
                .iter()
                .take(status.current_stage_index as usize)
                .map(|stage| stage.work_units)
                .sum::<u64>()
                + status.completed_work_units
        };
        let mut maximum = 0;
        for _ in 0..64 {
            let before = work(&state);
            attempt(&mut state, town, builder);
            maximum = maximum.max(work(&state) - before);
            if maximum == factor as u64 {
                break;
            }
        }
        assert_eq!(maximum, factor as u64, "canonical work per paid admission");
    }
}

#[test]
fn builder_finishes_the_last_roof_cells_beside_a_lower_perimeter() {
    let (mut state, town, builder) = fixture();
    start(&mut state, town, "spanish_church", 1, None);
    fund(&mut state, "spanish_church", 1);
    // The shared paid fixture constructs the initial cells with declared
    // navigation assistance. Only the last four cells exercise real movement.
    for _ in 0..4096 {
        let project = state
            .settlements
            .get(town)
            .unwrap()
            .project
            .as_ref()
            .unwrap();
        let status = executor::status(&state, town, &project.structure).unwrap();
        if status.stages[status.current_stage_index as usize].work_units
            - status.completed_work_units
            == 4
        {
            break;
        }
        attempt(&mut state, town, builder);
    }
    let project = state
        .settlements
        .get(town)
        .unwrap()
        .project
        .clone()
        .unwrap();
    let next = executor::next_cell(&state, town, &project.structure)
        .unwrap()
        .unwrap();
    assert!(
        next[1] >= project.origin[1] + 10,
        "setup: final high roof row: {next:?}"
    );
    let size = blueprints::get(&state.registry, &project.blueprint)
        .unwrap()
        .footprint(blueprints::Rotation::from_quarter_turns(project.rotation));
    // The nearest west perimeter is three blocks below the site, like the
    // retained church. Other sides remain supported and within placement reach.
    for x in 0..=2 {
        for z in 3..=3 + i32::from(size[2]) {
            state
                .world
                .set_block(x, 197, z, state.registry.states.stone);
            for y in 198..=200 {
                state.world.set_block(x, y, z, state.registry.states.air);
            }
        }
    }
    for x in 0..=2 {
        for (z, floor) in [(2, 198), (1, 199)] {
            state
                .world
                .set_block(x, floor, z, state.registry.states.stone);
            for y in floor + 1..=200 {
                if (x, z) != (1, 1) {
                    state.world.set_block(x, y, z, state.registry.states.air);
                }
            }
        }
    }
    state.world.fold_overlay();
    state
        .entities
        .set_pos(builder, [1.5, 198.0, f64::from(next[2]) + 0.5]);
    state.entities.set_vel(builder, [0.0; 3]);
    let mut v = state.entities.row_mut(builder).villager.take().unwrap();
    let goal = work(&mut state, builder, &mut v, &mut ChangeSet::default()).unwrap();
    state.entities.row_mut(builder).villager = Some(v);
    let body = state.entities.body(builder);
    assert!(
        crate::settlements::allocator::standing(
            &state.world,
            &state.registry,
            goal.pos,
            body.half_width,
            body.height
        )
        .is_some(),
        "builder selected an unsupported fixed-height perimeter: {:?}; next={next:?}",
        goal.pos
    );
    for _ in 0..2000 {
        crate::tick::run_tick(&mut state, &[]);
        if executor::built_at(&state, town, "spanish_church", project.origin) {
            return;
        }
    }
    panic!(
        "last roof cells did not finish through native movement: {:?}; next={:?}",
        state.entities.pos(builder),
        executor::next_cell(&state, town, &project.structure)
    );
}

fn final_approach_fixture() -> (TickState, SettlementId, EntitySlot, construction::Project) {
    let (mut state, town, builder) = fixture();
    crate::settlements::construction::tests::open_plan(&mut state);
    fund(&mut state, "spanish_house", 1);
    start(&mut state, town, "spanish_house", 1, None);
    let project = state
        .settlements
        .get(town)
        .unwrap()
        .project
        .clone()
        .unwrap();
    // Setup uses the shared paid executor fixture; the final approach below
    // runs only native allocator/brain motion, without navigation assistance.
    for _ in 0..32 {
        if executor::next_cell(&state, town, &project.structure).unwrap() == Some([5, 200, 3]) {
            break;
        }
        attempt(&mut state, town, builder);
    }
    assert_eq!(
        executor::next_cell(&state, town, &project.structure).unwrap(),
        Some([5, 200, 3])
    );
    // A lower supported perimeter puts the next floor cell at the same
    // elevation. Its legal centre is sqrt(10² + 6²) blocks from that cell;
    // .35 farther along that radius is just outside placement reach.
    for x in 14..=18 {
        for z in 3..=11 {
            state
                .world
                .set_block(x, 199, z, state.registry.states.stone);
            state.world.set_block(x, 200, z, state.registry.states.air);
        }
    }
    state.world.fold_overlay();
    state.tickets.insert(
        "arrival-stand".into(),
        (crate::boundary::ChunkPos { x: 1, z: 0 }, 1),
    );
    state.refresh_active_chunks(state.tick);
    let (item, count) = executor::next_material(&state, town, &project.structure)
        .unwrap()
        .unwrap();
    assert!(carried(
        state.entities.row(builder).villager.as_ref().unwrap(),
        item,
        count
    ));
    (state, town, builder, project)
}

fn builder_final_approach(offset: f64) {
    let (mut state, town, builder, project) = final_approach_fixture();
    let radius = 136.0_f64.sqrt();
    let initial = [
        15.5 + offset * 10.0 / radius,
        200.0,
        9.5 + offset * 6.0 / radius,
    ];
    let before = executor::status(&state, town, &project.structure)
        .unwrap()
        .completed_work_units;
    state.entities.set_pos(builder, initial);
    state.entities.set_vel(builder, [0.0; 3]);
    let mut v = state.entities.row_mut(builder).villager.take().unwrap();
    let target = work(&mut state, builder, &mut v, &mut ChangeSet::default()).unwrap();
    state.entities.row_mut(builder).villager = Some(v);
    assert_eq!(
        target.exact,
        offset > crate::villager::brain::WAYPOINT_REACH
    );
    assert_eq!(
        target.pos,
        BlockPos {
            x: 15,
            y: 200,
            z: 9
        }
    );
    for _ in 0..40 {
        state.day_time = 4000;
        state.tick_budget.reset(state.tick);
        let mut changes = ChangeSet::default();
        crate::settlements::allocator::tick(&mut state, &mut changes);
        crate::villager::brain::tick(&mut state, &mut changes);
        state.tick += 1;
        let pos = state.entities.pos(builder);
        if offset > crate::villager::brain::WAYPOINT_REACH {
            if (pos[0] - 15.5).powi(2) + (pos[2] - 9.5).powi(2)
                < crate::villager::brain::WAYPOINT_REACH.powi(2)
            {
                break;
            }
        } else {
            assert_eq!(
                pos, initial,
                "an already arrived builder must not take another step"
            );
        }
    }
    let pos = state.entities.pos(builder);
    assert!(
        (pos[0] - 15.5).powi(2) + (pos[2] - 9.5).powi(2)
            < crate::villager::brain::WAYPOINT_REACH.powi(2),
        "exact zero-waypoint route must finish the approach: pos={pos:?}, target={:?}",
        target.pos
    );
    tick(&mut state, town);
    assert!(
        executor::status(&state, town, &project.structure)
            .unwrap()
            .completed_work_units
            > before,
        "the arrived builder must advance the paid canonical project"
    );
}

#[test]
fn builder_finishes_an_exact_zero_waypoint_approach_from_outside_arrival_tolerance() {
    builder_final_approach(0.35);
}

#[test]
fn builder_already_inside_arrival_tolerance_takes_no_extra_step() {
    builder_final_approach(0.2);
}

#[test]
fn an_in_reach_builder_holds_position_instead_of_recentering_out_of_reach() {
    let (mut state, town, builder, project) = final_approach_fixture();
    let initial = [17.05, 200.0, 6.5];
    state.entities.set_pos(builder, initial);
    state.entities.set_vel(builder, [0.0; 3]);
    let before = executor::status(&state, town, &project.structure)
        .unwrap()
        .completed_work_units;
    for _ in 0..40 {
        state.day_time = 4000;
        state.tick_budget.reset(state.tick);
        let mut changes = ChangeSet::default();
        crate::settlements::allocator::tick(&mut state, &mut changes);
        crate::villager::brain::tick(&mut state, &mut changes);
        state.tick += 1;
        assert_eq!(
            state.entities.pos(builder),
            initial,
            "working feet must not be recentered"
        );
    }
    tick(&mut state, town);
    assert!(
        executor::status(&state, town, &project.structure)
            .unwrap()
            .completed_work_units
            > before
    );
}
