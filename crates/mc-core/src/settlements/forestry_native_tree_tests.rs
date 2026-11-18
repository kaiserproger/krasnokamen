use super::*;
use crate::settlements::SettlementId;
use crate::settlements::allocator::tests::block;

/// Exercise native growth, rather than a hand-painted crown or regeneration.
pub(crate) fn grow(state: &mut TickState, root: [i32; 3], height: i32) -> HashSet<[i32; 3]> {
    let dirt = block(state, "minecraft:dirt", &[]);
    let sapling = block(state, "minecraft:birch_sapling", &[("stage", "1")]);
    let log = block(state, "minecraft:birch_log", &[]);
    state.world.set_block(root[0], root[1] - 1, root[2], dirt);
    state.world.set_block(root[0], root[1], root[2], sapling);
    let at = crate::boundary::BlockPos {
        x: root[0],
        y: root[1],
        z: root[2],
    };
    let growth = (0..256)
        .find_map(|seed| {
            let plan = crate::plants::bonemeal(state, at, seed)?;
            let top = plan
                .writes
                .iter()
                .filter(|(_, value)| *value == log)
                .map(|(at, _)| at.y)
                .max()?;
            (top == root[1] + height - 1).then_some(plan)
        })
        .expect("native birch of the requested height can grow in this fixture");
    let leaves = growth
        .writes
        .iter()
        .filter_map(|(at, value)| {
            state
                .registry
                .blocks
                .by_id(BlockStateId(*value))
                .is_some_and(|definition| definition.block.id.path() == "birch_leaves")
                .then_some([at.x, at.y, at.z])
        })
        .collect();
    for (at, value) in growth.writes {
        state.world.set_block(at.x, at.y, at.z, value);
    }
    leaves
}

pub(crate) fn overlapping(connected: bool) -> (TickState, SettlementId, [i32; 3], [i32; 3]) {
    let (mut state, id, _, _, _, _) = super::tests::forest();
    super::tests::grown_world(&mut state, Vec::new(), "birch");
    let root = [0, 201, 0];
    let foreign = [if connected { 1 } else { 4 }, 201, 0];
    let own_leaves = grow(&mut state, root, 5);
    let foreign_leaves = grow(&mut state, foreign, if connected { 7 } else { 5 });
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .planted_trees
        .insert(root);
    let witness = own_leaves
        .intersection(&foreign_leaves)
        .copied()
        .filter(|at| {
            (at[0], at[2]) != (root[0], root[2]) && (at[0], at[2]) != (foreign[0], foreign[2])
        })
        .min()
        .expect("the two actually grown crowns overlap away from both trunks");
    assert!(!state.block_log.player_touched(witness));
    let leaf = state
        .world
        .block_at(witness[0], witness[1], witness[2])
        .unwrap();
    assert_eq!(
        crate::farming::prop(&state.registry.blocks, leaf, "persistent"),
        Some("false")
    );
    (state, id, root, witness)
}

fn preserves_foreign_crown(connected: bool) {
    let (state, _, root, witness) = overlapping(connected);
    let lot = Some(super::tests::lot_around(root));
    let leaf = state
        .world
        .block_at(witness[0], witness[1], witness[2])
        .unwrap();
    let mut clearance = Clearance::default();
    assert!(!clearance.clearable(&state, witness, lot));
    assert!(super::drops(&state, witness, leaf, lot, &mut clearance).is_err());
    assert_eq!(
        state.world.block_at(witness[0], witness[1], witness[2]),
        Some(leaf)
    );
    if !connected {
        // Native foliage on the opposite side is seven steps from the foreign
        // trunk, but attached to the registered trunk. Do not reject all trees.
        let own = [-2, root[1] + 3, 1];
        assert_eq!(
            state
                .registry
                .blocks
                .by_id(BlockStateId(
                    state.world.block_at(own[0], own[1], own[2]).unwrap()
                ))
                .unwrap()
                .block
                .id
                .path(),
            "birch_leaves"
        );
        assert!(clearance.clearable(&state, own, lot));
    }
}

#[test]
fn native_separated_player_tree_with_overlapping_crown_is_kept() {
    preserves_foreign_crown(false);
}

#[test]
fn native_touching_player_tree_with_a_second_soil_root_is_kept() {
    preserves_foreign_crown(true);
}

