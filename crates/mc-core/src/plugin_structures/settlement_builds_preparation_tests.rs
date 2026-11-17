use super::*;
use crate::settlements::allocator::tests::block;

fn counts(state: &TickState) -> BTreeMap<u32, i32> {
    let mut counts = BTreeMap::new();
    for stack in &state.block_entities[&(1, 201, 1)].slots {
        if !stack.is_empty() {
            *counts.entry(stack.item_id).or_default() += stack.count;
        }
    }
    counts
}

#[test]
fn an_upgrade_clear_cell_credits_its_drops_exactly_once() {
    let (mut state, id, builder, project, root, _) = crate::settlements::forestry::tests::forest();
    let mut project = project;
    // The same canonical debit must credit clear-cell drops for an upgrade,
    // not only for a new site's preparation.
    project.building = Some(crate::settlements::buildings::BuildingId(1));
    state.settlements.get_mut(id).unwrap().project = Some(project);
    let builder = state.entities.row(builder).uuid.to_string();
    let before = counts(&state);
    let cells = [super::super::Cell {
        at: root,
        state: state.registry.states.air,
        resource: None,
        quantity: 0,
    }];
    let block = state.world.block_at(root[0], root[1], root[2]).unwrap();
    let mut expected = before.clone();
    for drop in crate::settlements::forestry::roll(&state, root, block).unwrap() {
        *expected.entry(drop.item_id).or_default() += drop.count;
    }
    assert_eq!(
        debit(&mut state, id, &builder, &BTreeMap::new(), &cells),
        Ok(())
    );
    assert_eq!(counts(&state), expected, "upgrade clear drops once");
    // The root retires with an empty trunk column on the ordinary retire pass,
    // not cell by cell: the rest of its own tree still needs the column as
    // identity evidence.
    state.tick = 20;
    crate::settlements::forestry::retire_missing(&mut state);
    assert!(
        !state
            .settlements
            .get(id)
            .unwrap()
            .planted_trees
            .contains(&root)
    );
}

#[test]
fn non_tree_log_variants_cannot_pass_uncredited_through_preparation_debit() {
    let (mut state, id, builder, _, root, _) = crate::settlements::forestry::tests::forest();
    crate::settlements::construction::tests::fund(&mut state, "storage", 1);
    crate::settlements::construction::request_build(&mut state, id, "storage").unwrap();
    let builder = state.entities.row(builder).uuid.to_string();
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    let cells = [super::super::Cell {
        at: root,
        state: state.registry.states.air,
        resource: None,
        quantity: 0,
    }];
    for name in [
        "minecraft:stripped_birch_log",
        "minecraft:oak_wood",
        "minecraft:crimson_hyphae",
    ] {
        let value = block(&state, name, &[]);
        state.world.set_block(root[0], root[1], root[2], value);
        assert_eq!(
            debit(&mut state, id, &builder, &BTreeMap::new(), &cells),
            Err(Failure::Blocked),
            "{name}"
        );
        assert_eq!(state.world.block_at(root[0], root[1], root[2]), Some(value));
        assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    }
}

