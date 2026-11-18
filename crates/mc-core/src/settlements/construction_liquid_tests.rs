//! D10 liquid lot preparation and execution safety. Two layers are covered:
//! the S2 tick-order liveness (a settlement executor write schedules the fluid
//! ticks its replaced source's flow needs) and the S1 admission/interface
//! (closed bowl, source-first plan order, lava-only drain gate, stand veto).
use std::collections::BTreeSet;

use super::preparation::liquid;
use super::site_refusals::{self, Reason};
use super::tests::fixture;
use super::*;
use crate::settlements::allocator::tests::block;

/// Flat stone terrain with air above: `stone` roof at `ground` and `ground+1`,
/// clear above. The pool tests dig into the top layer.
fn plateau(state: &mut TickState, ground: i32) {
    let stone = state.registry.states.stone;
    let air = state.registry.states.air;
    for x in 0..16 {
        for z in 0..16 {
            state.world.set_block(x, ground - 1, z, stone);
            state.world.set_block(x, ground, z, stone);
            for y in ground + 1..ground + 10 {
                state.world.set_block(x, y, z, air);
            }
        }
    }
    state.world.fold_overlay();
}

/// The storage blueprint's lot at `[3, 201, 3]`, with a one-deep water pool in
/// its footprint dug at `ground = 200` and the ring left solid.
fn pool_lot() -> (
    TickState,
    SettlementId,
    [i32; 3],
    [u8; 3],
    BTreeSet<[i32; 2]>,
) {
    let (mut state, id, _slot) = fixture();
    plateau(&mut state, 200);
    let blueprint = blueprints::get(&state.registry, "storage").expect("storage blueprint");
    let size = blueprint.footprint(Rotation::from_quarter_turns(0));
    let origin = [3, 201, 3];
    let floors = site_refusals::floor_columns(blueprint, 0);
    let source = state.registry.states.water_levels[0];
    for x in origin[0]..origin[0] + i32::from(size[0]) {
        for z in origin[2]..origin[2] + i32::from(size[2]) {
            state.world.set_block(x, 200, z, source);
        }
    }
    state.world.fold_overlay();
    (state, id, origin, size, floors)
}

fn check_lot(
    state: &TickState,
    id: SettlementId,
    origin: [i32; 3],
    size: [u8; 3],
    floors: &BTreeSet<[i32; 2]>,
) -> Result<site_refusals::Cost, Reason> {
    let record = state.settlements.get(id).expect("fixture settlement");
    site_refusals::check(
        state,
        record,
        origin,
        size,
        record.founding.as_ref(),
        &mut Default::default(),
        Some(floors),
    )
}

#[test]
fn closed_bowl_admits_a_pool_and_prices_its_liquid_cells_once() {
    let (state, id, origin, size, floors) = pool_lot();
    let cost = check_lot(&state, id, origin, size, &floors).expect("closed pool is a legal lot");
    let columns = u32::from(size[0]) * u32::from(size[2]);
    assert_eq!(
        cost.liquid, columns,
        "one fluid fill cell per footprint column"
    );
    assert_eq!(cost.water, columns);
    assert_eq!(cost.lava, 0);
    // A fluid fill is a liquid cell, never also a dry fill: the only dry fill
    // cells left are the floor rows of columns the blueprint does not author.
    let unfloored = columns - u32::try_from(floors.len()).expect("floor columns fit u32");
    assert_eq!(
        cost.fill, unfloored,
        "no fluid cell is counted as a dry fill"
    );
    assert_eq!(cost.total(), cost.cut + cost.fill + cost.liquid);
}

#[test]
fn an_open_bowl_refuses_the_lot() {
    let (mut state, id, origin, size, floors) = pool_lot();
    let air = state.registry.states.air;
    // One ring cell at the pool's own level is air: the water could leave.
    state.world.set_block(origin[0] - 1, 200, origin[2], air);
    state.world.fold_overlay();
    assert_eq!(
        check_lot(&state, id, origin, size, &floors),
        Err(Reason::LiquidOpenShell)
    );
}

#[test]
fn mixed_liquids_and_preserved_water_supplies_refuse() {
    let (state, id, origin, size, floors) = pool_lot();
    let mut liquids = liquid::Liquids {
        water: 1,
        lava: 1,
        ..Default::default()
    };
    assert_eq!(
        liquid::finish(&state, &liquids, origin, size, origin[1] - 2),
        Err(Reason::MixedLiquids)
    );
    liquids = liquid::Liquids {
        preserved_water: true,
        ..Default::default()
    };
    assert_eq!(
        liquid::finish(&state, &liquids, origin, size, origin[1] - 2),
        Err(Reason::LiquidInWork)
    );
    let mut mixed = liquid::Liquids {
        lava: 1,
        preserved_water: true,
        ..Default::default()
    };
    assert_eq!(
        liquid::finish(&state, &mixed, origin, size, origin[1] - 2),
        Err(Reason::MixedLiquids)
    );
    mixed = liquid::Liquids {
        water: liquid::MAX_LIQUID_CELLS as u32 + 1,
        ..Default::default()
    };
    assert_eq!(
        liquid::finish(&state, &mixed, origin, size, origin[1] - 2),
        Err(Reason::LiquidBeyondBound)
    );
}

