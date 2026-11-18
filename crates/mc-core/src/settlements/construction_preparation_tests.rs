use super::super::tests::{attempt, complete, fund};
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
    let bill = super::super::active_project_bill(&state, id).expect("active bill");
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
