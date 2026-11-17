use super::super::tests::{attempt, complete, fund};
use super::*;
use crate::settlements::allocator::tests::block;
use crate::settlements::forestry;

fn stock(state: &TickState) -> std::collections::BTreeMap<u32, i32> {
    let mut counts = std::collections::BTreeMap::new();
    for stack in &state.block_entities[&(1, 201, 1)].slots {
        if !stack.is_empty() {
            *counts.entry(stack.item_id).or_default() += stack.count;
        }
    }
    counts
}

#[test]
fn on_lot_sapling_is_first_and_regrowth_requeues_an_unpaid_manifest() {
    let (mut state, id, builder, expected, _, _) = forestry::tests::forest();
    forestry::tests::grown_world(&mut state, Vec::new(), "birch");
    let size = blueprints::get(&state.registry, &expected.blueprint)
        .unwrap()
        .footprint(Rotation::from_quarter_turns(expected.rotation));
    let access = layout::Network::for_construction(&state, state.settlements.get(id).unwrap())
        .construction_access(&expected)
        .unwrap();
    let distance = |at: [i32; 3]| {
        access
            .iter()
            .map(|cell| at[0].abs_diff(cell[0]).max(at[2].abs_diff(cell[2])))
            .min()
            .unwrap()
    };
    let root = (expected.origin[0]..expected.origin[0] + i32::from(size[0]))
        .flat_map(|x| {
            (expected.origin[2]..expected.origin[2] + i32::from(size[2])).map(move |z| [x, 201, z])
        })
        .max_by_key(|&at| distance(at))
        .unwrap();
    assert!(distance(root) > forestry::PLANTED_CROWN_RADIUS);
    let sapling = block(&state, "minecraft:birch_sapling", &[("stage", "1")]);
    let dirt = state.registry.states.dirt;
    state.world.set_block(root[0], root[1] - 1, root[2], dirt);
    state.world.set_block(root[0], root[1], root[2], sapling);
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .planted_trees
        .insert(root);
    let plant = block(&state, "minecraft:short_grass", &[]);
    let other = [expected.origin[0], root[1], expected.origin[2]];
    state.world.set_block(other[0], other[1], other[2], plant);
    fund(&mut state, "storage", 1);
    assert!(begin(&mut state, expected.clone(), "oak", 0));
    let project = state.settlements.get(id).unwrap().project.clone().unwrap();
    assert_eq!(project.origin, expected.origin);
    assert_eq!(
        executor::next_cell(&state, id, &project.structure).unwrap(),
        Some(root)
    );
    let before = stock(&state);
    let leaves = forestry::native_tree_tests::grow(&mut state, root, 5);
    let log = block(&state, "minecraft:birch_log", &[]);
    let grown: Vec<_> = leaves
        .iter()
        .copied()
        .chain((0..5).map(|y| [root[0], root[1] + y, root[2]]))
        .map(|at| (at, state.world.block_at(at[0], at[1], at[2]).unwrap()))
        .collect();
    for _ in 0..16 {
        if state.settlements.get(id).unwrap().project.is_none() {
            break;
        }
        attempt(&mut state, id, builder);
    }
    assert!(
        state.settlements.get(id).unwrap().project.is_none(),
        "changed unpaid preparation must cancel, not resume its stale work cells"
    );
    let cancelled = executor::status(&state, id, &project.structure).unwrap();
    assert_eq!(cancelled.state, mc_script::ScriptStructureState::Cancelled);
    assert!(cancelled.consumed.is_empty());
    assert_eq!(stock(&state), before);
    assert_eq!(
        state
            .settlements
            .get(id)
            .unwrap()
            .pending_order
            .as_ref()
            .unwrap()
            .blueprint,
        "storage"
    );
    for &(at, block) in &grown {
        assert_eq!(
            state.world.block_at(at[0], at[1], at[2]),
            Some(block),
            "regrowth stays intact until recertification: {at:?}"
        );
    }
    assert!(
        begin(&mut state, expected.clone(), "oak", 0),
        "the grown native tree can be recertified"
    );
    let renewed = state.settlements.get(id).unwrap().project.clone().unwrap();
    assert_eq!(renewed.origin, expected.origin);
    let (_, cells) = planned_cells(&state, &renewed, "oak", &mut Default::default()).unwrap();
    for &(at, _) in &grown {
        assert!(
            cells
                .iter()
                .any(|cell| cell.at == at && cell.state == state.registry.states.air),
            "whole regrown tree is funded preparation: {at:?}"
        );
    }
    let mut logs = 0;
    for _ in 0..4096 {
        if executor::next_material(&state, id, &renewed.structure)
            .unwrap()
            .is_some()
        {
            break;
        }
        let at = executor::next_cell(&state, id, &renewed.structure)
            .unwrap()
            .unwrap();
        let block = state.world.block_at(at[0], at[1], at[2]).unwrap();
        let mut expected_stock = stock(&state);
        if forestry::tree(&state, block) {
            state.tick += 20;
            for drop in forestry::roll(&state, at, block).unwrap() {
                *expected_stock.entry(drop.item_id).or_default() += drop.count;
            }
            state.tick -= 20;
        }
        if block == log {
            logs += 1;
        }
        attempt(&mut state, id, builder);
        assert_eq!(
            stock(&state),
            expected_stock,
            "every recertified tree drop at {at:?}"
        );
    }
    assert_eq!(logs, 5);
    assert!(
        !state
            .settlements
            .get(id)
            .unwrap()
            .planted_trees
            .contains(&root)
    );
    complete(&mut state, id, builder);
    assert!(state.settlements.get(id).unwrap().project.is_none());
}