#[test]
fn a_waterlogged_supply_in_the_work_volume_is_not_a_fill_target() {
    let (mut state, _id, origin, _size, _floors) = pool_lot();
    let at = [origin[0], origin[1] - 1, origin[2]];
    let fence = block(&state, "minecraft:oak_fence", &[("waterlogged", "true")]);
    state.world.set_block(at[0], at[1], at[2], fence);
    state.world.fold_overlay();
    let mut liquids = liquid::Liquids::default();
    let mut cost = site_refusals::Cost::default();
    liquid::scan_column(
        &state,
        &mut liquids,
        &mut cost,
        at[0],
        at[2],
        at[1] - 1,
        origin[1],
        false,
    )
    .expect("a waterlogged block is reported, not written");
    assert!(
        liquids.preserved_water,
        "contained water is a preserved supply"
    );
    assert_eq!(liquids.total(), 0);
}

#[test]
fn plan_order_puts_sources_first_top_down() {
    let (mut state, _id, _origin, _size, _floors) = pool_lot();
    let air = state.registry.states.air;
    let source = state.registry.states.water_levels[0];
    let flowing = state.registry.states.water_levels[1];
    state.world.set_block(0, 200, 0, source);
    state.world.set_block(0, 201, 0, source);
    state.world.set_block(1, 200, 0, flowing);
    state.world.set_block(1, 201, 0, air);
    state.world.fold_overlay();
    let cell = |at: [i32; 3], plan: u32| executor::PlannedCell {
        at,
        state: plan,
        resource: None,
        quantity: 0,
    };
    let mut cells = vec![
        cell([1, 200, 0], state.registry.states.dirt),
        cell([0, 200, 0], state.registry.states.dirt),
        cell([0, 201, 0], state.registry.states.dirt),
        cell([1, 201, 0], state.registry.states.dirt),
    ];
    liquid::order(&state, &mut cells);
    let order: Vec<[i32; 3]> = cells.iter().map(|cell| cell.at).collect();
    assert_eq!(
        order,
        vec![[0, 201, 0], [0, 200, 0], [1, 200, 0], [1, 201, 0]],
        "sources first, top-down inside the column, the rest keeps its order"
    );
}

#[test]
fn the_gate_waits_out_only_lava() {
    let (mut state, _id, _origin, _size, _floors) = pool_lot();
    let air = state.registry.states.air;
    let dirt = state.registry.states.dirt;
    let lava_source = state.registry.states.lava_levels[0];
    let lava_flow = state.registry.states.lava_levels[1];
    let water_flow = state.registry.states.water_levels[1];
    let clear =
        |state: &mut TickState, at: [i32; 3]| state.world.set_block(at[0], at[1], at[2], air);
    for at in [[4, 210, 4], [5, 210, 4], [6, 210, 4], [7, 210, 4]] {
        clear(&mut state, at);
    }
    state.world.set_block(5, 210, 4, lava_flow);
    state.world.set_block(6, 210, 4, lava_source);
    state.world.set_block(7, 210, 4, water_flow);
    state.world.fold_overlay();
    assert_eq!(
        liquid::write_wait(&state, [5, 210, 4], dirt),
        Some("lava_draining"),
        "a flowing lava cell is waited out, never written"
    );
    assert_eq!(
        liquid::write_wait(&state, [6, 210, 4], dirt),
        None,
        "a lava source is the fill click itself"
    );
    assert_eq!(
        liquid::write_wait(&state, [7, 210, 4], dirt),
        None,
        "water has no drain gate"
    );
    // Air write beside lava frees the cell the flow would enter.
    assert_eq!(
        liquid::write_wait(&state, [4, 210, 4], air),
        Some("lava_neighbour")
    );
    assert_eq!(liquid::write_wait(&state, [4, 210, 4], dirt), None);
    clear(&mut state, [5, 210, 4]);
    state.world.set_block(4, 210, 4, air);
    state.world.fold_overlay();
    assert_eq!(liquid::write_wait(&state, [4, 210, 4], air), None);
}

#[test]
fn the_stand_veto_refuses_lava_in_the_columns_three_rows() {
    let (mut state, _id, _origin, _size, _floors) = pool_lot();
    let lava = state.registry.states.lava_levels[0];
    assert!(liquid::stand_safe(&state, [10, 201, 10]));
    // Feet row beside the stand.
    state.world.set_block(11, 201, 10, lava);
    state.world.fold_overlay();
    assert!(!liquid::stand_safe(&state, [10, 201, 10]));
    // Head row diagonally in front of the stand.
    state
        .world
        .set_block(11, 202, 10, state.registry.states.air);
    state.world.set_block(10, 202, 10, lava);
    state.world.fold_overlay();
    assert!(!liquid::stand_safe(&state, [10, 201, 10]));
}

#[test]
fn the_liquid_stage_is_the_work_volume_sources_only() {
    let (mut state, _id, _origin, _size, _floors) = pool_lot();
    let air = state.registry.states.air;
    let source = state.registry.states.water_levels[0];
    let flowing = state.registry.states.water_levels[1];
    state.world.set_block(0, 200, 0, source);
    state.world.set_block(1, 200, 0, flowing);
    state.world.set_block(2, 200, 0, air);
    state.world.fold_overlay();
    let cell = |at: [i32; 3], plan: u32| executor::PlannedCell {
        at,
        state: plan,
        resource: None,
        quantity: 0,
    };
    let cells = vec![
        cell([0, 200, 0], state.registry.states.dirt),
        cell([1, 200, 0], state.registry.states.dirt),
        cell([2, 200, 0], state.registry.states.dirt),
        cell([3, 200, 0], air),
    ];
    assert_eq!(
        liquid::stage_cells(&state, &cells),
        vec![0],
        "only a real source needs the stage before clear"
    );
}

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
