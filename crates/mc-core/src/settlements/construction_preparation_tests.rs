use super::super::tests::{attempt, complete, fund, start};
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
    let project = state.settlements.get(id).unwrap().projects[0].clone();
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
        if state.settlements.get(id).unwrap().projects.is_empty() {
            break;
        }
        attempt(&mut state, id, builder);
    }
    assert!(
        state.settlements.get(id).unwrap().projects.is_empty(),
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
            .orders
            .first()
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
    let renewed = state.settlements.get(id).unwrap().projects[0].clone();
    assert_eq!(renewed.origin, expected.origin);
    // Retiring the superseded manifest returned the builder to its career; the
    // recertified lot is admitted with a working builder again.
    state
        .entities
        .row_mut(builder)
        .villager
        .as_mut()
        .unwrap()
        .data
        .profession = Profession::Builder;
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
        let actual = stock(&state);
        assert_eq!(
            actual, expected_stock,
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
    assert!(state.settlements.get(id).unwrap().projects.is_empty());
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
    fund(&mut state, "storage", 1);
    let record = state.settlements.get(id).unwrap();
    let blueprint = blueprints::get(&state.registry, "storage").unwrap();
    let images = crate::plugin_structures::settlement_builds::source_images(&state, id);
    let before = forestry::proof_builds();
    // One owner scan over the same funded kind examines every candidate with a
    // single scan-wide Clearance: the registered root is proved only once.
    let mut sweeps = site_refusals::Sweeps::default();
    let pass = site_project(
        &state,
        record,
        std::iter::once(blueprint),
        &mut sweeps,
        0,
        u32::MAX,
        &images,
    );
    assert!(pass.report.examined > 1);
    assert_eq!(forestry::proof_builds() - before, 1);
}

/// Add `count` of `name` to the first free slot of the bootstrap chest.
fn add_stock(state: &mut TickState, name: &str, count: i32) {
    let item = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse(name).unwrap())
        .unwrap();
    let slots = &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots;
    let slot = slots
        .iter_mut()
        .find(|slot| slot.is_empty())
        .expect("free chest slot");
    *slot = mc_data::ItemStack::new(item, count);
}

fn raised_lot(id: super::super::SettlementId) -> Project {
    Project {
        settlement: id,
        blueprint: "storage".into(),
        level: 1,
        // Three blocks above the plateau: two fill rows below the authored
        // floor, and still inside the site's height variance.
        origin: [10, 203, 10],
        rotation: 0,
        building: None,
        structure: String::new(),
        ordered: false,
        builder: None,
        order_seq: None,
    }
}

/// Item path of a planned cell's bill, e.g. `dirt`, without an inline id.
fn resource_path(cell: &executor::PlannedCell) -> &str {
    cell.resource
        .as_deref()
        .and_then(|resource| resource.split_once(':'))
        .map_or("", |(_, path)| path)
}

/// The plan's fill cells: billed dirt at or below the authored floor. Authored
/// cells are excluded because their material is the blueprint's own bill.
fn fill_coords(cells: &[executor::PlannedCell], floor: i32) -> Vec<[i32; 3]> {
    cells
        .iter()
        .filter(|cell| cell.at[1] <= floor && resource_path(cell) == "dirt")
        .map(|cell| cell.at)
        .collect()
}

/// The authored bill is reserved before the fill is chosen: a chest holding
/// exactly the walls' cobblestone and no dirt leaves the fill cells as dirt, so
/// the walls can never be starved by the floor's substrate.
#[test]
fn fill_does_not_eat_the_authored_bill() {
    let (mut state, id, _) = super::super::tests::fixture();
    super::super::tests::open_plan(&mut state);
    let project = raised_lot(id);
    super::super::tests::fund(&mut state, "storage", 1);
    let (_, mut cells) = planned_cells(&state, &project, "oak", &mut Default::default()).unwrap();
    let fill = fill_coords(&cells, project.origin[1]);
    assert!(!fill.is_empty(), "a raised floor needs fill");
    let images = executor::source_images(&state, id);
    super::super::filler::apply_filler(&state, &project, &mut cells, &images);
    for at in &fill {
        let cell = cells.iter().find(|cell| cell.at == *at).unwrap();
        assert_eq!(
            resource_path(cell),
            "dirt",
            "the walls' material never becomes fill: {at:?}"
        );
    }
}

/// Stock beyond the authored bill is the only fill candidate: the same lot then
/// charges cobblestone for exactly that remainder, top layer first.
#[test]
fn fill_takes_only_the_remainder_after_the_bill() {
    let (mut state, id, _) = super::super::tests::fixture();
    super::super::tests::open_plan(&mut state);
    let project = raised_lot(id);
    super::super::tests::fund(&mut state, "storage", 1);
    add_stock(&mut state, "cobblestone", 48);
    let (_, mut cells) = planned_cells(&state, &project, "oak", &mut Default::default()).unwrap();
    let fill = fill_coords(&cells, project.origin[1]);
    assert!(fill.len() > 48, "the extra stock fits in the fill");
    let images = executor::source_images(&state, id);
    super::super::filler::apply_filler(&state, &project, &mut cells, &images);
    let filled: Vec<_> = fill
        .iter()
        .filter(|at| {
            cells
                .iter()
                .any(|cell| cell.at == **at && resource_path(cell) == "cobblestone")
        })
        .collect();
    assert_eq!(filled.len(), 48, "only the stock beyond the bill is spent");
    let top = fill.iter().map(|at| at[1]).max().unwrap();
    assert!(
        fill.iter()
            .filter(|at| at[1] == top)
            .all(|at| filled.contains(&at)),
        "the remainder fills from the top layer down"
    );
}