#[test]
fn vanilla_acacia_diagonal_trunk_steps_are_owned_and_conserved() {
    let (mut state, id, _, _, root, _) = super::tests::forest();
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .planted_trees
        .insert(root);
    // ForkingTrunkPlacer 26.3 advances Y and X/Z in the same step.
    let log = block(&state, "minecraft:acacia_log", &[]);
    let branch = [root[0] + 1, root[1] + 4, root[2]];
    let tip = [root[0] + 2, root[1] + 5, root[2]];
    for at in [branch, tip] {
        state.world.set_block(at[0], at[1], at[2], log);
    }
    let lot = Some(super::tests::lot_around(root));
    let mut clearance = Clearance::default();
    for at in [branch, tip] {
        assert!(clearance.clearable(&state, at, lot));
        let drops = super::drops(&state, at, log, lot, &mut clearance).unwrap();
        assert_eq!(drops.iter().map(|stack| stack.count).sum::<i32>(), 1);
        assert_eq!(
            state
                .registry
                .items
                .name_of(drops[0].item_id)
                .unwrap()
                .as_str(),
            "minecraft:acacia_log"
        );
    }
    let mut cells = Vec::new();
    clearance.append_whole_trees(&state, &mut cells);
        assert!(
            !cells.iter().any(|cell| cell.at == at),
            "the off-column step is never manifested: {at:?}"
        );
    }
    for at in [branch, tip] {
}

#[test]
fn a_selection_reuses_one_proof_for_every_leaf_of_a_registered_root() {
    let (mut state, id, _, _, _, _) = super::tests::forest();
    super::tests::grown_world(&mut state, Vec::new(), "birch");
    let root = [0, 201, 0];
    let leaves = grow(&mut state, root, 5);
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .planted_trees
        .insert(root);
    let mut clearance = Clearance::default();
    let lot = Some(super::tests::lot_around(root));
    for _ in 0..4 {
        for &at in &leaves {
            assert!(clearance.clearable(&state, at, lot));
        }
    }
    assert_eq!(clearance.proofs.as_ref().unwrap().len(), 1);
    assert!(Clearance::prepared().proofs.is_none());
}

#[test]
fn loaded_registered_trunk_with_an_unloaded_adjacent_column_is_kept() {
    let (mut state, id, _, _, _, _) = super::tests::forest();
    super::tests::grown_world(&mut state, Vec::new(), "birch");
    let root = [31, 201, 31];
    let leaf = [30, 204, 31];
    let log = block(&state, "minecraft:birch_log", &[]);
    let foliage = block(&state, "minecraft:birch_leaves", &[("persistent", "false")]);
    state.world.set_block(root[0], root[1], root[2], log);
    state.world.set_block(leaf[0], leaf[1], leaf[2], foliage);
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .planted_trees
        .insert(root);
    assert_eq!(state.world.physics_block_at(32, root[1], root[2]), None);
    let lot = Some(super::tests::lot_around(root));
    let mut clearance = Clearance::default();
    assert!(!clearance.clearable(&state, root, lot));
    assert!(!clearance.clearable(&state, leaf, lot));
}

#[test]
fn load_time_tree_tables_match_vanilla_tags_and_every_farmland_state() {
    let (state, _, _, _, _, _) = super::tests::forest();
    for definition in state.registry.blocks.states() {
        let id = definition.id.0 as usize;
        let raw = definition.block.raw_id;
        assert_eq!(
            state.registry.tree_logs[id],
            state.registry.tags.contains_raw_id(
                mc_data::tags::BLOCK_REGISTRY,
                "minecraft:logs",
                raw
            )
        );
        assert_eq!(
            state.registry.tree_soil[id],
            state.registry.tags.contains_raw_id(
                mc_data::tags::BLOCK_REGISTRY,
                "minecraft:dirt",
                raw
            ) || state.registry.farming.is_farmland(definition.id.0)
        );
    }
    for moisture in ["0", "1", "2", "3", "4", "5", "6", "7"] {
        let farmland = block(&state, "minecraft:farmland", &[("moisture", moisture)]);
        assert!(state.registry.farming.is_farmland(farmland));
        assert!(state.registry.tree_soil[farmland as usize]);
    }
    assert!(!state.registry.farming.is_farmland(u32::MAX));
    for name in [
        "minecraft:birch_log",
        "minecraft:stripped_birch_log",
        "minecraft:birch_wood",
        "minecraft:crimson_hyphae",
    ] {
        let log = block(&state, name, &[]);
        assert!(state.registry.tree_logs[log as usize], "{name}");
    }
}