/// N5 end to end: a lot covered by two touching registered trees is admitted
/// as one grove, every tree of it is felled, and the storage credit is exactly
/// the vanilla loot of the felled cells.
#[test]
fn touching_grove_is_felled_whole_with_exact_drops() {
    use crate::settlements::forestry::tests::{forest, grown_world};
    let (mut state, id, builder, project, _, _) = forest();
    let builder = state.entities.row(builder).uuid.to_string();
    grown_world(&mut state, Vec::new(), "birch");
    let blueprint = crate::settlements::blueprints::get(&state.registry, &project.blueprint)
        .expect("blueprint");
    let size = blueprint.footprint(crate::settlements::blueprints::Rotation::from_quarter_turns(
        project.rotation,
    ));
    let bounds = crate::settlements::forestry::lot_bounds(&state, &project).expect("lot");
    let z = bounds.min[2] + 2;
    let left = [bounds.max[0] - 3, 201, z];
    let right = [bounds.max[0] + 3, 201, z];
    let want: rustc_hash::FxHashMap<_, _> =
        ((bounds.min[0] - 8).div_euclid(16)..=(bounds.max[0] + 8).div_euclid(16))
            .flat_map(|x| {
                ((bounds.min[2] - 8).div_euclid(16)..=(bounds.max[2] + 8).div_euclid(16))
                    .map(move |z| (crate::boundary::ChunkPos { x, z }, 0))
            })
            .collect();
    state.world.ensure_blocking(&want);
    state.world.fold_overlay();
    crate::settlements::forestry::native_tree_tests::grow(&mut state, left, 5);
    crate::settlements::forestry::native_tree_tests::grow(&mut state, right, 5);
    let record = state.settlements.get_mut(id).expect("settlement");
    record.planted_trees.insert(left);
    record.planted_trees.insert(right);
    assert!(
        !(bounds.min[0]..=bounds.max[0]).contains(&right[0]),
        "the second trunk stands outside the lot"
    );
    let mut clearance = crate::settlements::forestry::Clearance::default();
    let preparation =
        crate::settlements::forestry::cells(&state, &project, size, &mut clearance)
            .expect("a touching grove does not refuse the lot");
    let mut manifested: Vec<_> = preparation.cells.iter().map(|cell| cell.at).collect();
    manifested.sort_unstable();
    manifested.dedup();
    assert_eq!(manifested.len(), preparation.cells.len(), "no cell twice");
    assert!(manifested.contains(&left), "the on-lot trunk is felled");
    assert!(
        manifested.contains(&right),
        "the touching off-lot trunk is felled whole"
    );
    let mut expected = counts(&state);
    for cell in &preparation.cells {
        let block = state
            .world
            .block_at(cell.at[0], cell.at[1], cell.at[2])
            .expect("loaded cell");
        assert!(
            crate::settlements::forestry::tree_or_sapling(&state, block),
            "manifest holds only tree cells"
        );
        for drop in crate::settlements::forestry::roll(&state, cell.at, block).expect("drops") {
            *expected.entry(drop.item_id).or_default() += drop.count;
        }
    }
    state.settlements.get_mut(id).expect("settlement").project = Some(project);
    let cells: Vec<_> = preparation
        .cells
        .iter()
        .map(|cell| super::super::Cell {
            at: cell.at,
            state: cell.state,
            resource: cell.resource.clone(),
            quantity: cell.quantity,
        })
        .collect();
    let outcome = debit(&mut state, id, &builder, &BTreeMap::new(), &cells);
    assert_eq!(outcome, Ok(()), "the grove is cleared");
    assert_eq!(counts(&state), expected, "exact vanilla drops into storage");
    assert!(!state.world.block_at(left[0], left[1], left[2]).is_some_and(|b| {
        crate::settlements::forestry::tree_or_sapling(&state, b)
    }));
}

/// N6: with block history disabled the executor still refuses a tree block a
/// player put into an admitted manifest cell after admission.
#[test]
fn late_player_log_in_an_admitted_manifest_is_not_felled() {
    use crate::settlements::forestry::tests::forest;
    let (mut state, id, builder, project, root, _) = forest();
    let builder = state.entities.row(builder).uuid.to_string();
    let blueprint = crate::settlements::blueprints::get(&state.registry, &project.blueprint)
        .expect("blueprint");
    let size = blueprint.footprint(crate::settlements::blueprints::Rotation::from_quarter_turns(
        project.rotation,
    ));
    let mut clearance = crate::settlements::forestry::Clearance::default();
    let preparation =
        crate::settlements::forestry::cells(&state, &project, size, &mut clearance)
            .expect("the natural lot is admitted");
    assert!(!state.block_log.enabled(), "capture disabled: no attribution");
    let at = preparation
        .cells
        .iter()
        .map(|cell| cell.at)
        .find(|at| (at[0] - root[0]).abs() == 2 || (at[2] - root[2]).abs() == 2)
        .expect("a crown edge leaf is in the manifest");
    let leaves = state.world.block_at(at[0], at[1], at[2]).expect("loaded");
    assert!(
        state
            .registry
            .blocks
            .by_id(mc_world::BlockStateId(leaves))
            .is_some_and(|block| block.block.id.path().ends_with("_leaves"))
    );
    // The player breaks that leaf and puts a log in the admitted cell.
    let log = block(&state, "minecraft:oak_log", &[]);
    state.world.set_block(at[0], at[1], at[2], log);
    state.settlements.get_mut(id).expect("settlement").project = Some(project);
    let cells = [super::super::Cell {
        at,
        state: state.registry.states.air,
        resource: None,
        quantity: 0,
    }];
    assert_eq!(
        debit(&mut state, id, &builder, &BTreeMap::new(), &cells),
        Err(Failure::Blocked),
        "a late player block is not felled"
    );
    assert_eq!(state.world.block_at(at[0], at[1], at[2]), Some(log));
}