/// Stock, for the filler-relevant materials only, exactly what the authored
/// bill's crafting chain reserves: the stone its bricks and stairs consume.
/// Other bill items cannot become fill, so they are not stocked here.
fn stock_reserved_fillers(state: &mut TickState, blueprint: &str) -> Vec<(u32, u32)> {
    let materials = blueprints::get(&state.registry, blueprint)
        .expect("catalogue blueprint")
        .level(1)
        .expect("level one")
        .materials
        .clone();
    let reserve = super::super::filler::plan_reserve(state, &materials);
    let mut stocked = Vec::new();
    for name in ["stone", "cobblestone"] {
        let Some(item) = mc_data::Identifier::parse(name)
            .ok()
            .and_then(|id| state.registry.items.id_of(&id))
        else {
            continue;
        };
        let count = reserve.get(&item).copied().unwrap_or(0);
        let mut quantity = i32::try_from(count).expect("fits an inventory");
        stocked.push((item, u32::try_from(quantity).unwrap()));
        let slots = &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots;
        let mut index = slots.iter().position(|slot| slot.is_empty()).unwrap_or(0);
        while quantity > 0 {
            let take = quantity.min(64);
            slots[index] = mc_data::ItemStack::new(item, take);
            index += 1;
            quantity -= take;
        }
    }
    stocked
}

/// A Spanish fortress needs stone for its bricks and brick stairs. With the whole
/// brick stone stocked and cobblestone but no dirt, the fill must spend
/// cobblestone and leave that stone alone.
#[test]
fn fortress_fill_spends_cobblestone_and_keeps_the_brick_stone() {
    let (mut state, id, _) = super::super::tests::fixture();
    super::super::tests::open_plan(&mut state);
    let mut project = raised_lot(id);
    project.blueprint = "spanish_fortress".into();
    let stocked = stock_reserved_fillers(&mut state, "spanish_fortress");
    let stone = stocked
        .iter()
        .find(|(item, _)| {
            state
                .registry
                .items
                .name_of(*item)
                .is_some_and(|name| name.path() == "stone")
        })
        .map(|(item, count)| (*item, *count))
        .expect("the brick chain reserves stone");
    assert!(stone.1 > 0, "the chain's stone is not empty");
    let (_, cells) = planned_cells(&state, &project, "oak", &mut Default::default()).unwrap();
    let fill = fill_coords(&cells, project.origin[1]);
    assert!(!fill.is_empty(), "an authored house floor needs fill");
    for _ in 0..fill.len().div_ceil(64) {
        add_stock(&mut state, "cobblestone", 64);
    }
    let images = executor::source_images(&state, id);
    let (_, mut cells) = planned_cells(&state, &project, "oak", &mut Default::default()).unwrap();
    super::super::filler::apply_filler(&state, &project, &mut cells, &images);
    for at in &fill {
        let cell = cells.iter().find(|cell| cell.at == *at).unwrap();
        assert_eq!(
            resource_path(cell),
            "cobblestone",
            "the fill spends the cut's own cobblestone: {at:?}"
        );
    }
    assert!(
        super::super::super::construction_crafting::can_fund(
            &state,
            &[(stone.0, stone.1)],
            &images
        ),
        "every unit of the brick chain's stone is still in storage"
    );
}

/// The reservation, not the material order, is what keeps a short fill off the
/// walls' stone: with half the cobblestone and no dirt the fill stops at dirt —
/// the honest wait — and never bills the brick stone.
#[test]
fn fortress_fill_stops_at_dirt_rather_than_billing_the_brick_stone() {
    let (mut state, id, _) = super::super::tests::fixture();
    super::super::tests::open_plan(&mut state);
    let mut project = raised_lot(id);
    project.blueprint = "spanish_fortress".into();
    // The chain's own stone, and nothing else the reserve would stock.
    let stocked = stock_reserved_fillers(&mut state, "spanish_fortress");
    let stone = stocked
        .iter()
        .find(|(item, _)| {
            state
                .registry
                .items
                .name_of(*item)
                .is_some_and(|name| name.path() == "stone")
        })
        .map(|(item, count)| (*item, *count))
        .expect("the brick chain reserves stone");
    assert!(stone.1 > 0, "the chain's stone is not empty");
    let cobble = mc_data::Identifier::parse("cobblestone")
        .ok()
        .and_then(|id| state.registry.items.id_of(&id))
        .expect("cobblestone item");
    for slot in &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots {
        if !slot.is_empty() && slot.item_id == cobble {
            *slot = mc_data::ItemStack::EMPTY;
        }
    }
    let (_, cells) = planned_cells(&state, &project, "oak", &mut Default::default()).unwrap();
    let fill = fill_coords(&cells, project.origin[1]);
    let half = fill.len() / 2;
    let mut left = half;
    while left > 0 {
        let take = left.min(64);
        add_stock(&mut state, "cobblestone", i32::try_from(take).unwrap());
        left -= take;
    }
    let images = executor::source_images(&state, id);
    let (_, mut cells) = planned_cells(&state, &project, "oak", &mut Default::default()).unwrap();
    super::super::filler::apply_filler(&state, &project, &mut cells, &images);
    let mut cobbled = 0;
    for at in &fill {
        let cell = cells.iter().find(|cell| cell.at == *at).unwrap();
        let path = resource_path(cell);
        assert_ne!(
            path, "stone",
            "the fill never bills the brick stone: {at:?}"
        );
        if path == "cobblestone" {
            cobbled += 1;
        }
    }
    assert_eq!(
        cobbled, half,
        "the fill spends exactly the cobblestone rate"
    );
    let bill = super::super::cell_bill(&state, &cells).expect("bill");
    assert!(
        bill.iter().all(|(item, _)| *item != stone.0),
        "no planned cell bills stone: the brick chain keeps it"
    );
    assert!(
        bill.iter().any(|(item, _)| {
            state
                .registry
                .items
                .name_of(*item)
                .is_some_and(|name| name.path() == "dirt")
        }),
        "the shortfall waits as dirt instead of spending the walls' stone"
    );
}

