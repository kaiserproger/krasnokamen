//! The autonomous perimeter wall, its gatehouses and the well
//! (docs/settlements/construction.md#perimeter-gates-and-well).

use super::*;
use crate::settlements::allocator::tests::villager;
use crate::settlements::construction::tests::{complete, fixture};
use crate::settlements::buildings::BuildingKind;
use crate::settlements::layout::{GATE, PERIMETER, RING, WELL};
use crate::villager::Activity;
use mc_data::ItemStack;
use std::collections::{HashSet, VecDeque};

const ANCHOR: [i32; 3] = [8, 201, 8];
const GROUND: i32 = 200;

fn item(state: &TickState, name: &str) -> u32 {
    state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse(format!("minecraft:{name}")).unwrap())
        .unwrap()
}

fn block_name(state: &TickState, x: i32, y: i32, z: i32) -> String {
    let id = state.world.physics_block_at(x, y, z).unwrap();
    state
        .registry
        .blocks
        .by_id(mc_world::BlockStateId(id))
        .unwrap()
        .block
        .id
        .path()
        .to_owned()
}

/// A flat plain at `y = 200` wide enough for the whole wall line around the
/// fixture's bell and the road stretches that cross it, ten residents, and a
/// chest stocking the whole fortification: cobblestone for every column and
/// one oak door per gatehouse.
fn town(residents: usize, stocked: bool) -> (TickState, SettlementId, crate::entities::EntitySlot) {
    let (mut state, id, builder) = fixture();
    state
        .world
        .ensure_blocking(&rustc_hash::FxHashMap::from_iter((-3..=3).flat_map(|x| {
            (-3..=3).map(move |z| (crate::boundary::ChunkPos { x, z }, 0))
        })));
    let grass = crate::settlements::allocator::tests::block(&state, "minecraft:grass_block", &[]);
    for x in -40..=56 {
        for z in -40..=56 {
            state.world.set_block(x, GROUND, z, grass);
            for y in 201..224 {
                if (x, y, z) != (1, 201, 1) {
                    state.world.set_block(x, y, z, state.registry.states.air);
                }
            }
        }
    }
    state.world.fold_overlay();
    for n in 1..residents {
        let slot = villager(&mut state, [4.5 + n as f64 * 0.5, 201.0, 4.5]);
        let entity = state.entities.body(slot).id;
        crate::settlements::bind(&mut state, id, entity).unwrap();
    }
    if stocked {
        let (cobble, door) = (item(&state, "cobblestone"), item(&state, "oak_door"));
        let chest = state.block_entities.get_mut(&(1, 201, 1)).unwrap();
        for slot in 0..14 {
            chest.slots[slot] = ItemStack::new(cobble, 64);
        }
        chest.slots[14] = ItemStack::new(door, 4);
    }
    (state, id, builder)
}

/// SETTLEMENT-WALL-STRETCH-WITHOUT-STAND: a stretch whose still-owed columns
/// have no builder stand beside them is not planned at all, instead of being
/// admitted and cancelled again on every rotation.
#[test]
fn a_wall_stretch_without_a_builder_stand_is_not_planned() {
    let (mut state, id, _) = town(10, true);
    let water = crate::settlements::allocator::tests::block(&state, "minecraft:water", &[]);
    let span = crate::settlements::layout::perimeter_spans(ANCHOR)
        .into_iter()
        .find(|span| span.kind == PERIMETER)
        .expect("the plan owes wall stretches");
    let x_fixed = span.min[0] == span.max[0];
    let side = if x_fixed {
        (span.min[0] - ANCHOR[0]).signum()
    } else {
        (span.min[1] - ANCHOR[2]).signum()
    };
    // Both inward columns of every column of the stretch become open water, so
    // the builder has nowhere to stand for this piece of the wall.
    for x in span.min[0]..=span.max[0] {
        for z in span.min[1]..=span.max[1] {
            for step in 1..=2 {
                let (wx, wz) = if x_fixed {
                    (x - side * step, z)
                } else {
                    (x, z - side * step)
                };
                state.world.set_block(wx, GROUND, wz, water);
            }
        }
    }
    state.world.fold_overlay();
    let record = state.settlements.get(id).unwrap();
    let planned: Vec<_> = crate::settlements::layout::fortification(&state, record)
        .map(|(kind, work)| (kind, work.bounds.min, work.bounds.max))
        .collect();
    let overlaps = |min: [i32; 3], max: [i32; 3]| {
        min[0] <= span.max[0]
            && max[0] >= span.min[0]
            && min[2] <= span.max[1]
            && max[2] >= span.min[1]
    };
    assert!(
        !planned.iter().any(|&(_, min, max)| overlaps(min, max)),
        "the standless stretch is not admitted: {planned:?}"
    );
    assert!(
        planned.iter().any(|&(kind, _, _)| kind == PERIMETER),
        "the other wall stretches remain planned"
    );
}

