//! The autonomous perimeter wall, its gatehouses and the well
//! (docs/settlements/construction.md#perimeter-gates-and-well).

use super::*;
use crate::settlements::allocator::tests::villager;
use crate::settlements::construction::tests::{complete, fixture};
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