/// The outpost path (`begin`, which founding calls) reserves the same chain, so
/// an outpost lot cannot spend its own walls' stone either.
#[test]
fn outpost_begin_reserves_the_brick_chain_for_its_fill() {
    let (mut state, id, _) = super::super::tests::fixture();
    super::super::tests::open_plan(&mut state);
    let mut project = raised_lot(id);
    project.blueprint = "spanish_fortress".into();
    let stocked = stock_reserved_fillers(&mut state, "spanish_fortress");
    let stone = stocked
        .iter()
        .find(|(item, _)| {
            state
                .registry
                .items
                .name_of(*item)
                .is_some_and(|name| name.path() == "stone")
        })
        .map(|(item, count)| (*item, *count))
        .expect("the brick chain reserves stone");
    assert!(
        super::super::super::construction_crafting::can_fund(
            &state,
            &[(stone.0, stone.1)],
            &executor::source_images(&state, id)
        ),
        "the chain's stone is stocked before admission"
    );
    assert!(super::super::begin(&mut state, project, "oak", 0));
    let bill = super::super::joint_active_bill(&state, id).expect("active bill");
    let stone_left = bill
        .iter()
        .find(|(item, _)| {
            state
                .registry
                .items
                .name_of(*item)
                .is_some_and(|name| name.path() == "stone")
        })
        .map(|(_, count)| *count)
        .unwrap_or(0);
    assert_eq!(
        stone_left, 0,
        "the admitted outpost bill never asks for stone: the fill kept cobblestone"
    );
}

/// Authored cells of one blueprint level, addressed in the world: the level's
/// own diff, which for level 1 is the whole building.
fn authored(
    state: &TickState,
    project: &Project,
    level: u8,
) -> std::collections::BTreeMap<[i32; 3], u32> {
    let blueprint = blueprints::get(&state.registry, &project.blueprint).unwrap();
    let rotated = blueprint
        .level(level)
        .unwrap()
        .rotated(Rotation::from_quarter_turns(project.rotation));
    rotated
        .blocks
        .iter()
        .map(|(offset, id)| (rotated.world(project.origin, *offset), id.0))
        .collect()
}

fn cleared(
    state: &TickState,
    project: &Project,
    level: u8,
) -> std::collections::BTreeSet<[i32; 3]> {
    let blueprint = blueprints::get(&state.registry, &project.blueprint).unwrap();
    let rotated = blueprint
        .level(level)
        .unwrap()
        .rotated(Rotation::from_quarter_turns(project.rotation));
    rotated
        .removals
        .iter()
        .map(|offset| rotated.world(project.origin, *offset))
        .collect()
}

fn raw(state: &TickState, block: u32) -> i32 {
    state
        .registry
        .blocks
        .by_id(mc_world::BlockStateId(block))
        .expect("known block state")
        .block
        .raw_id
}

/// A level-one cell level two replaces, with level one's state there and a
/// sibling state of the same raw block: a different silhouette or interaction
/// of one and the same block, as an opened door is to a closed one. Tree and
/// sapling cells are D18's fence, not this one, so they are skipped.
fn replaced_cell(state: &TickState, project: &Project) -> ([i32; 3], u32, u32) {
    let previous = authored(state, project, 1);
    for (at, id) in authored(state, project, 2) {
        let Some(&old) = previous.get(&at) else {
            continue;
        };
        if old == id || crate::settlements::forestry::tree_or_sapling(state, old) {
            continue;
        }
        let raw_id = raw(state, old);
        if let Some(sibling) = state
            .registry
            .blocks
            .states()
            .find(|state| state.block.raw_id == raw_id && state.id.0 != old)
        {
            return (at, old, sibling.id.0);
        }
    }
    panic!("level two replaces an authored level-one block with a sibling state");
}

/// A certified level-one house and its level-two project: the upgrade's only
/// evidence is the committed level-one record.
fn certified_house() -> (
    TickState,
    SettlementId,
    crate::entities::EntitySlot,
    Project,
) {
    let (mut state, id, builder) = super::super::tests::fixture();
    start(&mut state, id, "house", 1, None);
    let origin = state.settlements.get(id).unwrap().projects[0].origin;
    fund(&mut state, "house", 1);
    complete(&mut state, id, builder);
    let record = state.settlements.get(id).unwrap();
    assert!(record.projects.is_empty(), "level one certifies");
    let building = *record
        .buildings
        .keys()
        .next()
        .expect("a certified building");
    let project = Project {
        settlement: id,
        blueprint: "house".into(),
        level: 2,
        origin,
        rotation: 0,
        building: Some(building),
        structure: String::new(),
        ordered: true,
        builder: None,
        order_seq: None,
    };
    (state, id, builder, project)
}

fn foreign_block(state: &TickState, different: u32) -> u32 {
    [
        "minecraft:cobblestone",
        "minecraft:stone",
        "minecraft:bricks",
    ]
    .into_iter()
    .map(|name| block(state, name, &[]))
    .find(|id| raw(state, *id) != raw(state, different))
    .expect("a foreign solid block")
}

/// The admission action of every planned placement cell: what the fence, the
/// owner status and the player message all read.
fn actions(
    state: &TickState,
    id: SettlementId,
    project: &Project,
    cells: &[executor::PlannedCell],
) -> Vec<Action> {
    let evidence = executor::previous_committed(state, id, &project.blueprint, project.origin)
        .expect("the committed chain is the evidence");
    placement_actions(state, id, cells, &evidence, project.origin[1])
}

fn refusal_at(state: &TickState, project: &Project) -> Option<site_refusals::Reason> {
    refusal(state, project, "oak").map(|divergence| divergence.reason)
}

