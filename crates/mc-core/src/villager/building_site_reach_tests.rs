use super::*;
use crate::settlements::construction::tests::{attempt, fixture, fund, start};

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
