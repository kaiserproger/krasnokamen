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
fn a_native_haul_journals_recipe_changes_but_not_custody_transfers() {
    let (mut state, id, slot) = crate::settlements::construction::tests::fixture();
    crate::settlements::construction::tests::start(&mut state, id, "house", 1, None);
    crate::settlements::construction::tests::fund(&mut state, "house", 1);
    let item = |name| {
        state
            .registry
            .items
            .id_of(&mc_data::Identifier::parse(name).unwrap())
            .unwrap()
    };
    let logs = item("minecraft:oak_log");
    let planks = item("minecraft:oak_planks");
    for stack in &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots {
        if stack.item_id == planks && !stack.is_empty() {
            *stack = mc_data::ItemStack::new(logs, (stack.count + 3) / 4);
        }
    }
    let uuid = state.entities.row(slot).uuid;
    let project = &mut state.settlements.get_mut(id).unwrap().projects[0];
    project.builder = Some((uuid, None));
    let handle = project.structure.clone();
    let before = counts(&state);
    let next = next_material(&state, id, &handle).unwrap().unwrap().0;
    fetch(&mut state, uuid, &handle, [1, 201, 1], next).unwrap();
    let mut physical_delta: BTreeMap<_, i64> = counts(&state)
        .into_iter()
        .map(|(item, count)| (item, i64::from(count)))
        .collect();
    for stack in state
        .entities
        .row(slot)
        .villager
        .as_ref()
        .unwrap()
        .inventory
        .iter()
        .flatten()
    {
        *physical_delta.entry(stack.item_id).or_default() += i64::from(stack.count);
    }
    for (item, count) in before {
        *physical_delta.entry(item).or_default() -= i64::from(count);
    }
    physical_delta.retain(|_, count| *count != 0);
    assert_eq!(
        physical_delta.len(),
        2,
        "only the log-to-plank recipe transforms stock"
    );
    assert!(physical_delta[&logs] < 0);
    assert_eq!(physical_delta[&planks], -4 * physical_delta[&logs]);
    state.plugins.structure_cache.decoded.take();
    let recovered = ledger(&state).unwrap();
    assert_eq!(
        recovered.structures()[&handle].crafting_delta,
        physical_delta
    );
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
    let before_ids = [portions::identity(&state, block).unwrap()];
    let mut expected = before.clone();
    for drop in crate::settlements::forestry::roll(&state, root, block).unwrap() {
        *expected.entry(drop.item_id).or_default() += drop.count;
    }
    assert_eq!(
        debit(
            &mut state,
            id,
            &handle,
            &builder,
            &BTreeMap::new(),
            &cells,
            &before_ids,
            false,
            &mut BTreeMap::new(),
        ),
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
fn an_upgrade_clear_cell_credits_a_foreign_blocks_drop_exactly_once() {
    use crate::settlements::allocator::tests::block;
    // Owner D27: inside the village's territory a player-built block is cleared
    // like any other, and its own vanilla drop is the village's — the debit must
    // credit a plain solid block, not only foliage.
    let (mut state, id, builder) = crate::settlements::construction::tests::fixture();
    crate::settlements::construction::tests::start(&mut state, id, "house", 1, None);
    let mut project = state.settlements.get(id).unwrap().projects[0].clone();
    project.building = Some(crate::settlements::buildings::BuildingId(1));
    let handle = project.structure.clone();
    let uuid = state.entities.row(builder).uuid;
    project.builder = Some((uuid, None));
    state.settlements.get_mut(id).unwrap().projects = vec![project];
    let builder = uuid.to_string();
    let at = [8, 204, 8];
    let stone = block(&state, "minecraft:stone", &[]);
    state.world.set_block(at[0], at[1], at[2], stone);
    let before = counts(&state);
    let mut expected = before.clone();
    for drop in crate::settlements::forestry::roll(&state, at, stone).unwrap() {
        assert!(!drop.is_empty());
        *expected.entry(drop.item_id).or_default() += drop.count;
    }
    assert_ne!(expected, before, "a solid block drops something");
    let cells = [super::super::Cell {
        at,
        state: state.registry.states.air,
        resource: None,
        quantity: 0,
    }];
    let expected_identity = [portions::identity(&state, stone).unwrap()];
    assert_eq!(
        debit(
            &mut state,
            id,
            &handle,
            &builder,
            &BTreeMap::new(),
            &cells,
            &expected_identity,
            false,
            &mut BTreeMap::new(),
        ),
        Ok(())
    );
    assert_eq!(
        counts(&state),
        expected,
        "a foreign solid block's drop is credited exactly once"
    );
}

#[test]
fn non_tree_log_variants_cannot_pass_uncredited_through_preparation_debit() {
    let (mut state, id, builder, _, root, _) = crate::settlements::forestry::tests::forest();
    crate::settlements::construction::tests::fund(&mut state, "storage", 1);
    crate::settlements::construction::tests::request_started(&mut state, id, "storage");
    let handle = state.settlements.get(id).unwrap().projects[0]
        .structure
        .clone();
    let uuid = state.entities.row(builder).uuid;
    // The executor only pays for the project's bound builder (P2-5).
    if let Some(record) = state.settlements.get_mut(id) {
        record.projects[0].builder = Some((uuid, None));
    }
    state.settlements.get_mut(id).unwrap().radius = 1;
    state.settlements.get_mut(id).unwrap().anchor = [0, 201, 0];
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
        let expected = [portions::identity(&state, value).unwrap()];
        assert_eq!(
            debit(
                &mut state,
                id,
                &handle,
                &builder,
                &BTreeMap::new(),
                &cells,
                &expected,
                false,
                &mut BTreeMap::new(),
            ),
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
    let uuid = state.entities.row(builder).uuid;
    let builder = uuid.to_string();
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
    let handle = bind_project(&mut state, id, project, uuid);
    // The canonical executor advances one cell per debit and writes after it.
    for cell in &planned {
        let cells = [super::super::Cell {
            at: cell.at,
            state: cell.state,
            resource: cell.resource.clone(),
            quantity: cell.quantity,
        }];
        let admitted = [admitted_identity(&state, id, cell.at)];
        assert_eq!(
            debit(
                &mut state,
                id,
                &handle,
                &builder,
                &BTreeMap::new(),
                &cells,
                &admitted,
                false,
                &mut BTreeMap::new(),
            ),
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

/// Installs `project` as the settlement's only project, bound to `builder`
/// (the executor pays only for the project's bound builder), and returns the
/// handle its debits name. The territory shrinks to its anchor: N5/N6 pin the
/// outside-territory executor, whose tree cells must prove their identity;
/// inside territory D27 re-derives a changed cell from the live world instead.
fn bind_project(
    state: &mut TickState,
    id: SettlementId,
    mut project: crate::settlements::construction::Project,
    builder: uuid::Uuid,
) -> String {
    let handle = project.structure.clone();
    project.builder = Some((builder, None));
    let record = state.settlements.get_mut(id).expect("settlement");
    record.projects = vec![project];
    record.radius = 0;
    handle
}

/// The identity admission recorded for an outside-territory cell the world
/// has not changed since.
fn admitted_identity(state: &TickState, id: SettlementId, at: [i32; 3]) -> u32 {
    assert!(
        !portions::inside(state, id, at),
        "{at:?} is outside territory"
    );
    portions::identity(
        state,
        state.world.block_at(at[0], at[1], at[2]).expect("loaded"),
    )
    .expect("identity")
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
    let uuid = state.entities.row(builder).uuid;
    let builder = uuid.to_string();
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
    let handle = bind_project(&mut state, id, project, uuid);
    // Admission certified the leaf in this cell.
    let admitted = [admitted_identity(&state, id, at)];
    // The player breaks that leaf and puts a log in the admitted cell.
    let log = block(&state, "minecraft:oak_log", &[]);
    state.world.set_block(at[0], at[1], at[2], log);
    let cells = [super::super::Cell {
        at,
        state: state.registry.states.air,
        resource: None,
        quantity: 0,
    }];
    assert_eq!(
        debit(
            &mut state,
            id,
            &handle,
            &builder,
            &BTreeMap::new(),
            &cells,
            &admitted,
            false,
            &mut BTreeMap::new(),
        ),
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
    let uuid = state.entities.row(builder).uuid;
    let builder = uuid.to_string();
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
    let handle = bind_project(&mut state, id, project, uuid);
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
        let admitted = [admitted_identity(&state, id, cell.at)];
        assert_eq!(
            debit(
                &mut state,
                id,
                &handle,
                &builder,
                &BTreeMap::new(),
                &cells,
                &admitted,
                false,
                &mut BTreeMap::new(),
            ),
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
        portion: PortionKind::Build,
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
        debit(
            &mut state,
            id,
            &handle,
            &builder,
            &empty,
            &[],
            &[],
            false,
            &mut BTreeMap::new()
        ),
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
        portion: PortionKind::Build,
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
        debit(
            &mut state,
            id,
            &handle,
            &builder,
            &empty,
            &[],
            &[],
            false,
            &mut BTreeMap::new()
        ),
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
    state.registry = Arc::new(
        crate::registry::CoreRegistry::load(Some(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla"),
        ))
        .unwrap(),
    );
    let site = ScriptWorldBounds::new(
        [root[0] - 2, root[1], root[2] - 2],
        [root[0] + 3, root[1] + 5, root[2] + 3],
    )
    .expect("the lot bounds");
    let dirt = state.registry.states.dirt;
    let planned = PlannedCell {
        portion: PortionKind::Build,
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
        debit(
            &mut state,
            id,
            &handle,
            &builder,
            &empty,
            &[],
            &[],
            false,
            &mut BTreeMap::new()
        ),
        Ok(())
    );
    let dirt_item = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:dirt").unwrap())
        .unwrap();
    let stored_soil = |state: &TickState| -> i32 {
        state
            .block_entities
            .values()
            .flat_map(|entity| entity.slots.iter())
            .filter(|stack| stack.item_id == dirt_item && !stack.is_empty())
            .map(|stack| stack.count)
            .sum()
    };
    let before = stored_soil(&state);
    assert!(advance(&mut state, id, &handle, uuid).expect("the stage advances"));
    assert_eq!(
        stored_soil(&state),
        before,
        "free soil work keeps its ground, not a second dirt item"
    );
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
        portion: PortionKind::Build,
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

/// SETTLEMENT-CONSTRUCTION-UPGRADE-PENDING-CELL-UNAVAILABLE: a structure persisted at the terminal
/// stage index without a commit (`state = Running`, `current_stage_index == stages.len()`, zero
/// work — a shape `ScriptStructureSnapshot::validate` accepts) has no next cell, and recovery used
/// to ignore it, so the House L2 upgrade held its slot with `cause="pending_cell_unavailable"` on
/// every shift (20261124T204805-buildings-qa-klwqfvj5). Recovery must finish it.
#[test]
fn a_terminal_index_running_structure_is_committed_by_recovery() {
    let (mut state, id, _slot) = crate::settlements::construction::tests::fixture();
    crate::settlements::construction::tests::start(&mut state, id, "house", 1, None);
    crate::settlements::construction::tests::fund(&mut state, "house", 1);
    let handle = state.settlements.get(id).unwrap().projects[0]
        .structure
        .clone();
    let plan: Vec<([i32; 3], u32)> = {
        let ledger = ledger(&state).unwrap();
        let record = structure(&ledger, &owner(id), &handle).unwrap();
        record
            .cells
            .iter()
            .flatten()
            .map(|cell| (cell.at, cell.state))
            .collect()
    };
    assert!(!plan.is_empty(), "the house plan has cells");
    for (at, block) in plan {
        state.world.set_block(at[0], at[1], at[2], block);
    }
    {
        let mut ledger = ledger_mut(&mut state).unwrap();
        let mut record = take_structure(&mut ledger, &owner(id), &handle).unwrap();
        record.snapshot.current_stage_index = record.snapshot.stages.len() as u32;
        record.snapshot.completed_work_units = 0;
        record.snapshot.state = ScriptStructureState::Running;
        ledger.insert_structure(handle.clone(), record);
        save(&mut state, ledger);
    }
    assert_eq!(
        next_cell(&state, id, &handle).unwrap(),
        None,
        "a terminal-index structure has no next cell: the stall the real client reported"
    );
    assert_eq!(recover(&mut state, id, &handle), Ok(true));
    assert_eq!(
        status(&state, id, &handle).unwrap().state,
        ScriptStructureState::Committed,
        "recovery finishes a placed terminal-index structure instead of leaving it stalled"
    );
}

#[test]
fn a_committed_record_with_a_later_empty_stage_is_reopened_and_finished() {
    let (mut state, id, _slot) = crate::settlements::construction::tests::fixture();
    crate::settlements::construction::tests::start(&mut state, id, "house", 1, None);
    crate::settlements::construction::tests::fund(&mut state, "house", 1);
    let handle = state.settlements.get(id).unwrap().projects[0]
        .structure
        .clone();
    let plan: Vec<([i32; 3], u32)> = {
        let ledger = ledger(&state).unwrap();
        let record = structure(&ledger, &owner(id), &handle).unwrap();
        record
            .cells
            .iter()
            .flatten()
            .map(|cell| (cell.at, cell.state))
            .collect()
    };
    for (at, block) in plan {
        state.world.set_block(at[0], at[1], at[2], block);
    }
    {
        let mut ledger = ledger_mut(&mut state).unwrap();
        let mut record = take_structure(&mut ledger, &owner(id), &handle).unwrap();
        // The exact stalled shape of 20261125T030519: `Committed`, index 2 inside `cells`, its
        // stage planned with no cells, `work=0`, so `next_cell` is None and `finish_project` is
        // never asked to close the project.
        record.snapshot.stages.push(ScriptStructureStagePlan::new(
            "build".into(),
            0,
            0,
            Vec::new(),
        ));
        record.cells.push(Vec::new());
        record.snapshot.current_stage_index = (record.cells.len() - 1) as u32;
        record.snapshot.completed_work_units = 0;
        record.snapshot.state = ScriptStructureState::Committed;
        ledger.insert_structure(handle.clone(), record);
        save(&mut state, ledger);
    }
    assert_eq!(
        next_cell(&state, id, &handle).unwrap(),
        None,
        "the stalled shape names no cell"
    );
    assert_eq!(step_empty_stage(&mut state, id, &handle), Ok(true));
    assert_eq!(
        status(&state, id, &handle).unwrap().state,
        ScriptStructureState::Running,
        "a committed record with a stage left is reopened, not left finished"
    );
    assert_eq!(recover(&mut state, id, &handle), Ok(true));
    assert_eq!(
        status(&state, id, &handle).unwrap().state,
        ScriptStructureState::Committed,
        "recovery then finishes it at the terminal index"
    );
}

#[test]
fn a_stage_whose_cells_are_all_worked_is_stepped_over() {
    let (mut state, id, _slot) = crate::settlements::construction::tests::fixture();
    crate::settlements::construction::tests::start(&mut state, id, "house", 1, None);
    crate::settlements::construction::tests::fund(&mut state, "house", 1);
    let handle = state.settlements.get(id).unwrap().projects[0]
        .structure
        .clone();
    {
        let mut ledger = ledger_mut(&mut state).unwrap();
        let mut record = take_structure(&mut ledger, &owner(id), &handle).unwrap();
        // The House L2 shape `stage=2 work=N next=None`: every cell of the current stage worked,
        // index not advanced. `next_cell` is None and only a step can move it.
        let cells = record.cells[record.snapshot.current_stage_index as usize].len();
        record.snapshot.completed_work_units = cells as u64;
        record.snapshot.state = ScriptStructureState::Running;
        ledger.insert_structure(handle.clone(), record);
        save(&mut state, ledger);
    }
    assert_eq!(
        next_cell(&state, id, &handle).unwrap(),
        None,
        "a worked-through stage names no cell"
    );
    assert_eq!(step_empty_stage(&mut state, id, &handle), Ok(true));
    let after = status(&state, id, &handle).unwrap();
    assert!(
        after.current_stage_index > 0 && after.completed_work_units == 0,
        "the step moves to the next stage and resets its work counter"
    );
}

#[test]
fn the_watcher_step_over_an_empty_stage_names_the_following_cell() {
    let (mut state, id, _slot) = crate::settlements::construction::tests::fixture();
    crate::settlements::construction::tests::start(&mut state, id, "house", 1, None);
    crate::settlements::construction::tests::fund(&mut state, "house", 1);
    let handle = state.settlements.get(id).unwrap().projects[0]
        .structure
        .clone();
    {
        let mut ledger = ledger_mut(&mut state).unwrap();
        let mut record = take_structure(&mut ledger, &owner(id), &handle).unwrap();
        // Insert an empty stage in the middle so the step-over must leave the index on a stage
        // that has a cell: the watcher path of the House L2 stall.
        let index = record.snapshot.current_stage_index as usize;
        record.snapshot.stages.insert(
            index,
            ScriptStructureStagePlan::new("build".into(), 0, 0, Vec::new()),
        );
        record.cells.insert(index, Vec::new());
        record.snapshot.completed_work_units = 0;
        record.snapshot.state = ScriptStructureState::Running;
        ledger.insert_structure(handle.clone(), record);
        save(&mut state, ledger);
    }
    assert_eq!(
        next_cell(&state, id, &handle).unwrap(),
        None,
        "the empty stage names no cell"
    );
    assert_eq!(step_empty_stage(&mut state, id, &handle), Ok(true));
    assert!(
        next_cell(&state, id, &handle).unwrap().is_some(),
        "stepping past the empty stage names the following stage's first cell"
    );
}

#[test]
fn an_empty_planned_stage_is_stepped_over_by_recovery() {
    let (mut state, id, _slot) = crate::settlements::construction::tests::fixture();
    crate::settlements::construction::tests::start(&mut state, id, "house", 1, None);
    crate::settlements::construction::tests::fund(&mut state, "house", 1);
    let handle = state.settlements.get(id).unwrap().projects[0]
        .structure
        .clone();
    let plan: Vec<([i32; 3], u32)> = {
        let ledger = ledger(&state).unwrap();
        let record = structure(&ledger, &owner(id), &handle).unwrap();
        record
            .cells
            .iter()
            .flatten()
            .map(|cell| (cell.at, cell.state))
            .collect()
    };
    for (at, block) in plan {
        state.world.set_block(at[0], at[1], at[2], block);
    }
    {
        let mut ledger = ledger_mut(&mut state).unwrap();
        let mut record = take_structure(&mut ledger, &owner(id), &handle).unwrap();
        // The House L2 shape on the real client (20261124T231255): index 2 inside `cells`, its
        // stage has no cells, zero work, state Running, so `next_cell` is None and no builder pass
        // can move it.
        record.snapshot.stages.push(ScriptStructureStagePlan::new(
            "build".into(),
            0,
            0,
            Vec::new(),
        ));
        record.cells.push(Vec::new());
        record.snapshot.current_stage_index = (record.cells.len() - 1) as u32;
        record.snapshot.completed_work_units = 0;
        record.snapshot.state = ScriptStructureState::Running;
        ledger.insert_structure(handle.clone(), record);
        save(&mut state, ledger);
    }
    assert_eq!(
        next_cell(&state, id, &handle).unwrap(),
        None,
        "an empty stage has no cell"
    );
    assert_eq!(recover(&mut state, id, &handle), Ok(true));
    assert_eq!(
        status(&state, id, &handle).unwrap().state,
        ScriptStructureState::Committed,
        "recovery steps over a planned stage with no cells and finishes the structure"
    );
}

#[test]
fn a_terminal_index_structure_with_empty_cells_is_not_committed_by_recovery() {
    let (mut state, id, _slot) = crate::settlements::construction::tests::fixture();
    crate::settlements::construction::tests::start(&mut state, id, "house", 1, None);
    crate::settlements::construction::tests::fund(&mut state, "house", 1);
    let handle = state.settlements.get(id).unwrap().projects[0]
        .structure
        .clone();
    let plan: Vec<([i32; 3], u32)> = {
        let ledger = ledger(&state).unwrap();
        let record = structure(&ledger, &owner(id), &handle).unwrap();
        record
            .cells
            .iter()
            .flatten()
            .map(|cell| (cell.at, cell.state))
            .collect()
    };
    assert!(!plan.is_empty(), "the house plan has cells");
    for (at, _block) in plan {
        state.world.set_block(at[0], at[1], at[2], 0);
    }
    {
        let mut ledger = ledger_mut(&mut state).unwrap();
        let mut record = take_structure(&mut ledger, &owner(id), &handle).unwrap();
        record.snapshot.current_stage_index = record.snapshot.stages.len() as u32;
        record.snapshot.completed_work_units = 0;
        record.snapshot.state = ScriptStructureState::Running;
        ledger.insert_structure(handle.clone(), record);
        save(&mut state, ledger);
    }
    assert_eq!(recover(&mut state, id, &handle), Ok(true));
    assert_ne!(
        status(&state, id, &handle).unwrap().state,
        ScriptStructureState::Committed,
        "an abandoned site whose blocks were never placed must not be committed: the BuildQA \
         house L1 select of 20261124T221059 lost its project this way"
    );
}