#[test]
fn upgrade_refuses_a_divergent_cell_outside_the_territory() {
    let (mut state, id, _, project) = certified_house();
    // Outside the village's territory a foreign block stays someone else's, so
    // the upgrade refuses there (owner D27 keeps the protection outside).
    state.settlements.get_mut(id).unwrap().radius = 1;
    let (at, authored_state, _) = replaced_cell(&state, &project);
    let live = state.world.block_at(at[0], at[1], at[2]).unwrap();
    assert_eq!(
        raw(&state, live),
        raw(&state, authored_state),
        "level one's own block still stands at the replaced cell"
    );
    let (_, cells) = planned_cells(&state, &project, "oak", &mut Default::default()).unwrap();
    assert!(
        cells
            .iter()
            .any(|cell| cell.at == at && cell.state != state.registry.states.air),
        "level two places a block here"
    );
    assert!(
        !actions(&state, id, &project, &cells)
            .iter()
            .any(|action| matches!(action, Action::Refuse(_))),
        "the previous build's own cell is never a divergence"
    );

    let foreign = foreign_block(&state, authored_state);
    state.world.set_block(at[0], at[1], at[2], foreign);
    let divergence = actions(&state, id, &project, &cells)
        .into_iter()
        .find_map(|action| match action {
            Action::Refuse(divergence) => Some(divergence),
            Action::Clear(_) | Action::ClearTerrain(_) => None,
        })
        .expect("a foreign block outside the territory refuses the upgrade");
    assert_eq!(
        (divergence.at, divergence.live),
        (at, Some(foreign)),
        "the refusal names the divergent cell"
    );
    assert_eq!(
        divergence.reason,
        site_refusals::Reason::PreviousBuildDiverged
    );
    assert!(
        matches!(
            planned_cells(&state, &project, "oak", &mut Default::default()),
            Err(site_refusals::Reason::PreviousBuildDiverged)
        ),
        "a foreign block outside the territory refuses the upgrade"
    );

    // Admission, not the executor: no project, no block write, no receipt and
    // no ledger change.
    fund(&mut state, "house", 2);
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    let ledger = state
        .plugins
        .get_domain("@native-structures", "ledger")
        .cloned();
    let [x, y, z] = at;
    assert_eq!(
        start_upgrade(&mut state, id, project.building.unwrap()),
        Err(format!(
            "Construction admission refused: PreviousBuildDiverged at {x},{y},{z}."
        )),
        "the player's message names the cell too"
    );
    assert_eq!(state.world.block_at(at[0], at[1], at[2]), Some(foreign));
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    assert!(
        state.settlements.get(id).unwrap().projects.is_empty(),
        "nothing was admitted"
    );
    assert_eq!(
        state
            .plugins
            .get_domain("@native-structures", "ledger")
            .cloned(),
        ledger,
        "no receipt and no record write before the refusal"
    );
}

#[test]
fn a_fluid_on_a_placement_cell_refuses_at_admission() {
    // A fluid is the executor's liquid work, not village property a cut may
    // remove: admitting it would stall the paid project on `Blocked` mid-plan.
    let (mut state, _, _, project) = certified_house();
    let (at, authored_state, _) = replaced_cell(&state, &project);
    assert_ne!(
        raw(&state, authored_state),
        raw(&state, block(&state, "minecraft:water", &[]))
    );
    let water = block(&state, "minecraft:water", &[("level", "0")]);
    state.world.set_block(at[0], at[1], at[2], water);
    assert_eq!(
        refusal_at(&state, &project),
        Some(site_refusals::Reason::PreviousBuildDiverged),
        "a fluid clear cell refuses instead of a mid-plan Blocked loop"
    );
    assert!(
        matches!(
            planned_cells(&state, &project, "oak", &mut Default::default()),
            Err(site_refusals::Reason::PreviousBuildDiverged)
        ),
        "the plan refuses the fluid cell"
    );
    assert_eq!(state.world.block_at(at[0], at[1], at[2]), Some(water));
}

#[test]
fn a_cleared_door_takes_both_halves_and_credits_one_drop() {
    let (mut state, _, _, project) = certified_house();
    // A door at a placement cell whose cell above is not itself a placement: the
    // second half then comes from the door, never from the plan.
    let (_, plan) = planned_cells(&state, &project, "oak", &mut Default::default()).unwrap();
    let air = state.registry.states.air;
    let placements: std::collections::BTreeSet<[i32; 3]> = plan
        .iter()
        .filter(|cell| cell.state != air)
        .map(|cell| cell.at)
        .collect();
    let at = *placements
        .iter()
        .find(|at| !placements.contains(&[at[0], at[1] + 1, at[2]]))
        .expect("a placement cell with a free cell above it");
    let door = block(&state, "minecraft:oak_door", &[("half", "lower")]);
    let upper = block(&state, "minecraft:oak_door", &[("half", "upper")]);
    state.world.set_block(at[0], at[1], at[2], door);
    state.world.set_block(at[0], at[1] + 1, at[2], upper);
    let (_, cells) = planned_cells(&state, &project, "oak", &mut Default::default())
        .expect("a village door inside the territory is cleared");
    let cut = |target: [i32; 3]| {
        cells
            .iter()
            .position(|cell| cell.at == target && cell.state == air)
            .is_some()
    };
    assert!(cut(at), "the lower half is cleared");
    assert!(
        cut([at[0], at[1] + 1, at[2]]),
        "the upper half is cleared too, though the plan places nothing there"
    );
    // The player's authored block is not double-counted: the door halves are
    // two cells and each is cleared once.
    assert_eq!(
        cells
            .iter()
            .filter(|cell| cell.at == at && cell.state == state.registry.states.air)
            .count(),
        1
    );
}

