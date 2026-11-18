//! D10 liquid lot preparation and execution safety. Two layers are covered:
//! the S2 tick-order liveness (a settlement executor write schedules the fluid
//! ticks its replaced source's flow needs) and the S1 admission/interface
//! (closed bowl, source-first plan order, lava-only drain gate, stand veto).
use std::collections::BTreeSet;

use super::preparation::liquid;
use super::site_refusals::{self, Reason};
use super::tests::fixture;
use super::*;
use crate::entities::EntitySlot;
use crate::settlements::allocator::tests::block;

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
    // A loaded flat plan: stone ground at 200 (plus a 199 row for the pool
    // bottoms) and air above, so both `check` and `planned_cells` can run.
    super::tests::open_plan(&mut state);
    let stone = state.registry.states.stone;
    for x in 0..16 {
        for z in 0..16 {
            state.world.set_block(x, 199, z, stone);
        }
    }
    state.world.fold_overlay();
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
fn mixed_liquids_refuse() {
    let (state, _id, origin, size, _floors) = pool_lot();
    let liquids = liquid::Liquids {
        water: 1,
        lava: 1,
        ..Default::default()
    };
    assert_eq!(
        liquid::finish(
            &state,
            &liquids,
            origin,
            max_of(origin, size),
            origin[1] - 2,
            &[]
        ),
        Err(Reason::MixedLiquids)
    );
    // A preserved water source (waterlogged/bubble) counts as water for mixing.
    let preserved = liquid::Liquids {
        lava: 1,
        preserved_water: true,
        ..Default::default()
    };
    assert_eq!(
        liquid::finish(
            &state,
            &preserved,
            origin,
            max_of(origin, size),
            origin[1] - 2,
            &[]
        ),
        Err(Reason::MixedLiquids)
    );
}

#[test]
fn a_preserved_water_supply_alone_refuses() {
    let (state, _id, origin, size, _floors) = pool_lot();
    let liquids = liquid::Liquids {
        preserved_water: true,
        ..Default::default()
    };
    assert_eq!(
        liquid::finish(
            &state,
            &liquids,
            origin,
            max_of(origin, size),
            origin[1] - 2,
            &[]
        ),
        Err(Reason::LiquidInWork)
    );
}

#[test]
fn the_liquid_bound_refuses() {
    let (state, _id, origin, size, _floors) = pool_lot();
    let liquids = liquid::Liquids {
        water: liquid::MAX_LIQUID_CELLS as u32 + 1,
        ..Default::default()
    };
    assert_eq!(
        liquid::finish(
            &state,
            &liquids,
            origin,
            max_of(origin, size),
            origin[1] - 2,
            &[]
        ),
        Err(Reason::LiquidBeyondBound)
    );
}

