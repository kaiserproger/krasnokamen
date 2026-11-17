use super::*;
use crate::settlements::allocator::tests::block;

fn planted() -> (TickState, crate::settlements::SettlementId, [i32; 3]) {
    let (mut state, id, _, _, _, _) = tests::forest();
    tests::grown_world(&mut state, Vec::new(), "birch");
    let root = [0, 201, 0];
    native_tree_tests::grow(&mut state, root, 5);
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .planted_trees
        .insert(root);
    (state, id, root)
}

/// Two registered trees whose crowns touch. Only the left tree stands on the
/// lot; the right trunk is outside it. N5: the leaves between them are one
/// union, and touching a grove fells every tree of it whole.
fn grove() -> (
    TickState,
    crate::settlements::SettlementId,
    [i32; 3],
    [i32; 3],
    Vec<[i32; 3]>,
) {
    let (mut state, id, _, _, _, _) = tests::forest();
    tests::grown_world(&mut state, Vec::new(), "birch");
    let left = [0, 201, 0];
    let right = [5, 201, 0];
    let leaves = native_tree_tests::grow(&mut state, left, 5)
        .into_iter()
        .collect();
    native_tree_tests::grow(&mut state, right, 5);
    let record = state.settlements.get_mut(id).unwrap();
    record.planted_trees.insert(left);
    record.planted_trees.insert(right);
    (state, id, left, right, leaves)
}

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
                        .is_some_and(|block| tree_or_sapling(state, block))
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

#[test]
fn touching_registered_grove_is_certified_and_felled_whole() {
    let (state, _, left, right, leaves) = grove();
    let lot = tests::lot_around(left);
    assert!(
        !(lot.min[0]..=lot.max[0]).contains(&right[0]),
        "right trunk is off the lot"
    );
    let mut clearance = Clearance::default();
    // The witness leaf is the left tree's outermost leaf towards the right
    // trunk: it lies between the trunks and is nearer than RANGE to that trunk.
    let leaf = *leaves
        .iter()
        .filter(|at| (lot.min[0]..=lot.max[0]).contains(&at[0]))
        .max_by_key(|at| at[0])
        .expect("the left crown has leaves on the lot");
    let reach = (0..5)
        .map(|dy| (leaf[0] - right[0]).abs() + (leaf[1] - right[1] - dy).abs())
        .min()
        .expect("right trunk column");
    assert!(
        reach <= 6,
        "the witness leaf is nearer than range to the foreign trunk: {leaf:?} {reach}"
    );
    assert_eq!(
        state
            .registry
            .blocks
            .by_id(BlockStateId(
                state
                    .world
                    .block_at(leaf[0], leaf[1], leaf[2])
                    .expect("loaded leaf")
            ))
            .expect("leaf definition")
            .block
            .id
            .path(),
        "birch_leaves"
    );
    for at in tree_cells(&state, &[left]) {
        assert!(
            clearance.clearable(&state, at, Some(lot)),
            "left tree cell {at:?} is certified"
        );
    }
    let mut cells: Vec<_> = tree_cells(&state, &[left])
        .into_iter()
        .map(|at| crate::plugin_structures::settlement_builds::PlannedCell {
            at,
            state: state.registry.states.air,
            resource: None,
            quantity: 0,
        })
        .collect();
    clearance.append_whole_trees(&state, &mut cells);
    let mut manifested: Vec<_> = cells.iter().map(|cell| cell.at).collect();
    manifested.sort_unstable();
    manifested.dedup();
    assert_eq!(manifested.len(), cells.len(), "no cell is felled twice");
    let mut expected = tree_cells(&state, &[left, right]);
    expected.sort_unstable();
    assert_eq!(
        manifested, expected,
        "the whole touching grove is felled, both trees once"
    );
    assert!(
        manifested.contains(&right),
        "the off-lot registered trunk is felled with the grove"
    );
}