/// A finished street or fortification project releases its Builder; the
/// planning tick hires one again before the next project, as `tick` does.
fn finish(state: &mut TickState, id: SettlementId, builder: crate::entities::EntitySlot) {
    let v = state.entities.row_mut(builder).villager.as_mut().unwrap();
    v.data.profession = Profession::Builder;
    v.activity = Activity::Work;
    complete(state, id, builder);
}

/// Plan and finish projects until the planner starts none; the blueprint name
/// of each, in order.
fn build_everything(
    state: &mut TickState,
    id: SettlementId,
    builder: crate::entities::EntitySlot,
) -> Vec<String> {
    let mut order = Vec::new();
    for _ in 0..40 {
        plan(state, id);
        let Some(project) = state.settlements.get(id).unwrap().projects.first().cloned() else {
            break;
        };
        order.push(project.blueprint);
        finish(state, id, builder);
    }
    order
}

/// Every wall and gate column of the ring, as `(x, z, across)` with `across`
/// the signed offset along the side from the road through the bell.
fn ring_columns() -> Vec<(i32, i32, i32)> {
    let mut columns = Vec::new();
    for x in ANCHOR[0] - RING..=ANCHOR[0] + RING {
        for z in ANCHOR[2] - RING..=ANCHOR[2] + RING {
            let (dx, dz) = (x - ANCHOR[0], z - ANCHOR[2]);
            if dx.abs().max(dz.abs()) == RING {
                columns.push((x, z, if dx.abs() == RING { dz } else { dx }));
            }
        }
    }
    columns
}

#[test]
fn ten_residents_wall_the_town_in_with_four_gates_and_one_well() {
    let (mut state, id, builder) = town(10, true);
    let order = build_everything(&mut state, id, builder);
    assert_eq!(order[0], WELL, "the well comes first: {order:?}");
    assert_eq!(order.iter().filter(|kind| *kind == WELL).count(), 1);
    assert_eq!(order.iter().filter(|kind| *kind == GATE).count(), 4);
    assert_eq!(order.iter().filter(|kind| *kind == PERIMETER).count(), 24);
    assert!(
        state.settlements.get(id).unwrap().projects.is_empty(),
        "nothing owed once the wall and well stand"
    );
    // Wall columns are three cobblestone blocks on the ground, gatehouse
    // columns four, with one door in the road's centre column.
    let mut doors = 0;
    for (x, z, across) in ring_columns() {
        let height = if across.abs() <= 3 { 4 } else { 3 };
        for step in 1..=height {
            let name = block_name(&state, x, GROUND + step, z);
            if across == 0 && step <= 2 {
                assert_eq!(name, "oak_door", "{x} {z} {step}");
                doors += usize::from(step == 1);
            } else {
                assert_eq!(name, "cobblestone", "{x} {z} {step}");
            }
        }
        assert_eq!(block_name(&state, x, GROUND + height + 1, z), "air");
    }
    assert_eq!(doors, 4);
    // A well on the north-west plaza corner: a water source ringed by eight
    // cobblestone blocks.
    assert_eq!(block_name(&state, 5, GROUND, 5), "water");
    for (x, z) in [
        (4, 4),
        (5, 4),
        (6, 4),
        (4, 5),
        (6, 5),
        (4, 6),
        (5, 6),
        (6, 6),
    ] {
        assert_eq!(block_name(&state, x, GROUND + 1, z), "cobblestone");
    }
    // Enclosure, proven by walking: from the bell no open column at foot
    // height reaches a column outside the wall line.
    let mut reached = HashSet::from([(ANCHOR[0], ANCHOR[2])]);
    let mut queue = VecDeque::from([(ANCHOR[0], ANCHOR[2])]);
    while let Some((x, z)) = queue.pop_front() {
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let next = (x + dx, z + dz);
            if reached.contains(&next) || block_name(&state, next.0, GROUND + 1, next.1) != "air" {
                continue;
            }
            reached.insert(next);
            queue.push_back(next);
        }
    }
    assert!(reached.len() > 2000, "the town interior is open");
    assert!(
        reached
            .iter()
            .all(|&(x, z)| (x - ANCHOR[0]).abs().max((z - ANCHOR[2]).abs()) < RING),
        "a walker reaches the outside through the wall"
    );
    // Planned once: a second pass owes nothing.
    plan(&mut state, id);
    assert!(state.settlements.get(id).unwrap().projects.is_empty());
}