#[test]
fn a_waterlogged_supply_in_the_work_volume_is_not_a_fill_target() {
    let (mut state, id, origin, _size, _floors) = pool_lot();
    let at = [origin[0], origin[1] - 1, origin[2]];
    let fence = block(&state, "minecraft:oak_fence", &[("waterlogged", "true")]);
    state.world.set_block(at[0], at[1], at[2], fence);
    state.world.fold_overlay();
    let mut liquids = liquid::Liquids::default();
    let mut cost = site_refusals::Cost::default();
    liquid::scan_column(
        &state,
        state.settlements.get(id).unwrap(),
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
        builder: None,
        order_seq: None,
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

/// Replace the whole footprint at `y` with one fluid state (no authored floor
/// test setup): every column then looks like a one-deep pool.
fn fill_footprint(state: &mut TickState, origin: [i32; 3], size: [u8; 3], block: u32) {
    for x in origin[0]..origin[0] + i32::from(size[0]) {
        for z in origin[2]..origin[2] + i32::from(size[2]) {
            state.world.set_block(x, 200, z, block);
        }
    }
    state.world.fold_overlay();
}

/// Owner D27.4: a player-attributed fluid fill cell is the village's property
/// inside the territory; outside it (a founding site of a settlement whose
/// territory lies far away) it is never a fill target.
#[test]
fn a_player_attributed_fluid_below_the_floor_refuses_outside_the_territory_only() {
    let (mut state, id, origin, size, floors) = pool_lot();
    let at = [origin[0], origin[1] - 1, origin[2]];
    state.block_log = crate::settlements::forestry::tests::player_history(&state, at, 0);
    assert!(state.block_log.player_touched(at));
    assert!(state.settlements.get(id).unwrap().contains(at));
    let before = state.world.block_at(at[0], at[1], at[2]);
    let slots = state.block_entities[&(1, 201, 1)].slots.clone();
    assert!(
        check_lot(&state, id, origin, size, &floors).is_ok(),
        "inside the territory a player's water cell is an ordinary fill target"
    );
    let record = state.settlements.get_mut(id).unwrap();
    record.anchor = [origin[0] + 1_000, origin[1], origin[2]];
    record.radius = 8;
    let expedition = FoundingExpedition {
        destination: [
            origin[0] - 1,
            origin[1] + i32::from(size[1]) - 6,
            origin[2] - 1,
        ],
        household: String::new(),
    };
    let record = state.settlements.get(id).unwrap();
    assert!(!record.contains(at));
    assert_eq!(
        site_refusals::check(
            &state,
            record,
            origin,
            size,
            Some(&expedition),
            &mut Default::default(),
            Some(&floors),
        ),
        Err(Reason::ProtectedObstruction),
        "outside the territory a player-attributed water cell is never a fill target"
    );
    assert_eq!(state.world.block_at(at[0], at[1], at[2]), before);
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, slots);
}

/// Owner D27.4: a resume re-proves the certified envelope without reading
/// block history inside the working settlement's territory; outside it a
/// player-attributed cell in the envelope keeps refusing the resume.
#[test]
fn a_resume_reads_player_history_outside_the_territory_only() {
    let (mut state, id, origin, size, _floors) = pool_lot();
    let project = Project {
        settlement: id,
        blueprint: "storage".into(),
        level: 1,
        origin,
        rotation: 0,
        building: None,
        structure: String::new(),
        ordered: false,
        builder: None,
        order_seq: None,
    };
    let (site, cells) =
        super::preparation::planned_cells(&state, &project, "oak", &mut Default::default())
            .expect("the pool lot plans");
    let proof = liquid::proof(&state, &cells, site.min[1]).expect("a liquid plan has a proof");
    let at = proof.cells[0];
    state.block_log = crate::settlements::forestry::tests::player_history(&state, at, 0);
    assert!(state.block_log.player_touched(at));
    let footprint = size.map(i32::from);
    let mut record = state.settlements.get(id).unwrap().clone();
    assert!(record.contains(at));
    assert!(
        liquid::revalidate(&state, Some(&record), &proof, origin, footprint).is_ok(),
        "inside the territory the player's water cell is the village's"
    );
    record.anchor = [origin[0] + 1_000, origin[1], origin[2]];
    record.radius = 8;
    assert!(
        matches!(
            liquid::revalidate(&state, Some(&record), &proof, origin, footprint),
            Err(mc_script::ScriptOperationFailure::Blocked)
        ),
        "outside the territory a player's block in the envelope refuses the resume"
    );
}

#[test]
fn a_tree_in_the_ring_does_not_close_the_bowl() {
    let (mut state, id, origin, size, floors) = pool_lot();
    let log = block(&state, "minecraft:oak_log", &[]);
    state.world.set_block(origin[0] - 1, 200, origin[2], log);
    state.world.fold_overlay();
    assert_eq!(
        check_lot(&state, id, origin, size, &floors),
        Err(Reason::LiquidOpenShell),
        "a ring tree is felled by this same plan, so it is not a wall"
    );
}

#[test]
fn a_cave_under_a_neighbouring_column_opens_the_bowl() {
    let (mut state, id, origin, size, floors) = pool_lot();
    let air = state.registry.states.air;
    let stone = state.registry.states.stone;
    // Support stays at 200 with a pocket at 199, while the pool floors are 199:
    // the fluid could flow sideways and descend under the ring.
    state.world.set_block(origin[0], 200, origin[2], stone);
    state.world.set_block(origin[0], 199, origin[2], air);
    state.world.fold_overlay();
    assert_eq!(
        check_lot(&state, id, origin, size, &floors),
        Err(Reason::LiquidOpenShell)
    );
}

#[test]
fn a_combustible_ring_is_not_a_retaining_shell_for_lava() {
    let (mut state, id, origin, size, floors) = pool_lot();
    let lava = state.registry.states.lava_levels[0];
    fill_footprint(&mut state, origin, size, lava);
    let planks = block(&state, "minecraft:oak_planks", &[]);
    state.world.set_block(origin[0] - 1, 200, origin[2], planks);
    state.world.fold_overlay();
    assert_eq!(
        check_lot(&state, id, origin, size, &floors),
        Err(Reason::CombustibleShell)
    );
    // The same ring holds water: water does not ignite it.
    let water = state.registry.states.water_levels[0];
    fill_footprint(&mut state, origin, size, water);
    state.world.fold_overlay();
    assert!(
        check_lot(&state, id, origin, size, &floors).is_ok(),
        "a wooden ring is a legal bowl for water"
    );
}

#[test]
fn a_fluid_on_an_unfloored_floor_row_is_one_paid_fill() {
    let (mut state, id, origin, size, floors) = pool_lot();
    // A column the blueprint does not floor: its floor row may be a fluid cell,
    // and the plan writes exactly one paid fill cell there.
    let column = origin[0]..origin[0] + i32::from(size[0]);
    let unfloored = column
        .flat_map(|x| (origin[2]..origin[2] + i32::from(size[2])).map(move |z| [x, z]))
        .find(|[x, z]| !floors.contains(&[x - origin[0], z - origin[2]]))
        .expect("a storage lot keeps unfloored columns");
    let at = [unfloored[0], origin[1], unfloored[1]];
    let air = state.registry.states.air;
    let stone = state.registry.states.stone;
    let water = state.registry.states.water_levels[0];
    // A dry support under a one-deep pond whose surface is the floor row.
    state.world.set_block(at[0], at[1] - 1, at[2], stone);
    state.world.set_block(at[0], at[1], at[2], water);
    // Leave the rest of the footprint dry so only this column is liquid.
    for x in origin[0]..origin[0] + i32::from(size[0]) {
        for z in origin[2]..origin[2] + i32::from(size[2]) {
            if [x, z] != unfloored {
                state.world.set_block(x, 200, z, stone);
                state.world.set_block(x, 201, z, air);
            }
        }
    }
    // A solid one-cell ring up to the fluid's own top.
    let max: [i32; 3] = std::array::from_fn(|axis| origin[axis] + i32::from(size[axis]) - 1);
    for x in origin[0] - 1..=max[0] + 1 {
        for z in origin[2] - 1..=max[2] + 1 {
            let ring =
                x == origin[0] - 1 || x == max[0] + 1 || z == origin[2] - 1 || z == max[2] + 1;
            if ring {
                state.world.set_block(x, 200, z, stone);
                state.world.set_block(x, 201, z, stone);
            }
        }
    }
    state.world.fold_overlay();
    let cost = check_lot(&state, id, origin, size, &floors).expect("fluid on the floor row");
    assert!(cost.liquid >= 1, "the floor-row fluid is a liquid cell");
    assert!(cost.water >= 1);
    let project = Project {
        settlement: id,
        blueprint: "storage".into(),
        level: 1,
        origin,
        rotation: 0,
        building: None,
        structure: String::new(),
        ordered: false,
        builder: None,
        order_seq: None,
    };
    let (_, cells) =
        super::preparation::planned_cells(&state, &project, "oak", &mut Default::default())
            .expect("the lot plans");
    let fill = cells
        .iter()
        .find(|cell| cell.at == at)
        .expect("the floor-row fluid is in the plan");
    assert_eq!(fill.state, state.registry.states.dirt);
    assert_eq!(fill.resource.as_deref(), Some("minecraft:dirt"));
    assert_eq!(fill.quantity, 1, "one paid fill block");
}

fn max_of(origin: [i32; 3], size: [u8; 3]) -> [i32; 3] {
    std::array::from_fn(|axis| origin[axis] + i32::from(size[axis]) - 1)
}

/// A material-free settlement plan over `cells`, the shape a road tile takes.
fn begin_plan(
    state: &mut TickState,
    id: SettlementId,
    cells: Vec<([i32; 3], u32, Option<&str>)>,
) -> String {
    let mut min = cells[0].0;
    let mut max = cells[0].0;
    let planned = cells
        .into_iter()
        .map(|(at, plan, resource)| {
            for axis in 0..3 {
                min[axis] = min[axis].min(at[axis]);
                max[axis] = max[axis].max(at[axis]);
            }
            executor::PlannedCell {
                at,
                state: plan,
                resource: resource.map(str::to_owned),
                quantity: u32::from(resource.is_some()),
            }
        })
        .collect();
    let bounds = ScriptWorldBounds::new(min, max).expect("plan bounds");
    let project = Project {
        settlement: id,
        blueprint: layout::ROAD.into(),
        level: 1,
        origin: [min[0], min[1] - 1, min[2]],
        rotation: 0,
        building: None,
        structure: String::new(),
        ordered: false,
        builder: None,
        order_seq: None,
    };
    assert!(
        begin_cells(state, project, bounds, planned),
        "plan is admitted"
    );
    state
        .settlements
        .get(id)
        .unwrap()
        .projects
        .last()
        .unwrap()
        .structure
        .clone()
}

fn plan_stages(state: &TickState, id: SettlementId, handle: &str) -> Vec<String> {
    executor::status(state, id, handle)
        .expect("plan status")
        .stages
        .iter()
        .map(|stage| stage.stage.clone())
        .collect()
}

#[test]
fn a_source_fill_stage_runs_before_clear() {
    let (mut state, id, _slot) = fixture();
    let source = state.registry.states.water_levels[0];
    let plant = block(&state, "minecraft:short_grass", &[]);
    let fill = [3, 201, 3];
    let clear = [5, 201, 3];
    state.world.set_block(fill[0], fill[1], fill[2], source);
    state.world.set_block(clear[0], clear[1], clear[2], plant);
    state.world.fold_overlay();
    let air = state.registry.states.air;
    let dirt = state.registry.states.dirt;
    let handle = begin_plan(&mut state, id, vec![(fill, dirt, None), (clear, air, None)]);
    assert_eq!(
        plan_stages(&state, id, &handle),
        vec!["liquid".to_owned(), "clear".to_owned()],
        "a source fill runs in its own stage before the clear stage"
    );
}

#[test]
fn flowing_lava_blocks_the_write_and_wakes_once() {
    let (mut state, id, slot) = fixture();
    let at = [3, 201, 3];
    let flowing = state.registry.states.lava_levels[1];
    state.world.set_block(at[0], at[1], at[2], flowing);
    state.world.fold_overlay();
    let dirt = state.registry.states.dirt;
    begin_plan(&mut state, id, vec![(at, dirt, None)]);
    let writes = state.plugin_block_writes.len();
    super::tests::attempt(&mut state, id, slot);
    assert_eq!(
        state.plugin_block_writes.len(),
        writes,
        "a flowing lava cell is never written"
    );
    assert_eq!(state.world.block_at(at[0], at[1], at[2]), Some(flowing));
    assert!(
        state.fluid_ticks.queued_around(crate::boundary::BlockPos {
            x: at[0],
            y: at[1],
            z: at[2],
        }),
        "the waiting gate wakes the orphan flow"
    );
}

#[test]
fn water_is_written_without_a_gate() {
    let (mut state, id, slot) = fixture();
    let at = [3, 201, 3];
    let water = state.registry.states.water_levels[1];
    state.world.set_block(at[0], at[1], at[2], water);
    state.world.fold_overlay();
    let dirt = state.registry.states.dirt;
    begin_plan(&mut state, id, vec![(at, dirt, None)]);
    super::tests::attempt(&mut state, id, slot);
    assert_eq!(
        state.world.block_at(at[0], at[1], at[2]),
        Some(dirt),
        "water has no drain gate and is filled immediately"
    );
}

/// A two-cell plan whose one-cell ring is solid, so the closed bowl holds.
fn closed_two_cell_plan(state: &mut TickState, id: SettlementId) -> ([i32; 3], [i32; 3], String) {
    let a = [3, 201, 3];
    let b = [6, 201, 3];
    let water = state.registry.states.water_levels[0];
    let stone = state.registry.states.stone;
    let dirt = state.registry.states.dirt;
    state.world.set_block(b[0], b[1], b[2], water);
    for x in 2..=7 {
        for z in 2..=4 {
            if x < 3 || x > 6 || z < 3 || z > 3 {
                state.world.set_block(x, 201, z, stone);
            }
        }
    }
    state.world.fold_overlay();
    let handle = begin_plan(state, id, vec![(a, dirt, None), (b, dirt, None)]);
    (a, b, handle)
}

/// Pause the plan by editing a cell inside its fence.
fn pause_plan(state: &mut TickState, id: SettlementId, slot: EntitySlot) {
    super::tests::attempt(state, id, slot);
    let handle = state.settlements.get(id).unwrap().projects[0]
        .structure
        .clone();
    let paused = executor::status(state, id, &handle).expect("status").state
        == mc_script::ScriptStructureState::Paused;
    assert!(paused, "an edit inside the fence pauses the plan");
}

#[test]
fn recover_refuses_an_open_shell() {
    let (mut state, id, slot) = fixture();
    let (a, b, handle) = closed_two_cell_plan(&mut state, id);
    let stone = state.registry.states.stone;
    // An edit inside the fence pauses the plan; the fill cells are still ahead.
    state.world.set_block(b[0], b[1], b[2], stone);
    state.world.fold_overlay();
    pause_plan(&mut state, id, slot);
    // A new fluid cell ahead of the plan with an opened ring: no resume.
    let water = state.registry.states.water_levels[0];
    let air = state.registry.states.air;
    state.world.set_block(b[0], b[1], b[2], water);
    state.world.set_block(2, 201, 3, air);
    state.world.fold_overlay();
    assert_eq!(
        executor::recover(&mut state, id, &handle),
        Err(mc_script::ScriptOperationFailure::Blocked)
    );
    let _ = a;
}

#[test]
fn recover_refuses_a_new_preserved_supply() {
    let (mut state, id, slot) = fixture();
    let (_a, b, handle) = closed_two_cell_plan(&mut state, id);
    let stone = state.registry.states.stone;
    state.world.set_block(b[0], b[1], b[2], stone);
    state.world.fold_overlay();
    pause_plan(&mut state, id, slot);
    let fence = block(&state, "minecraft:oak_fence", &[("waterlogged", "true")]);
    state.world.set_block(b[0], b[1], b[2], fence);
    state.world.fold_overlay();
    assert_eq!(
        executor::recover(&mut state, id, &handle),
        Err(mc_script::ScriptOperationFailure::Blocked)
    );
}

#[test]
fn a_funded_fill_charges_once() {
    let (mut state, id, slot) = fixture();
    let a = [3, 201, 3];
    let b = [6, 201, 3];
    let dirt_item = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:dirt").expect("dirt id"))
        .expect("dirt item");
    let chest = state.block_entities.get_mut(&(1, 201, 1)).expect("chest");
    chest.slots[0] = mc_data::ItemStack::new(dirt_item, 2);
    let dirt = state.registry.states.dirt;
    let handle = begin_plan(
        &mut state,
        id,
        vec![
            (a, dirt, Some("minecraft:dirt")),
            (b, dirt, Some("minecraft:dirt")),
        ],
    );
    let stored = |state: &TickState| {
        state.block_entities[&(1, 201, 1)]
            .slots
            .iter()
            .filter(|stack| !stack.is_empty() && stack.item_id == dirt_item)
            .map(|stack| stack.count)
            .sum::<i32>()
    };
    for _ in 0..16 {
        super::tests::attempt(&mut state, id, slot);
    }
    assert_eq!(state.world.block_at(a[0], a[1], a[2]), Some(dirt));
    assert_eq!(state.world.block_at(b[0], b[1], b[2]), Some(dirt));
    assert_eq!(
        stored(&state),
        0,
        "exactly one storage block per paid fill cell"
    );
    assert!(
        executor::status(&state, id, &handle).is_ok(),
        "the plan is complete or still tracked without a second charge"
    );
}

#[test]
fn a_refused_resume_keeps_a_paid_cell_and_its_charge() {
    let (mut state, id, slot) = fixture();
    let a = [3, 201, 3];
    let b = [6, 201, 3];
    let dirt_item = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:dirt").expect("dirt id"))
        .expect("dirt item");
    let chest = state.block_entities.get_mut(&(1, 201, 1)).expect("chest");
    chest.slots[0] = mc_data::ItemStack::new(dirt_item, 2);
    let stone = state.registry.states.stone;
    // A flowing lava cell the gate will not write: the plan keeps a paid cell and
    // a fluid cell ahead of it at the same time.
    let flowing = state.registry.states.lava_levels[1];
    let dirt = state.registry.states.dirt;
    state.world.set_block(b[0], b[1], b[2], flowing);
    for x in 2..=7 {
        for z in 2..=4 {
            if x < 3 || x > 6 || z < 3 || z > 3 {
                state.world.set_block(x, 201, z, stone);
            }
        }
    }
    state.world.fold_overlay();
    let handle = begin_plan(
        &mut state,
        id,
        vec![
            (a, dirt, Some("minecraft:dirt")),
            (b, dirt, Some("minecraft:dirt")),
        ],
    );
    for _ in 0..8 {
        super::tests::attempt(&mut state, id, slot);
    }
    assert_eq!(state.world.block_at(a[0], a[1], a[2]), Some(dirt));
    let dirt_units = |state: &TickState| {
        let stored: i32 = state.block_entities[&(1, 201, 1)]
            .slots
            .iter()
            .filter(|stack| !stack.is_empty() && stack.item_id == dirt_item)
            .map(|stack| stack.count)
            .sum();
        let carried: i32 = state.entities.row(slot).villager.as_ref().map_or(0, |v| {
            v.inventory
                .iter()
                .flatten()
                .filter(|stack| !stack.is_empty() && stack.item_id == dirt_item)
                .map(|stack| stack.count)
                .sum()
        });
        stored + carried
    };
    assert_eq!(dirt_units(&state), 1, "one of two blocks paid for one cell");
    // A ring edit inside the widened fence pauses the plan.
    let air = state.registry.states.air;
    state.world.set_block(2, 201, 3, air);
    state.world.fold_overlay();
    super::tests::attempt(&mut state, id, slot);
    assert_eq!(
        executor::status(&state, id, &handle).expect("status").state,
        mc_script::ScriptStructureState::Paused
    );
    // The opened ring plus the fluid still ahead of the plan refuse the resume.
    assert_eq!(
        executor::recover(&mut state, id, &handle),
        Err(mc_script::ScriptOperationFailure::Blocked)
    );
    assert_eq!(
        state.world.block_at(a[0], a[1], a[2]),
        Some(dirt),
        "the placed cell survives the refusal"
    );
    assert_eq!(
        executor::status(&state, id, &handle).expect("status").state,
        mc_script::ScriptStructureState::Paused,
        "a refused resume keeps the pause"
    );
    assert_eq!(
        dirt_units(&state),
        1,
        "a refused resume charges nothing further"
    );
}

#[test]
fn removing_a_support_under_remaining_liquid_refuses_the_resume() {
    let (mut state, id, slot) = fixture();
    let (a, b, handle) = closed_two_cell_plan(&mut state, id);
    let stone = state.registry.states.stone;
    // Pause on a real edit first: the plan has fluid ahead and no paid cell yet.
    state.world.set_block(b[0], b[1], b[2], stone);
    state.world.fold_overlay();
    pause_plan(&mut state, id, slot);
    // Put the fluid back but take its ground away: the certified support is
    // inside the fence and the resume reads it.
    let water = state.registry.states.water_levels[0];
    let air = state.registry.states.air;
    state.world.set_block(b[0], b[1], b[2], water);
    state.world.set_block(b[0], b[1] - 1, b[2], air);
    state.world.fold_overlay();
    assert_eq!(
        executor::recover(&mut state, id, &handle),
        Err(mc_script::ScriptOperationFailure::Blocked),
        "a removed support under remaining liquid refuses the resume"
    );
    let _ = a;
}

#[test]
fn a_paid_cell_replaced_by_a_preserved_supply_refuses_the_resume() {
    let (mut state, id, slot) = fixture();
    let a = [3, 201, 3];
    let b = [6, 201, 3];
    let dirt_item = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:dirt").expect("dirt id"))
        .expect("dirt item");
    let chest = state.block_entities.get_mut(&(1, 201, 1)).expect("chest");
    chest.slots[0] = mc_data::ItemStack::new(dirt_item, 2);
    let stone = state.registry.states.stone;
    let flowing = state.registry.states.lava_levels[1];
    let source = state.registry.states.water_levels[0];
    let dirt = state.registry.states.dirt;
    // A holds a real source at admission, so it belongs to the certified
    // envelope; B holds the flowing lava that keeps the plan unfinished.
    state.world.set_block(a[0], a[1], a[2], source);
    state.world.set_block(b[0], b[1], b[2], flowing);
    for x in 2..=7 {
        for z in 2..=4 {
            if x < 3 || x > 6 || z < 3 || z > 3 {
                state.world.set_block(x, 201, z, stone);
            }
        }
    }
    state.world.fold_overlay();
    let handle = begin_plan(
        &mut state,
        id,
        vec![
            (a, dirt, Some("minecraft:dirt")),
            (b, dirt, Some("minecraft:dirt")),
        ],
    );
    for _ in 0..8 {
        super::tests::attempt(&mut state, id, slot);
    }
    assert_eq!(state.world.block_at(a[0], a[1], a[2]), Some(dirt));
    // The paid cell becomes a preserved supply; the ring stays closed and the
    // flowing lava ahead is untouched, so only the whole-envelope read can see
    // this refusal.
    let fence = block(&state, "minecraft:oak_fence", &[("waterlogged", "true")]);
    state.world.set_block(a[0], a[1], a[2], fence);
    state.world.fold_overlay();
    pause_plan(&mut state, id, slot);
    assert_eq!(
        state.world.block_at(b[0], b[1], b[2]),
        Some(flowing),
        "the fluid ahead is untouched"
    );
    assert_eq!(
        executor::recover(&mut state, id, &handle),
        Err(mc_script::ScriptOperationFailure::Blocked),
        "a paid cell replaced by a supply refuses the resume"
    );
    assert_eq!(
        executor::status(&state, id, &handle).expect("status").state,
        mc_script::ScriptStructureState::Paused
    );
}

#[test]
fn a_wait_is_named_for_a_clear_cell_beside_lava() {
    let (mut state, id, slot) = fixture();
    let at = [3, 201, 3];
    let lava = state.registry.states.lava_levels[0];
    let stone = state.registry.states.stone;
    state.world.set_block(at[0] - 1, at[1], at[2], lava);
    state.world.set_block(at[0], at[1], at[2], stone);
    state.world.fold_overlay();
    let air = state.registry.states.air;
    let handle = begin_plan(&mut state, id, vec![(at, air, None)]);
    let project = state.settlements.get(id).unwrap().projects[0].clone();
    assert_eq!(
        crate::villager::building::wait_reason(&state, slot, &project, at),
        "lava_neighbour",
        "clearing beside lava names the neighbour wait"
    );
    let _ = handle;
}

#[test]
fn a_repeated_wait_schedules_once() {
    let (mut state, id, slot) = fixture();
    let at = [3, 201, 3];
    let flowing = state.registry.states.lava_levels[1];
    state.world.set_block(at[0], at[1], at[2], flowing);
    state.world.fold_overlay();
    let dirt = state.registry.states.dirt;
    begin_plan(&mut state, id, vec![(at, dirt, None)]);
    let image = |state: &mut TickState| {
        let mut entries = Vec::new();
        state.fluid_ticks.journal(&mut entries);
        entries.len()
    };
    super::tests::attempt(&mut state, id, slot);
    assert_eq!(image(&mut state), 1, "the first wait schedules one tick");
    state.tick += 7;
    super::tests::attempt(&mut state, id, slot);
    assert_eq!(image(&mut state), 0, "a repeated wait schedules nothing");
    state.tick += 7;
    super::tests::attempt(&mut state, id, slot);
    assert_eq!(
        image(&mut state),
        0,
        "a third wait on a later tick wakes nothing"
    );
}