#[test]
fn every_refusal_reason_keeps_its_array_slot() {
    // `Report` indexes counts and examples by `reason as usize`, so the enum's
    // discriminants and the report's name array must stay in step. The fence's
    // reason is appended last (25th) and must still print itself.
    for reason in [
        site_refusals::Reason::PreviousBuildDiverged,
        site_refusals::Reason::MaterialBill,
    ] {
        let mut report = site_refusals::Report::default();
        report.reject(reason, [4, 5, 6]);
        let text = format!("{report:?}");
        assert!(
            text.contains(&format!("{reason:?}")),
            "{reason:?} must print under its own name: {text}"
        );
        assert!(
            text.contains("(1, Some([4, 5, 6]))"),
            "one rejection, one example: {text}"
        );
    }
}

#[test]
fn upgrade_clears_a_foreign_block_inside_the_territory() {
    let (mut state, id, builder, project) = certified_house();
    let (at, authored_state, _) = replaced_cell(&state, &project);
    let foreign = foreign_block(&state, authored_state);
    // Owner D27: everything standing on the village's territory is the village's
    // property, so a player-built block there is cleared, not refused, and its
    // drop is the village's.
    state.world.set_block(at[0], at[1], at[2], foreign);
    state.block_log = crate::settlements::forestry::tests::player_history(&state, at, 0);
    let (_, cells) = planned_cells(&state, &project, "oak", &mut Default::default())
        .expect("a block inside the village's territory never refuses an upgrade");
    let cut = cells
        .iter()
        .position(|cell| cell.at == at && cell.state == state.registry.states.air)
        .expect("the plan clears the foreign block first");
    let placed = cells
        .iter()
        .position(|cell| cell.at == at && cell.state != state.registry.states.air)
        .expect("and then places its own block");
    assert!(
        cut < placed,
        "the clear cell precedes the placement, so the debit credits it first"
    );
    // Executing the plan clears the foreign block and certifies the upgrade.
    fund(&mut state, "house", 2);
    start(&mut state, id, "house", 2, Some(project.building.unwrap()));
    complete(&mut state, id, builder);
    assert!(
        state.settlements.get(id).unwrap().projects.is_empty(),
        "the upgrade certifies through the cleared cell"
    );
    let planned = cells
        .iter()
        .find(|cell| cell.at == at && cell.state != state.registry.states.air)
        .expect("the placement cell")
        .state;
    assert_eq!(
        state.world.block_at(at[0], at[1], at[2]),
        Some(planned),
        "the plan's own block stands where the village's property was cleared"
    );
}

#[test]
fn a_state_only_change_is_not_a_previous_build_divergence() {
    let (mut state, id, _, project) = certified_house();
    let (at, authored_state, sibling) = replaced_cell(&state, &project);
    state.world.set_block(at[0], at[1], at[2], sibling);
    assert_eq!(
        raw(&state, sibling),
        raw(&state, authored_state),
        "the sibling changes only the state, never the block"
    );
    let (_, cells) = planned_cells(&state, &project, "oak", &mut Default::default()).expect(
        "a stair reshaped, a door opened, a fence connected or a furnace lit is still the same block",
    );
    assert!(cells.iter().any(|cell| cell.at == at));
    assert!(
        !actions(&state, id, &project, &cells)
            .iter()
            .any(|action| matches!(action, Action::Refuse(_))),
        "the building's own block is placed as planned"
    );
    // Even outside the territory, a state-only change is the building's own.
    state.settlements.get_mut(id).unwrap().radius = 1;
    assert_eq!(refusal_at(&state, &project), None);

    let foreign = foreign_block(&state, authored_state);
    state.world.set_block(at[0], at[1], at[2], foreign);
    assert_eq!(
        refusal_at(&state, &project),
        Some(site_refusals::Reason::PreviousBuildDiverged),
        "a different raw block in the same cell is a divergence outside the territory"
    );
    // Inside the territory it is village property: cleared, not refused (D27).
    state.settlements.get_mut(id).unwrap().radius = 48;
    assert_eq!(refusal_at(&state, &project), None);
}

#[test]
fn an_earlier_levels_untouched_cell_is_upgrade_evidence() {
    let (mut state, id, builder, project) = certified_house();
    // Level two certifies on top of level one.
    fund(&mut state, "house", 2);
    start(&mut state, id, "house", 2, Some(project.building.unwrap()));
    complete(&mut state, id, builder);
    assert!(
        state.settlements.get(id).unwrap().projects.is_empty(),
        "level two certifies"
    );
    let third = Project {
        level: 3,
        ..project.clone()
    };
    // Level three replaces a cell only level one ever wrote: level two's diff
    // and removals both leave it alone, so newest-only evidence has nothing.
    let first = authored(&state, &third, 1);
    let second = authored(&state, &third, 2);
    let second_cleared = cleared(&state, &third, 2);
    let at = authored(&state, &third, 3)
        .into_iter()
        .find(|(at, id)| {
            first
                .get(at)
                .is_some_and(|old| old != id && !clearable(&state, *old))
                && !second.contains_key(at)
                && !second_cleared.contains(at)
        })
        .map(|(at, _)| at)
        .expect("level three replaces a cell level two never wrote");
    let live = state.world.block_at(at[0], at[1], at[2]).unwrap();
    assert_eq!(
        raw(&state, live),
        raw(&state, first[&at]),
        "level one's block still stands where level two wrote nothing"
    );
    let evidence = executor::previous_committed(&state, id, &third.blueprint, third.origin)
        .expect("the committed chain is the evidence");
    assert_eq!(
        raw(&state, evidence[&at]),
        raw(&state, live),
        "the whole executed chain, not the newest record, proves the cell"
    );
    let (_, cells) = planned_cells(&state, &third, "oak", &mut Default::default())
        .expect("an earlier level's authored cell is the building's own");
    assert!(
        cells.iter().any(|cell| cell.at == at),
        "level three places its block on level one's cell"
    );
}

