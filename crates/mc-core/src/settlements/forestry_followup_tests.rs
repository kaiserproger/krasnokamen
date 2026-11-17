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
        let proof = tree_proof::Proof::new(&state, root);
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
    let proof = tree_proof::Proof::new(&state, root);
    assert!(!proof.owns_log(root));
    for x in root[0]..=root[0] + 5 {
        assert!(!proof.owns_log([x, root[1] + 3, root[2]]));
        assert_eq!(state.world.block_at(x, root[1] + 3, root[2]), Some(log));
    }
}

#[test]
fn proof_height_tracks_the_connected_trunk_top_plus_two_leaf_ranges() {
    let (state, _, root) = planted();
    let proof = tree_proof::Proof::new(&state, root);
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
