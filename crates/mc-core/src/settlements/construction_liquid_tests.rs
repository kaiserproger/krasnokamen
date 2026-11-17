//! D10 liquid tick liveness: a settlement executor write that replaces a liquid
//! source must schedule the fluid ticks its remaining flow needs, because the
//! executor writes after this tick's `fluids::schedule` has already run.
use super::tests::fixture;
use super::*;

/// A one-cell material-free settlement plan at `at`, the shape a road tile
/// takes, so the next admission is exactly one placement.
fn one_cell_project(
    state: &TickState,
    id: SettlementId,
    at: [i32; 3],
) -> (Project, ScriptWorldBounds, Vec<executor::PlannedCell>) {
    let cells = vec![executor::PlannedCell {
        at,
        state: state.registry.states.dirt,
        resource: None,
        quantity: 0,
    }];
    let bounds = ScriptWorldBounds::new(at, at).expect("one-cell bounds");
    let project = Project {
        settlement: id,
        blueprint: layout::ROAD.into(),
        level: 1,
        origin: [at[0], at[1] - 1, at[2]],
        rotation: 0,
        building: None,
        structure: String::new(),
        ordered: true,
        borrowed_builder: None,
    };
    (project, bounds, cells)
}

/// Place a lava source at the next planned cell and the flow it feeds beside
/// it, then let one settlement tick place that cell.
fn lava_source_and_flow_after_one_settlement_write() -> (TickState, [i32; 3], [i32; 3]) {
    let (mut state, id, _slot) = fixture();
    let at = [3, 201, 3];
    let flow = [4, 201, 3];
    let (project, bounds, cells) = one_cell_project(&state, id, at);
    let source = state.registry.states.lava_levels[0];
    let flowing = state.registry.states.lava_levels[1];
    state.world.set_block(at[0], at[1], at[2], source);
    state.world.set_block(flow[0], flow[1], flow[2], flowing);
    state.world.fold_overlay();
    // The structure image is taken here, so the lava is part of the certified
    // footprint and the next advance is not a `footprint_changed` pause.
    assert!(begin_cells(&mut state, project, bounds, cells));
    // The fixture builder already stands outside the footprint within reach of
    // the cell, so one admission is one placement.
    state.tick = 3000;
    state.tick_budget.reset(state.tick);
    super::super::tick(&mut state, &mut crate::boundary::ChangeSet::default());
    (state, at, flow)
}

#[test]
fn executor_write_over_a_lava_source_schedules_the_flow_it_fed() {
    let (mut state, at, flow) = lava_source_and_flow_after_one_settlement_write();
    // The write really happened and replaced the source, so the flow beside it
    // is the one the source used to feed.
    assert!(
        state
            .plugin_block_writes
            .iter()
            .any(|(pos, _)| [pos.x, pos.y, pos.z] == at),
        "the settlement tick must write the planned cell"
    );
    assert_ne!(
        state.world.block_at(at[0], at[1], at[2]),
        Some(state.registry.states.lava_levels[0]),
        "the planned write replaces the lava source"
    );
    let air = state.registry.states.air;
    assert_ne!(
        state.world.block_at(flow[0], flow[1], flow[2]),
        Some(air),
        "the flow still holds lava right after the source write"
    );
    let (mut writes, mut spawned) = (Vec::new(), Vec::new());
    for _ in 0..40 {
        state.tick += 1;
        crate::fluids::run(&mut state, &mut writes, &mut spawned);
    }
    assert_eq!(
        state.world.block_at(flow[0], flow[1], flow[2]),
        Some(air),
        "the flow the replaced source fed must recede on its due fluid tick"
    );
}