#[test]
fn committed_records_are_upgrade_evidence_after_a_journal_replay() {
    use crate::journal::JournalEntry;
    let (mut state, id, _, project) = certified_house();
    let (at, _, _) = replaced_cell(&state, &project);
    assert!(executor::built_at(
        &state,
        id,
        &project.blueprint,
        project.origin
    ));
    let evidence = executor::previous_committed(&state, id, &project.blueprint, project.origin)
        .expect("evidence before the cycle");
    assert!(evidence.contains_key(&at));

    let mut entries = Vec::new();
    crate::plugin::journal(&mut state, &mut entries);
    let JournalEntry::Plugins { value } = entries.pop().expect("the ledger's post-image") else {
        panic!("the plugin domain journal entry")
    };
    let (mut replayed, again, _) = super::super::tests::fixture();
    assert_eq!(again, id, "the fixture settles the same settlement");
    replayed.plugins.apply_delta(&value);

    assert!(
        executor::built_at(&replayed, id, &project.blueprint, project.origin),
        "the replayed committed record still certifies the site"
    );
    assert_eq!(
        executor::previous_committed(&replayed, id, &project.blueprint, project.origin)
            .expect("evidence after the replay"),
        evidence,
        "the prepared-cell lookup resolves the same image from persisted evidence"
    );

    // A cancelled record is not evidence.
    executor::cancel(&mut replayed, id).expect("the settlement's records cancel");
    assert!(
        executor::previous_committed(&replayed, id, &project.blueprint, project.origin)
            .expect("a cancelled ledger still reads")
            .is_empty(),
        "only committed records are upgrade evidence"
    );
}

#[test]
fn a_later_levels_last_write_wins_in_the_fold() {
    let (mut state, id, builder, project) = certified_house();
    let (replaced, first_state, _) = replaced_cell(&state, &project);
    fund(&mut state, "house", 2);
    start(&mut state, id, "house", 2, Some(project.building.unwrap()));
    complete(&mut state, id, builder);
    let live = state
        .world
        .block_at(replaced[0], replaced[1], replaced[2])
        .unwrap();
    assert_ne!(
        raw(&state, live),
        raw(&state, first_state),
        "level two replaced level one's block"
    );
    let evidence = executor::previous_committed(&state, id, &project.blueprint, project.origin)
        .expect("the committed chain is the evidence");
    assert_eq!(
        raw(&state, evidence[&replaced]),
        raw(&state, live),
        "the fold keeps level two's write at a cell both levels wrote, not level one's"
    );
    // Only one record writes any position inside a record: `prepare` partitions
    // the plan into the clear and build stages, and a repair stage re-places a
    // build cell with its own state, so a level's own stages never conflict.
}

#[test]
fn an_unloaded_placement_cell_refuses_as_unloaded() {
    let (state, id, _, project) = certified_house();
    let far = [30_000, project.origin[1] + 3, 30_000];
    let cells = [executor::PlannedCell {
        portion: executor::PortionKind::Build,
        at: far,
        state: state.registry.states.stone,
        resource: None,
        quantity: 0,
    }];
    let divergence = actions(&state, id, &project, &cells)
        .into_iter()
        .find_map(|action| match action {
            Action::Refuse(divergence) => Some(divergence),
            Action::Clear(_) | Action::ClearTerrain(_) => None,
        })
        .expect("a cell in an unloaded chunk is a refusal");
    assert_eq!(divergence.reason, site_refusals::Reason::Unloaded);
    assert_eq!((divergence.at, divergence.live), (far, None));
}

#[test]
fn the_foundation_grant_keeps_the_site_searches_vetoes_outside_the_territory() {
    let (mut state, id, _, project) = certified_house();
    // Outside the village's territory the grant is the only way a level's wider
    // floor may replace its own ground, and the site search's vetoes still hold.
    state.settlements.get_mut(id).unwrap().radius = 1;
    let floor = project.origin[1];
    // One of the level's new width columns, on the previous build's own fill: the
    // level's wider floor replaces the dirt level one filled there, and grass has
    // grown over it since.
    let at = [project.origin[0] + 9, floor, project.origin[2] + 3];
    let dirt = state.registry.states.dirt;
    let grass = block(&state, "minecraft:grass_block", &[("snowy", "false")]);
    let planks = block(&state, "minecraft:oak_planks", &[]);
    let mut evidence = std::collections::BTreeMap::new();
    evidence.insert(at, dirt);
    let cells = [executor::PlannedCell {
        portion: executor::PortionKind::Build,
        at,
        state: planks,
        resource: None,
        quantity: 0,
    }];
    let divergence = |state: &TickState| {
        placement_actions(state, id, &cells, &evidence, floor)
            .into_iter()
            .find_map(|action| match action {
                Action::Refuse(divergence) => Some(divergence.reason),
                Action::Clear(_) | Action::ClearTerrain(_) => None,
            })
    };
    state.world.set_block(at[0], at[1], at[2], grass);
    assert_eq!(
        divergence(&state),
        None,
        "grass over the previous build's own foundation fill is the site's ground"
    );

    // A container is never overwritten: the executor refuses a stocked one only
    // after the project is paid for, so admission has to refuse here.
    let chest = block(&state, "minecraft:chest", &[]);
    let stack = |state: &TickState, name: &str, count: i32| {
        let item = state
            .registry
            .items
            .id_of(&mc_data::Identifier::parse(name).unwrap())
            .unwrap();
        mc_data::ItemStack::new(item, count)
    };
    for filled in [false, true] {
        state.world.set_block(at[0], at[1], at[2], chest);
        let mut slots = vec![mc_data::ItemStack::EMPTY; 27];
        if filled {
            slots[0] = stack(&state, "minecraft:cobblestone", 3);
        }
        state.block_entities.insert(
            (at[0], at[1], at[2]),
            crate::tick::BlockEntityState {
                slots,
                furnace: None,
            },
        );
        assert_eq!(
            divergence(&state),
            Some(site_refusals::Reason::PreviousBuildDiverged),
            "a foreign container on the foundation row refuses (filled: {filled})"
        );
    }
    // Inside the territory it still refuses: clearing a container would lose its
    // contents, which no drop credit covers.
    state.settlements.get_mut(id).unwrap().radius = 48;
    assert_eq!(
        divergence(&state),
        Some(site_refusals::Reason::PreviousBuildDiverged),
        "a container refuses inside the territory too"
    );
    state.settlements.get_mut(id).unwrap().radius = 1;
    state.block_entities.remove(&(at[0], at[1], at[2]));

    // A ground the site search itself refuses is not granted either.
    let path = block(&state, "minecraft:dirt_path", &[]);
    state.world.set_block(at[0], at[1], at[2], path);
    assert_eq!(
        divergence(&state),
        Some(site_refusals::Reason::PreviousBuildDiverged),
        "a floor the site search refuses is not granted"
    );

    // A retained player edit stays protected while its history is held.
    state.world.set_block(at[0], at[1], at[2], grass);
    state.block_log = crate::settlements::forestry::tests::player_history(&state, at, 0);
    assert!(
        state.block_log.player_touched(at),
        "the fixture's player edit is live"
    );
    assert_eq!(
        divergence(&state),
        Some(site_refusals::Reason::PreviousBuildDiverged),
        "a player edit on the foundation row refuses while its history is held"
    );

    // The public admission still refuses as well, with the same cell.
    assert!(matches!(
        planned_cells(&state, &project, "oak", &mut Default::default()),
        Err(site_refusals::Reason::PreviousBuildDiverged)
    ));
}