#[test]
fn registered_sapling_precedes_a_natural_tree_in_the_clear_manifest() {
    let (mut state, id, _, expected, _, _) = forestry::tests::forest();
    let size = blueprints::get(&state.registry, &expected.blueprint)
        .unwrap()
        .footprint(Rotation::from_quarter_turns(expected.rotation));
    let natural = [expected.origin[0], 201, expected.origin[2]];
    let sapling_at = [
        expected.origin[0] + i32::from(size[0]) - 1,
        201,
        expected.origin[2] + i32::from(size[2]) - 1,
    ];
    forestry::tests::grown_world(&mut state, vec![natural], "birch");
    let sapling = block(&state, "minecraft:birch_sapling", &[("stage", "1")]);
    state
        .world
        .set_block(sapling_at[0], sapling_at[1], sapling_at[2], sapling);
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .planted_trees
        .insert(sapling_at);
    let project = expected.clone();
    let preparation =
        forestry::cells(&state, &project, size, &mut forestry::Clearance::default()).unwrap();
    assert!(
        preparation.cells.iter().any(|cell| cell.at == natural),
        "the natural tree is clearable preparation"
    );
    assert_eq!(
        preparation.cells.first().map(|cell| cell.at),
        Some(sapling_at),
        "a registered sapling is certified before the natural tree it will not outgrow"
    );
}

#[test]
fn one_owner_scan_shares_a_registered_root_proof_across_candidate_lots() {
    let (mut state, id, _, _, root, _) = forestry::tests::forest();
    forestry::tests::grown_world(&mut state, vec![root], "birch");
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .planted_trees
        .insert(root);
    let record = state.settlements.get(id).unwrap();
    let blueprint = blueprints::get(&state.registry, "storage").unwrap();
    let before = forestry::proof_builds();
    // The same funded kind twice forces two full passes over the same
    // candidates: one scan-wide Clearance must prove the root only once.
    let (site, _, report) = site_project(
        &state,
        record,
        [blueprint, blueprint].into_iter(),
        0,
        u32::MAX,
        &[],
    );
    assert!(
        site.is_none(),
        "empty funding images force the whole owner scan"
    );
    assert!(report.examined > 1);
    assert_eq!(forestry::proof_builds() - before, 1);
}