#[test]
fn player_log_in_a_grove_makes_the_whole_component_ambiguous() {
    let (mut state, _, root) = planted();
    let at = [root[0] + 1, root[1] + 3, root[2]];
    let log = block(&state, "minecraft:birch_log", &[]);
    state.world.set_block(at[0], at[1], at[2], log);
    let lot = tests::lot_around(root);
    let mut clearance = Clearance::default();
    assert!(clearance.clearable(&state, root, Some(lot)));
    state.block_log = tests::player_history(&state, at, 0);
    assert!(state.block_log.player_touched(at));
    let mut clearance = Clearance::default();
    assert!(
        !clearance.clearable(&state, root, Some(lot)),
        "a player log touching the trunk keeps the tree"
    );
    let mut cells = Vec::new();
    clearance.append_whole_trees(&state, &mut cells);
    assert!(cells.is_empty(), "no part of the tree is manifested");
    assert_eq!(state.world.block_at(at[0], at[1], at[2]), Some(log));
}

#[test]
fn diagonal_log_is_foreign_because_no_grown_tree_has_one() {
    // Committee decision (D20 review, Opus evidence): a limb that steps both
    // horizontal axes at once is not a shape this tree grows. Saplings grow a
    // straight trunk (mc-world plant_rules_26_1_2.rs:1057-1080) and worldgen
    // refuses `fancy_trunk_placer` (mc-worldgen vanilla_features_tests.rs:2259),
    // so the walk stays with vanilla forking steps, which advance one horizontal
    // axis. Broadening it to 26-adjacency would instead join two diagonal
    // registered trunks (each standing on soil, so both would become ambiguous).
    let (mut state, _, root) = planted();
    let log = block(&state, "minecraft:birch_log", &[]);
    let flat = [root[0] + 1, root[1] + 2, root[2] + 1];
    let rising = [root[0] + 2, root[1] + 3, root[2] + 2];
    for at in [flat, rising] {
        state.world.set_block(at[0], at[1], at[2], log);
    }
    let proof = tree_proof::Proof::new(&state, root, &mut tree_proof::Grove::default());
    for at in [flat, rising] {
        assert!(!proof.owns_log(at), "diagonal log {at:?} stays foreign");
    }
    let lot = tests::lot_around(root);
    let mut clearance = Clearance::default();
    assert!(
        clearance.clearable(&state, root, Some(lot)),
        "the straight trunk is still certified"
    );
    for at in [flat, rising] {
        assert!(!clearance.clearable(&state, at, Some(lot)));
    }
}

#[test]
fn touching_stripped_logs_wood_and_hyphae_are_not_owned_or_manifested() {
    for name in [
        "minecraft:stripped_birch_log",
        "minecraft:oak_wood",
        "minecraft:crimson_hyphae",
    ] {
        let (mut state, _, root) = planted();
        let foreign = [root[0] + 1, root[1] + 2, root[2]];
        let value = block(&state, name, &[]);
        state
            .world
            .set_block(foreign[0], foreign[1], foreign[2], value);
        let proof = tree_proof::Proof::new(&state, root, &mut tree_proof::Grove::default());
        assert!(
            proof.owns_log(root),
            "the registered trunk remains eligible: {name}"
        );
        assert!(!proof.owns_log(foreign), "foreign log-tag variant: {name}");
        let mut clearance = Clearance::default();
        assert!(clearance.clearable(&state, root, Some(tests::lot_around(root))));
        let mut cells = Vec::new();
        clearance.append_whole_trees(&state, &mut cells);
        assert!(cells.iter().any(|cell| cell.at == root));
        assert!(!cells.iter().any(|cell| cell.at == foreign), "{name}");
        assert_eq!(
            state.world.block_at(foreign[0], foreign[1], foreign[2]),
            Some(value)
        );
    }
}

