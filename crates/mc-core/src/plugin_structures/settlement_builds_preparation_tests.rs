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
/// as one grove, every tree of it is certified, and the storage credit is
/// exactly the vanilla loot of the felled cells.
#[test]
fn touching_grove_is_felled_whole_with_exact_drops() {
    use crate::settlements::forestry::tests::{forest, grown_world};
    let (mut state, id, builder, project, _, _) = forest();
    let builder = state.entities.row(builder).uuid.to_string();
    grown_world(&mut state, Vec::new(), "birch");
    let left = [0, 201, 0];
    let right = [5, 201, 0];
    let want: rustc_hash::FxHashMap<_, _> = (-1..=1)
        .flat_map(|x| (-1..=1).map(move |z| (crate::boundary::ChunkPos { x, z }, 0)))
        .collect();
    state.world.ensure_blocking(&want);
    state.world.fold_overlay();
    crate::settlements::forestry::native_tree_tests::grow(&mut state, left, 5);
    crate::settlements::forestry::native_tree_tests::grow(&mut state, right, 5);
    let record = state.settlements.get_mut(id).expect("settlement");
    record.planted_trees.insert(left);
    record.planted_trees.insert(right);
    let lot = mc_script::ScriptWorldBounds::new(
        [left[0] - 4, left[1] - 1, left[2] - 4],
        [left[0] + 4, left[1] + 64, left[2] + 4],
    )
    .expect("witness lot");
    let mut clearance = crate::settlements::forestry::Clearance::default();
    for at in tree_cells(&state, &[left]) {
        assert!(
            clearance.clearable(&state, at, Some(lot)),
            "the left tree is certified at {at:?}"
        );
    }
    let mut planned = Vec::new();
    clearance.append_whole_trees(&state, &mut planned);
    let mut manifested: Vec<_> = planned.iter().map(|cell| cell.at).collect();
    manifested.sort_unstable();
    manifested.dedup();
    assert_eq!(manifested.len(), planned.len(), "no cell is felled twice");
    assert!(manifested.contains(&left), "the on-lot trunk is felled");
    assert!(
        manifested.contains(&right),
        "the touching neighbouring trunk is felled whole"
    );
    let mut expected = counts(&state);
    for cell in &planned {
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
    // The canonical executor advances one cell per debit and writes after it.
    for cell in &planned {
        let cells = [super::super::Cell {
            at: cell.at,
            state: cell.state,
            resource: cell.resource.clone(),
            quantity: cell.quantity,
        }];
        assert_eq!(
            debit(&mut state, id, &builder, &BTreeMap::new(), &cells),
            Ok(()),
            "one-cell grove debit at {:?}",
            cell.at
        );
        state.world.set_block(
            cell.at[0],
            cell.at[1],
            cell.at[2],
            state.registry.states.air,
        );
    }
    assert_eq!(counts(&state), expected, "exact vanilla drops into storage");
    for at in [left, right] {
        assert!(
            !state
                .world
                .block_at(at[0], at[1], at[2])
                .is_some_and(|block| crate::settlements::forestry::tree_or_sapling(&state, block)),
            "the grove is gone at {at:?}"
        );
    }
}

/// Tree blocks inside and around the given roots, for the manual manifest.
fn tree_cells(state: &TickState, around: &[[i32; 3]]) -> Vec<[i32; 3]> {
    let mut cells = Vec::new();
    for &root in around {
        for y in root[1]..=root[1] + 8 {
            for dx in -4i32..=4 {
                for dz in -4i32..=4 {
                    let at = [root[0] + dx, y, root[2] + dz];
                    if state
                        .world
                        .block_at(at[0], at[1], at[2])
                        .is_some_and(|block| {
                            crate::settlements::forestry::tree_or_sapling(state, block)
                        })
                    {
                        cells.push(at);
                    }
                }
            }
        }
    }
    cells.sort_unstable();
    cells.dedup();
    cells
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
    let size = blueprint
        .footprint(crate::settlements::blueprints::Rotation::from_quarter_turns(project.rotation));
    let mut clearance = crate::settlements::forestry::Clearance::default();
    let preparation = crate::settlements::forestry::cells(&state, &project, size, &mut clearance)
        .expect("the natural lot is admitted");
    assert!(
        !state.block_log.enabled(),
        "capture disabled: no attribution"
    );
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

/// P2 livelock guard: selection never certifies a log outside its root's own
/// column, which is exactly what the executor proves, so a trunk that steps
/// sideways refuses the whole tree **before** any clearing instead of being
/// certified and then refused cell by cell (the requeue loop).
#[test]
fn off_column_trunk_steps_are_refused_before_clearing() {
    use crate::settlements::forestry::tests::forest;
    let (mut state, id, _, _, _, _) = forest();
    let root = [0, 201, 0];
    let dirt = block(&state, "minecraft:dirt", &[]);
    let log = block(&state, "minecraft:acacia_log", &[]);
    state.world.set_block(root[0], root[1] - 1, root[2], dirt);
    let mut trunk = Vec::new();
    for (dx, dy) in [(0, 0), (0, 1), (0, 2), (1, 3), (1, 4), (2, 5), (2, 6)] {
        let at = [root[0] + dx, root[1] + dy, root[2]];
        state.world.set_block(at[0], at[1], at[2], log);
        trunk.push(at);
    }
    state
        .settlements
        .get_mut(id)
        .expect("settlement")
        .planted_trees
        .insert(root);
    let lot = mc_script::ScriptWorldBounds::new(
        [root[0] - 4, root[1] - 1, root[2] - 4],
        [root[0] + 4, root[1] + 64, root[2] + 4],
    )
    .expect("witness lot");
    let mut clearance = crate::settlements::forestry::Clearance::default();
    for at in &trunk {
        assert!(
            !clearance.clearable(&state, *at, Some(lot)),
            "the off-column step makes the whole component ambiguous: {at:?}"
        );
    }
    let mut planned = Vec::new();
    clearance.append_whole_trees(&state, &mut planned);
    assert!(
        planned.is_empty(),
        "nothing is manifested, so the executor is never handed a failing cell"
    );
    // Worldgen evidence is the supported way for a sideways-grown vanilla tree:
    // a natural acacia is cleared per cell by `natural`, not by this walk.
    assert!(
        state
            .settlements
            .get(id)
            .expect("settlement")
            .planted_trees
            .contains(&root),
        "the registration is untouched by a refusal"
    );
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
    let root = [0, 201, 0];
    let leaves = crate::settlements::forestry::native_tree_tests::grow(&mut state, root, 5);
    state
        .settlements
        .get_mut(id)
        .expect("settlement")
        .planted_trees
        .insert(root);
    let lot = mc_script::ScriptWorldBounds::new(
        [root[0] - 4, root[1] - 1, root[2] - 4],
        [root[0] + 4, root[1] + 64, root[2] + 4],
    )
    .expect("witness lot");
    let mut clearance = crate::settlements::forestry::Clearance::default();
    for at in tree_cells(&state, &[root]) {
        assert!(clearance.clearable(&state, at, Some(lot)));
    }
    let mut planned = Vec::new();
    clearance.append_whole_trees(&state, &mut planned);
    let trunk: Vec<_> = (0..5).map(|dy| [root[0], root[1] + dy, root[2]]).collect();
    for at in &trunk {
        assert!(
            planned.iter().any(|cell| cell.at == *at),
            "trunk cell {at:?} is certified"
        );
    }
    state.settlements.get_mut(id).expect("settlement").project = Some(project);
    // Manifest order is the executor's own order; the trunk is cleared before
    // the crown, which is exactly the identity hazard this test pins.
    let mut felled = 0;
    for cell in &planned {
        let cells = [super::super::Cell {
            at: cell.at,
            state: cell.state,
            resource: cell.resource.clone(),
            quantity: cell.quantity,
        }];
        assert_eq!(
            debit(&mut state, id, &builder, &BTreeMap::new(), &cells),
            Ok(()),
            "one-cell debit at {:?}",
            cell.at
        );
        state.world.set_block(
            cell.at[0],
            cell.at[1],
            cell.at[2],
            state.registry.states.air,
        );
        felled += 1;
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
