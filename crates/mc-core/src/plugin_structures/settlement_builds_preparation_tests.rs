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
    let site = ScriptWorldBounds::new(
        [root[0] - 2, root[1], root[2] - 2],
        [root[0] + 3, root[1] + 5, root[2] + 3],
    )
    .expect("the lot bounds");
    let path = block(&state, "minecraft:dirt_path", &[]);
    let dirt = state.registry.states.dirt;
    let planned = PlannedCell {
        at: root,
        state: path,
        resource: None,
        quantity: 0,
    };
    // Ground under the cell, the cell itself clear: paving replaces the space
    // above ground, so the cell is normally air.
    let under = [root[0], root[1] - 1, root[2]];
    state.world.set_block(under[0], under[1], under[2], dirt);
    state
        .world
        .set_block(root[0], root[1], root[2], state.registry.states.air);
    state.world.fold_overlay();
    let handle = prepare(&mut state, id, "storage".into(), site, site, vec![planned])
        .expect("the pavement project prepares");
    let uuid = state.entities.row(builder).uuid;
    // The executor only pays for the project's bound builder.
    state.settlements.get_mut(id).unwrap().projects =
        vec![crate::settlements::construction::Project {
            settlement: id,
            blueprint: "storage".into(),
            level: 1,
            origin: site.min,
            rotation: 0,
            building: None,
            structure: handle.clone(),
            ordered: true,
            builder: Some((uuid, None)),
            order_seq: None,
        }];
    let cell = super::super::Cell {
        at: root,
        state: path,
        resource: None,
        quantity: 0,
    };
    let empty: BTreeMap<String, u64> = BTreeMap::new();
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    let builder = uuid.to_string();
    assert_eq!(
        debit(
            &mut state,
            id,
            &handle,
            &builder,
            &empty,
            std::slice::from_ref(&cell)
        ),
        Ok(()),
        "paving over real ground is free work"
    );
    assert_eq!(
        state.block_entities[&(1, 201, 1)].slots.clone(),
        stock,
        "paving charges nothing"
    );
    // The player digs the ground out before the project resumes: the same free
    // cell is now a block from nothing, so the project pauses.
    state
        .world
        .set_block(root[0], root[1], root[2], state.registry.states.air);
    state
        .world
        .set_block(under[0], under[1], under[2], state.registry.states.air);
    state.world.fold_overlay();
    assert_eq!(
        debit(&mut state, id, &handle, &builder, &empty, &[cell]),
        Err(Failure::Blocked)
    );
    assert_eq!(
        state.world.block_at(root[0], root[1], root[2]),
        Some(state.registry.states.air),
        "air over a dug column is never paved"
    );
}

#[test]
fn a_pending_detour_holds_its_corridor_against_a_neighbour_lot() {
    let (mut state, id, _builder, _, root, _) = crate::settlements::forestry::tests::forest();
    let site = ScriptWorldBounds::new(
        [root[0] - 2, root[1], root[2] - 2],
        [root[0] + 3, root[1] + 5, root[2] + 3],
    )
    .expect("the lot bounds");
    let path = block(&state, "minecraft:dirt_path", &[]);
    let detour_at = [root[0] + 8, root[1], root[2]];
    let bounds = ScriptWorldBounds::new(
        [site.min[0], site.min[1], site.min[2]],
        [detour_at[0], site.max[1], site.max[2]],
    )
    .expect("the placement bounds cover the detour");
    let cell = |at: [i32; 3]| PlannedCell {
        at,
        state: path,
        resource: None,
        quantity: 0,
    };
    // A prepares with a detour cell outside its authored site.
    prepare(
        &mut state,
        id,
        "storage".into(),
        bounds,
        site,
        vec![cell(root), cell(detour_at)],
    )
    .expect("A prepares");
    // B's site would take that detour corridor: the pending detour holds it.
    let neighbour = ScriptWorldBounds::new(
        [detour_at[0] - 1, detour_at[1], detour_at[2] - 1],
        [detour_at[0] + 1, detour_at[1] + 5, detour_at[2] + 1],
    )
    .expect("the neighbour bounds");
    assert_eq!(
        prepare(
            &mut state,
            id,
            "storage".into(),
            neighbour,
            neighbour,
            vec![cell(detour_at)]
        ),
        Err(Failure::Blocked),
        "an unbuilt detour holds its whole corridor"
    );
}