/// The well's water source costs nothing, and the structure ledger that is
/// saved and replayed must accept that one free cell (and the gatehouse's free
/// door half) instead of refusing the whole ledger as malformed.
#[test]
fn the_free_water_source_and_door_halves_pass_ledger_validation() {
    let (mut state, id, builder) = town(10, true);
    for expected in [WELL, GATE] {
        plan(&mut state, id);
        let project = state.settlements.get(id).unwrap().projects[0].clone();
        assert_eq!(project.blueprint, expected);
        crate::plugin_structures::validate(&state).expect("ledger accepts the free cells");
        finish(&mut state, id, builder);
    }
}

#[test]
fn fewer_than_ten_residents_or_no_stock_start_no_fortification() {
    let (mut state, id, _) = town(9, true);
    plan(&mut state, id);
    assert!(state.settlements.get(id).unwrap().projects.is_empty());
    let (mut state, id, _) = town(10, false);
    plan(&mut state, id);
    assert!(state.settlements.get(id).unwrap().projects.is_empty());
}

/// Street paving crosses the wall line on all four axes. Paved first, the road
/// must not keep the gatehouse from standing on it.
#[test]
fn a_gatehouse_stands_on_a_road_that_was_paved_first() {
    let (mut state, id, builder) = town(10, true);
    for (x, z) in [(38, 8), (-22, 8), (8, 38), (8, -22)] {
        let tile = layout::tile(ANCHOR, x, z).unwrap();
        let record = state.settlements.get(id).unwrap();
        let work = layout::Network::walk(&state, record)
            .road_work(&tile)
            .expect("the road is unpaved");
        let project = Project {
            settlement: id,
            blueprint: layout::ROAD.into(),
            level: 1,
            origin: work.bounds.min,
            rotation: tile.axis,
            building: None,
            structure: String::new(),
            ordered: false,
            builder: None,
            order_seq: None,
        };
        assert!(begin_cells(&mut state, project, work.bounds, work.cells));
        finish(&mut state, id, builder);
    }
    let order = build_everything(&mut state, id, builder);
    assert_eq!(
        order.iter().filter(|kind| *kind == GATE).count(),
        4,
        "{order:?}"
    );
    for (x, z) in [(38, 8), (-22, 8), (8, 38), (8, -22)] {
        assert_eq!(block_name(&state, x, GROUND + 1, z), "oak_door", "{x} {z}");
    }
}

#[test]
fn a_column_over_water_stays_a_gap_and_the_rest_of_the_wall_stands() {
    let (mut state, id, builder) = town(10, true);
    let water = crate::settlements::allocator::tests::block(&state, "minecraft:water", &[]);
    state.world.set_block(38, GROUND, 20, water);
    state.world.fold_overlay();
    let order = build_everything(&mut state, id, builder);
    assert_eq!(order.iter().filter(|kind| *kind == PERIMETER).count(), 24);
    assert_eq!(block_name(&state, 38, GROUND + 1, 20), "air");
    assert_eq!(block_name(&state, 38, GROUND + 1, 19), "cobblestone");
    assert_eq!(block_name(&state, 38, GROUND + 1, 21), "cobblestone");
}