#[test]
fn a_queued_upgrade_reports_the_cell_and_recovers_when_it_is_cleared() {
    let (mut state, id, _, project) = certified_house();
    // Outside the territory the divergent cell refuses, so the queue status has a
    // refusal to name (inside it would simply be cleared, D27).
    state.settlements.get_mut(id).unwrap().radius = 1;
    let (at, authored_state, _) = replaced_cell(&state, &project);
    let foreign = foreign_block(&state, authored_state);
    state.world.set_block(at[0], at[1], at[2], foreign);
    fund(&mut state, "house", 2);
    let building = project.building.unwrap();
    access::push_order(
        state.settlements.get_mut(id).unwrap(),
        OwnerOrder {
            seq: 1,
            blueprint: "house".into(),
            level: 2,
            building: Some(building),
        },
    );
    let [x, y, z] = at;
    assert_eq!(
        pending_reason_at(&state, id, 0),
        Some(format!("previous_build_diverged at {x},{y},{z}")),
        "the owner status names the cell instead of claiming ready"
    );
    // Clearing the obstacle makes the same queue slot ready again: the refusal is
    // a read-only probe, not a stored lifecycle.
    state.world.set_block(at[0], at[1], at[2], authored_state);
    assert_eq!(pending_reason_at(&state, id, 0), Some("ready".into()));
    // And no refusal was recorded for a queued request that never failed.
    assert!(
        state.settlements.order_refusal.is_empty(),
        "no notification is stored until the queue actually retries and fails"
    );
}

/// Every item the village holds at the end of a build: chest storage plus what
/// the builder still carries, so a credit is counted wherever it landed.
fn holdings(
    state: &TickState,
    builder: crate::entities::EntitySlot,
) -> std::collections::BTreeMap<u32, i32> {
    let mut counts = stock(state);
    if let Some(villager) = state.entities.row(builder).villager.as_ref() {
        for stack in villager.inventory.iter().flatten() {
            *counts.entry(stack.item_id).or_default() += stack.count;
        }
    }
    counts
}

/// A completed level-two house, for the control half of a credit comparison.
fn completed_upgrade() -> (
    crate::entities::EntitySlot,
    std::collections::BTreeMap<u32, i32>,
) {
    let (mut state, id, builder, project) = certified_house();
    fund(&mut state, "house", 2);
    start(&mut state, id, "house", 2, Some(project.building.unwrap()));
    complete(&mut state, id, builder);
    assert!(
        state.settlements.get(id).unwrap().projects.is_empty(),
        "the control upgrade certifies"
    );
    (builder, holdings(&state, builder))
}