/// N6 boundary: the canonical executor advances one cell per debit, and a
/// registered tree's crown sits above a trunk whose cells are cleared first.
/// Every remaining cell must still prove its identity, so a registered tree is
/// felled to its last leaf and only then retires.
#[test]
fn planted_tree_is_felled_cell_by_cell_to_the_end() {
    use crate::settlements::forestry::tests::{forest, grown_world};
    let (mut state, id, builder, project, _, _) = forest();
    let builder = state.entities.row(builder).uuid.to_string();
    grown_world(&mut state, Vec::new(), "birch");
    let blueprint = crate::settlements::blueprints::get(&state.registry, &project.blueprint)
        .expect("blueprint");
    let size = blueprint.footprint(crate::settlements::blueprints::Rotation::from_quarter_turns(
        project.rotation,
    ));
    let bounds = crate::settlements::forestry::lot_bounds(&state, &project).expect("lot");
    let want: rustc_hash::FxHashMap<_, _> =
        ((bounds.min[0] - 8).div_euclid(16)..=(bounds.max[0] + 8).div_euclid(16))
            .flat_map(|x| {
                ((bounds.min[2] - 8).div_euclid(16)..=(bounds.max[2] + 8).div_euclid(16))
                    .map(move |z| (crate::boundary::ChunkPos { x, z }, 0))
            })
            .collect();
    state.world.ensure_blocking(&want);
    state.world.fold_overlay();
    let root = [bounds.min[0] + 2, 201, bounds.min[2] + 2];
    let leaves = crate::settlements::forestry::native_tree_tests::grow(&mut state, root, 5);
    state
        .settlements
        .get_mut(id)
        .expect("settlement")
        .planted_trees
        .insert(root);
    let mut clearance = crate::settlements::forestry::Clearance::default();
    let preparation = crate::settlements::forestry::cells(&state, &project, size, &mut clearance)
        .expect("a registered tree does not refuse its own lot");
    state.settlements.get_mut(id).expect("settlement").project = Some(project);
    let mut felled = 0;
    for cell in &preparation.cells {
        let block = state
            .world
            .block_at(cell.at[0], cell.at[1], cell.at[2])
            .expect("loaded cell");
        if crate::settlements::forestry::tree_or_sapling(&state, block) {
            let cells = [super::super::Cell {
                at: cell.at,
                state: state.registry.states.air,
                resource: None,
                quantity: 0,
            }];
            assert_eq!(
                debit(&mut state, id, &builder, &BTreeMap::new(), &cells),
                Ok(()),
                "one-cell debit at {:?}",
                cell.at
            );
            felled += 1;
        }
        // The caller applies the infallible native edit after the debit.
        state
            .world
            .set_block(cell.at[0], cell.at[1], cell.at[2], state.registry.states.air);
    }
    assert!(felled >= 5, "the trunk and its crown were felled: {felled}");
    for leaf in &leaves {
        assert_eq!(
            state.world.block_at(leaf[0], leaf[1], leaf[2]),
            Some(state.registry.states.air),
            "every grown leaf is cleared, {leaf:?}"
        );
    }
    state.tick = 20;
    crate::settlements::forestry::retire_missing(&mut state);
    assert!(
        !state
            .settlements
            .get(id)
            .expect("settlement")
            .planted_trees
            .contains(&root),
        "the root retires after its column is empty"
    );
}