/// The 2026-11-24 perimeter real-client gate (`20261124T125551`) built the well and the streets
/// but never started a wall or gate column in 1580 s with cobblestone and four doors stocked.
/// Drive the same fixture through the real tick and require a fortification project to be
/// admitted: a planner-only test (`build_everything`) cannot see an execution stall.
#[test]
fn a_funded_fortification_starts_under_the_real_tick() {
    let (mut state, id, _builder) = town(10, true);
    let mut history: Vec<(u64, Vec<String>, usize)> = Vec::new();
    let mut started = None;
    for _ in 0..6000 {
        crate::tick::run_tick(&mut state, &[]);
        let record = state.settlements.get(id).unwrap();
        let projects: Vec<String> = record
            .projects
            .iter()
            .map(|project| format!("{} L{}", project.blueprint, project.level))
            .collect();
        if history.last().map(|(_, p, _)| p) != Some(&projects) {
            history.push((state.tick, projects.clone(), record.orders.len()));
        }
        if projects
            .iter()
            .any(|project| project.starts_with(PERIMETER) || project.starts_with(GATE))
        {
            started = Some(state.tick);
            break;
        }
    }
    let stock = state
        .block_entities
        .get(&(1, 201, 1))
        .map(|chest| {
            chest
                .slots
                .iter()
                .filter(|stack| !stack.is_empty())
                .map(|stack| (stack.item_id, stack.count))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    assert!(
        started.is_some(),
        "no wall or gate project in 6000 ticks; history={history:?}; stock={stock:?}; \
         builder={:?}",
        state
            .settlements
            .get(id)
            .unwrap()
            .projects
            .iter()
            .map(|project| (project.blueprint.clone(), project.builder))
            .collect::<Vec<_>>()
    );
}

/// Gate-like reproduction (`20261124T125551`): no native Builder (the gate summons plain
/// residents), stock cobblestone, four doors and wheat seeds, no logs. The gate built the well and
/// three road blocks, then stopped for 1580 s: plain residents (no native Builder) still start
/// the funded list under the real tick.
#[test]
fn plain_residents_start_fortification_under_the_real_tick() {
    let (mut state, id, builder) = town(10, true);
    {
        let v = state.entities.row_mut(builder).villager.as_mut().unwrap();
        v.data.profession = Profession::None;
        v.activity = Activity::Idle;
    }
    let seeds = item(&state, "wheat_seeds");
    state
        .block_entities
        .get_mut(&(1, 201, 1))
        .unwrap()
        .slots[15] = ItemStack::new(seeds, 64);
    let mut started = None;
    for _ in 0..6000 {
        crate::tick::run_tick(&mut state, &[]);
        let record = state.settlements.get(id).unwrap();
        if record.projects.iter().any(|project| {
            project.blueprint.starts_with(PERIMETER)
                || project.blueprint.starts_with(GATE)
                || project.blueprint == WELL
        }) {
            started = Some(state.tick);
            break;
        }
    }
    assert!(
        started.is_some(),
        "no fortification project in 6000 ticks; projects={:?}",
        state
            .settlements
            .get(id)
            .unwrap()
            .projects
            .iter()
            .map(|project| project.blueprint.clone())
            .collect::<Vec<_>>()
    );
}

/// A settlement building whose job site is a natural zone: a certified Mine or LumberCamp
/// (terrain, no placed blocks), as the real world's gate fixture gets for free.
fn certify_natural(state: &mut TickState, id: SettlementId, kind: BuildingKind) {
    use crate::settlements::buildings::{Building, BuildingId};
    let record = state.settlements.get_mut(id).unwrap();
    let bid = BuildingId((id.0 << 32) | (record.building_sequence + 1));
    record.building_sequence += 1;
    record.buildings.insert(
        bid,
        Building {
            id: bid,
            settlement: id,
            kind,
            level: 1,
            bounds: mc_script::ScriptWorldBounds::new([1, 200, 1], [5, 204, 5]).unwrap(),
            work_points: vec![[3, 201, 3]],
            revision: 1,
            natural_zone: true,
        },
    );
}

/// 2026-11-24 perimeter gate: a staffed gatherer of the demanded profession (a certified natural
/// zone plus a resident working its job site) made `plan_pass` return before the fortification,
/// so 1580 s passed with the well standing and 768 cobblestone in storage. The gatherer keeps out
/// of the optional street work only; the funded fortification list is not held.
#[test]
fn a_demanded_staffed_lumberjack_does_not_hold_the_fortification() {
    let (mut state, id, builder) = town(10, true);
    certify_natural(&mut state, id, BuildingKind::LumberCamp);
    {
        let v = state.entities.row_mut(builder).villager.as_mut().unwrap();
        v.data.profession = Profession::Lumberjack;
        v.job_site = Some(crate::boundary::BlockPos { x: 3, y: 201, z: 3 });
        v.activity = Activity::Work;
    }
    let images = executor::source_images(&state, id);
    let demand = {
        let record = state.settlements.get(id).unwrap();
        gatherer_demand(&state, record, &images)
    };
    assert_eq!(
        demand,
        Some(Profession::Lumberjack),
        "the unfunded House bill demands logs, and the camp staffs the Lumberjack"
    );
    assert!(
        has_staffed_gatherer(&state, id, Profession::Lumberjack),
        "the camp's Lumberjack holds the 3 201 3 job site"
    );
    plan(&mut state, id);
    let started: Vec<String> = state
        .settlements
        .get(id)
        .unwrap()
        .projects
        .iter()
        .map(|project| project.blueprint.clone())
        .collect();
    assert!(
        started.iter().any(|blueprint| blueprint == WELL
            || blueprint.starts_with(PERIMETER)
            || blueprint.starts_with(GATE)),
        "a staffed gatherer holds the streets, not the funded fortification: {started:?}"
    );
}
