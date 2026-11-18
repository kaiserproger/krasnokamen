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
fn a_dug_out_free_pavement_cell_writes_nothing_and_the_plan_completes() {
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
    // A free pavement cell is the soil itself: a player dug it out (air, with
    // air under it) before the builder reached it.
    state
        .world
        .set_block(root[0], root[1], root[2], state.registry.states.air);
    state
        .world
        .set_block(root[0], root[1] - 1, root[2], state.registry.states.air);
    state.world.fold_overlay();
    let handle = prepare(&mut state, id, "storage".into(), site, site, vec![planned])
        .expect("the pavement project prepares");
    let uuid = state.entities.row(builder).uuid;
    bind(&mut state, id, &handle, uuid, site.min);
    let empty: BTreeMap<String, u64> = BTreeMap::new();
    let builder = uuid.to_string();
    assert_eq!(
        debit(&mut state, id, &handle, &builder, &empty, &[]),
        Ok(()),
        "the debit is not where the dug cell is refused"
    );
    advance(&mut state, id, &handle, uuid).expect("the stage advances past the dug cell");
    assert_eq!(
        state.world.block_at(root[0], root[1], root[2]),
        Some(state.registry.states.air),
        "no block is written into the dug column"
    );
    let _ = dirt;
}

#[test]
fn a_foreign_block_in_a_free_pavement_cell_is_untouched() {
    let (mut state, id, builder, _, root, _) = crate::settlements::forestry::tests::forest();
    let site = ScriptWorldBounds::new(
        [root[0] - 2, root[1], root[2] - 2],
        [root[0] + 3, root[1] + 5, root[2] + 3],
    )
    .expect("the lot bounds");
    let path = block(&state, "minecraft:dirt_path", &[]);
    let planks = block(&state, "minecraft:oak_planks", &[]);
    let planned = PlannedCell {
        at: root,
        state: path,
        resource: None,
        quantity: 0,
    };
    state.world.set_block(root[0], root[1], root[2], planks);
    state.world.fold_overlay();
    let handle = prepare(&mut state, id, "storage".into(), site, site, vec![planned])
        .expect("the pavement project prepares");
    let uuid = state.entities.row(builder).uuid;
    bind(&mut state, id, &handle, uuid, site.min);
    let builder = uuid.to_string();
    let empty: BTreeMap<String, u64> = BTreeMap::new();
    assert_eq!(
        debit(&mut state, id, &handle, &builder, &empty, &[]),
        Ok(())
    );
    advance(&mut state, id, &handle, uuid).expect("the stage advances past the foreign cell");
    assert_eq!(
        state.world.block_at(root[0], root[1], root[2]),
        Some(planks),
        "a foreign block is never overwritten by free paving"
    );
}

#[test]
fn a_dug_out_free_retirement_cell_never_reopens_the_house() {
    let (mut state, id, builder, _, root, _) = crate::settlements::forestry::tests::forest();
    let site = ScriptWorldBounds::new(
        [root[0] - 2, root[1], root[2] - 2],
        [root[0] + 3, root[1] + 5, root[2] + 3],
    )
    .expect("the lot bounds");
    let dirt = state.registry.states.dirt;
    let planned = PlannedCell {
        at: root,
        state: dirt,
        resource: None,
        quantity: 0,
    };
    let path = block(&state, "minecraft:dirt_path", &[]);
    state.world.set_block(root[0], root[1], root[2], path);
    state.world.fold_overlay();
    let handle = prepare(&mut state, id, "storage".into(), site, site, vec![planned])
        .expect("the retirement project prepares");
    let uuid = state.entities.row(builder).uuid;
    bind(&mut state, id, &handle, uuid, site.min);
    let builder = uuid.to_string();
    let empty: BTreeMap<String, u64> = BTreeMap::new();
    assert_eq!(
        debit(&mut state, id, &handle, &builder, &empty, &[]),
        Ok(())
    );
    assert!(advance(&mut state, id, &handle, uuid).expect("the stage advances"));
    // The house stood; the player digs the free retirement cell out afterwards.
    state
        .world
        .set_block(root[0], root[1], root[2], state.registry.states.air);
    state.world.fold_overlay();
    assert!(
        !reopen_emptied(&mut state, id, &handle).expect("the certification reads"),
        "a dug free soil cell has nothing to restore and never reopens the house"
    );
}

fn bind(state: &mut TickState, id: SettlementId, handle: &str, uuid: uuid::Uuid, origin: [i32; 3]) {
    state.settlements.get_mut(id).unwrap().projects =
        vec![crate::settlements::construction::Project {
            settlement: id,
            blueprint: "storage".into(),
            level: 1,
            origin,
            rotation: 0,
            building: None,
            structure: handle.to_string(),
            ordered: true,
            builder: Some((uuid, None)),
            order_seq: None,
        }];
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