#[test]
fn touching_log_pillar_on_cobblestone_is_ambiguous_and_kept() {
    let (mut state, _, root) = planted();
    let log = block(&state, "minecraft:birch_log", &[]);
    let stone = block(&state, "minecraft:cobblestone", &[]);
    state
        .world
        .set_block(root[0] + 1, root[1] - 1, root[2], stone);
    for y in root[1]..=root[1] + 3 {
        state.world.set_block(root[0] + 1, y, root[2], log);
    }
    let mut clearance = Clearance::default();
    assert!(!clearance.clearable(&state, root, Some(tests::lot_around(root))));
    let mut cells = Vec::new();
    clearance.append_whole_trees(&state, &mut cells);
    assert!(cells.is_empty());
    for y in root[1]..=root[1] + 3 {
        assert_eq!(state.world.block_at(root[0] + 1, y, root[2]), Some(log));
    }
}

#[test]
fn foreign_trunk_five_blocks_away_with_a_touching_limb_is_kept() {
    let (mut state, _, root) = planted();
    let log = block(&state, "minecraft:birch_log", &[]);
    let dirt = state.registry.states.dirt;
    state
        .world
        .set_block(root[0] + 5, root[1] - 1, root[2], dirt);
    for y in root[1]..=root[1] + 3 {
        state.world.set_block(root[0] + 5, y, root[2], log);
    }
    for x in root[0] + 1..=root[0] + 5 {
        state.world.set_block(x, root[1] + 3, root[2], log);
    }
    let proof = tree_proof::Proof::new(&state, root, &mut tree_proof::Grove::default());
    assert!(!proof.owns_log(root));
    for x in root[0]..=root[0] + 5 {
        assert!(!proof.owns_log([x, root[1] + 3, root[2]]));
        assert_eq!(state.world.block_at(x, root[1] + 3, root[2]), Some(log));
    }
}

#[test]
fn proof_height_tracks_the_connected_trunk_top_plus_two_leaf_ranges() {
    let (state, _, root) = planted();
    let proof = tree_proof::Proof::new(&state, root, &mut tree_proof::Grove::default());
    assert_eq!(
        proof.positions(root).map(|at| at[1]).max(),
        Some(root[1] + 4 + 12)
    );
}

#[test]
fn replant_house_scan_never_proves_a_nonoverlapping_registered_root() {
    let (mut state, id, _, _, root, _) = tests::forest();
    tests::grown_world(&mut state, Vec::new(), "birch");
    native_tree_tests::grow(&mut state, root, 5);
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .planted_trees
        .insert([100, 201, 100]);
    let record = state.settlements.get(id).unwrap();
    let blueprint = crate::settlements::blueprints::get(&state.registry, "house").unwrap();
    let mut network = crate::settlements::layout::Network::for_construction(&state, record);
    let before = tree_proof::BUILDS.with(std::cell::Cell::get);
    assert!(crate::settlements::construction::has_house_lot(
        &state,
        record,
        &mut network,
        blueprint
    ));
    assert_eq!(tree_proof::BUILDS.with(std::cell::Cell::get), before);
}

#[test]
fn shared_scan_keeps_needed_whole_trees_local_to_the_selected_candidate() {
    let (mut state, id, _, project, _, _) = tests::forest();
    tests::grown_world(&mut state, Vec::new(), "birch");
    let root = [0, 201, 0];
    native_tree_tests::grow(&mut state, root, 5);
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .planted_trees
        .insert(root);
    let mut clearance = Clearance::default();
    assert!(clearance.clearable(&state, root, Some(tests::lot_around(root))));
    let blueprint =
        crate::settlements::blueprints::get(&state.registry, &project.blueprint).unwrap();
    let size = blueprint
        .footprint(crate::settlements::blueprints::Rotation::from_quarter_turns(project.rotation));
    let preparation = cells(&state, &project, size, &mut clearance).unwrap();
    assert!(!preparation.cells.iter().any(|cell| cell.at == root));
}
