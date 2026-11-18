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
    let handle = project.structure.clone();
    let uuid = state.entities.row(builder).uuid;
    // The executor only pays for the project's bound builder (P2-5).
    project.builder = Some((uuid, None));
    state.settlements.get_mut(id).unwrap().projects = vec![project];
    let builder = uuid.to_string();
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
        debit(&mut state, id, &handle, &builder, &BTreeMap::new(), &cells),
        Ok(())
    );
    assert_eq!(counts(&state), expected, "upgrade clear drops once");
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
    let handle = state.settlements.get(id).unwrap().projects[0]
        .structure
        .clone();
    let uuid = state.entities.row(builder).uuid;
    // The executor only pays for the project's bound builder (P2-5).
    if let Some(mut record) = state.settlements.get_mut(id) {
        record.projects[0].builder = Some((uuid, None));
    }
    let builder = uuid.to_string();
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
            debit(&mut state, id, &handle, &builder, &BTreeMap::new(), &cells),
            Err(Failure::Blocked),
            "{name}"
        );
        assert_eq!(state.world.block_at(root[0], root[1], root[2]), Some(value));
        assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    }
}

#[test]
fn a_player_dug_pavement_cell_pauses_instead_of_paving_air() {
    let (mut state, id, builder, _, root, _) = crate::settlements::forestry::tests::forest();
    crate::settlements::construction::tests::fund(&mut state, "storage", 1);
    crate::settlements::construction::request_build(&mut state, id, "storage").unwrap();
    let builder = state.entities.row(builder).uuid.to_string();
    let path = block(&state, "minecraft:dirt_path", &[]);
    let cells = [super::super::Cell {
        at: root,
        state: path,
        resource: None,
        quantity: 0,
    }];
    state.world.set_block(root[0], root[1], root[2], path);
    state.world.fold_overlay();
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    assert_eq!(
        debit(&mut state, id, &builder, &BTreeMap::new(), &cells),
        Ok(()),
        "village work under the cell is re-laid free"
    );
    assert_eq!(
        state.block_entities[&(1, 201, 1)].slots.clone(),
        stock,
        "paving natural work charges nothing"
    );
    // The player digs the work out before the project resumes: the same free
    // cell would be a block from nothing, so the project pauses instead.
    state
        .world
        .set_block(root[0], root[1], root[2], state.registry.states.air);
    assert_eq!(
        debit(&mut state, id, &builder, &BTreeMap::new(), &cells),
        Err(Failure::Blocked)
    );
    assert_eq!(
        state.world.block_at(root[0], root[1], root[2]),
        Some(state.registry.states.air),
        "air is never paved over"
    );
}