#[test]
fn upgrade_credits_player_blocks_once_and_air_nothing() {
    // Control: the same house and bill with nothing in the cells.
    let (_, control) = completed_upgrade();

    // Variant: a player-built block on three placement cells inside the
    // territory (owner D27: village property, cleared and credited), and one
    // placement cell that is already air (credits nothing).
    let (mut state, id, builder, project) = certified_house();
    let air = state.registry.states.air;
    let (_, plan) = planned_cells(&state, &project, "oak", &mut Default::default()).unwrap();
    let mut placements: Vec<[i32; 3]> = plan
        .iter()
        .filter(|cell| cell.state != air && cell.at[1] > project.origin[1])
        .map(|cell| cell.at)
        .collect();
    placements.dedup();
    let mut empty_cell = None;
    let mut chosen = Vec::new();
    for at in placements {
        let live = state.world.block_at(at[0], at[1], at[2]).unwrap();
        if live == air {
            empty_cell.get_or_insert(at);
            continue;
        }
        if chosen.len() < 3 {
            chosen.push(at);
        }
    }
    assert_eq!(chosen.len(), 3, "three placement cells to fill");
    let names = [
        "minecraft:spruce_planks",
        "minecraft:cobblestone",
        "minecraft:glass",
    ];
    let mut placed = Vec::new();
    for (at, name) in chosen.iter().zip(names) {
        let block = block(&state, name, &[]);
        state.world.set_block(at[0], at[1], at[2], block);
        state.block_log = crate::settlements::forestry::tests::player_history(&state, *at, 0);
        placed.push((*at, block));
    }
    let empty = empty_cell.expect("a placement cell that is already air");
    // The last cell's history survives the reassignment; all three are the same
    // player-authored kind.
    assert!(state.block_log.player_touched(chosen[2]));

    fund(&mut state, "house", 2);
    start(&mut state, id, "house", 2, Some(project.building.unwrap()));
    complete(&mut state, id, builder);
    assert!(
        state.settlements.get(id).unwrap().projects.is_empty(),
        "the upgrade certifies through the player's blocks"
    );
    let after = holdings(&state, builder);
    for (at, block) in placed {
        // Exactly the block's own drop, once: the variant minus the control is
        // the credit, because the build consumes and places the same bill.
        for drop in crate::settlements::forestry::roll(&state, at, block).unwrap() {
            let name = state
                .registry
                .items
                .name_of(drop.item_id)
                .map_or_else(|| "?".into(), |name| name.to_string());
            let got = after.get(&drop.item_id).copied().unwrap_or(0)
                - control.get(&drop.item_id).copied().unwrap_or(0);
            assert_eq!(
                got, drop.count,
                "the cleared block's drop reaches the village exactly once: {name} x{} (got {got})",
                drop.count
            );
        }
    }
    // Nothing was mined out of the air cell: no item appears for it.
    assert_eq!(
        state.world.block_at(empty[0], empty[1], empty[2]),
        Some(
            plan.iter()
                .filter(|cell| cell.at == empty && cell.state != air)
                .map(|cell| cell.state)
                .next()
                .expect("the air cell is a placement")
        ),
        "the already-air cell is simply built on"
    );
}

#[test]
fn player_terrain_inside_is_cleared_without_income() {
    // Owner D27-4: the village clears its own ground set to build and pays
    // nothing for it. A player's dirt, grass or coarse dirt on a placement cell
    // inside the territory is one of those clears: the block goes, no dirt item
    // is credited, and the upgrade still certifies.
    let (_, control) = completed_upgrade();
    for name in [
        "minecraft:dirt",
        "minecraft:grass_block",
        "minecraft:coarse_dirt",
    ] {
        let (mut state, id, builder, project) = certified_house();
        let (at, authored_state, _) = replaced_cell(&state, &project);
        let ground = block(&state, name, &[]);
        assert_ne!(raw(&state, ground), raw(&state, authored_state), "{name}");
        state.world.set_block(at[0], at[1], at[2], ground);
        state.block_log = crate::settlements::forestry::tests::player_history(&state, at, 0);
        assert_eq!(
            refusal_at(&state, &project),
            None,
            "{name} inside the territory is a village clear, not a divergence"
        );
        fund(&mut state, "house", 2);
        start(&mut state, id, "house", 2, Some(project.building.unwrap()));
        let handle = state.settlements.get(id).unwrap().projects[0]
            .structure
            .clone();
        let mid = holdings(&state, builder);
        complete(&mut state, id, builder);
        assert!(
            state.settlements.get(id).unwrap().projects.is_empty(),
            "{name}: the upgrade certifies through its own ground"
        );
        assert_ne!(
            state.world.block_at(at[0], at[1], at[2]),
            Some(ground),
            "{name}: the ground is cleared for the build"
        );
        // No income for the ground: the exact delta of its own drop against the
        // control run is zero.
        let after = holdings(&state, builder);
        for drop in crate::settlements::forestry::roll(&state, at, ground).unwrap() {
            let got = after.get(&drop.item_id).copied().unwrap_or(0)
                - control.get(&drop.item_id).copied().unwrap_or(0);
            assert_eq!(
                got, 0,
                "{name} pays nothing for the ground it clears (got {got})"
            );
        }
        // Retrying the completed project pays nothing a second time.
        assert!(!executor::reopen_emptied(&mut state, id, &handle).unwrap());
        assert_eq!(holdings(&state, builder), after, "{name}: no second effect");
        assert_eq!(
            holdings(&state, builder)
                .get(
                    &state
                        .registry
                        .items
                        .id_of(&mc_data::Identifier::parse("dirt").unwrap())
                        .unwrap()
                )
                .copied()
                .unwrap_or(0),
            control
                .get(
                    &state
                        .registry
                        .items
                        .id_of(&mc_data::Identifier::parse("dirt").unwrap())
                        .unwrap()
                )
                .copied()
                .unwrap_or(0),
            "{name}: the village still holds exactly the control's dirt"
        );
        let _ = mid;
    }
}

#[test]
fn a_completed_upgrade_is_not_credited_twice_by_reopen() {
    let (mut state, id, builder, project) = certified_house();
    let (at, authored_state, _) = replaced_cell(&state, &project);
    let foreign = block(&state, "minecraft:bricks", &[]);
    assert_ne!(raw(&state, foreign), raw(&state, authored_state));
    state.world.set_block(at[0], at[1], at[2], foreign);
    fund(&mut state, "house", 2);
    start(&mut state, id, "house", 2, Some(project.building.unwrap()));
    let handle = state.settlements.get(id).unwrap().projects[0]
        .structure
        .clone();
    complete(&mut state, id, builder);
    let after = holdings(&state, builder);
    // Reopening a completed record queues nothing: every authored cell is in
    // place, and no clear cell is ever re-queued, so no drop is credited twice.
    assert!(!executor::reopen_emptied(&mut state, id, &handle).unwrap());
    assert_eq!(holdings(&state, builder), after, "no second credit");
    // And a plan rebuilt now carries no clear cell for an already-clear cell.
    let (_, cells) = planned_cells(&state, &project, "oak", &mut Default::default()).unwrap();
    assert!(
        !cells
            .iter()
            .any(|cell| cell.at == at && cell.state == state.registry.states.air),
        "the cleared cell is not planned as work again"
    );
}
