use super::*;
use crate::settlements::allocator::tests::block;
use crate::settlements::construction::site_tests;
use crate::settlements::construction::tests::{
    complete, fixture, fund, open_plan, plan_until_project, start,
};
use mc_data::ItemStack;
use std::collections::BTreeMap;

fn certificate(state: &mut TickState, id: SettlementId) {
    let record = state.settlements.get_mut(id).unwrap();
    record.building_sequence += 1;
    let building = BuildingId((id.0 << 32) | record.building_sequence);
    record.buildings.insert(
        building,
        buildings::Building {
            id: building,
            settlement: id,
            kind: BuildingKind::Storage,
            level: 1,
            bounds: ScriptWorldBounds::new([30, 200, 30], [32, 203, 32]).unwrap(),
            work_points: Vec::new(),
            revision: 1,
            natural_zone: false,
        },
    );
}

fn saturate(state: &mut TickState) {
    let dirt = crate::villager::mining::item_id(state, "dirt").unwrap();
    for stack in &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots {
        if stack.is_empty() {
            *stack = ItemStack::new(dirt, 64);
        }
    }
}

#[test]
fn storage_pressure_is_strictly_less_than_one_quarter_of_physical_slots() {
    let (mut state, id, _) = fixture();
    certificate(&mut state, id);
    saturate(&mut state);
    let slots = &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots;
    for stack in &mut slots[..7] {
        *stack = ItemStack::EMPTY;
    }
    assert!(!needed(
        &state,
        state.settlements.get(id).unwrap(),
        &executor::source_images(&state, id)
    ));
    let dirt = crate::villager::mining::item_id(&state, "dirt").unwrap();
    state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots[6] = ItemStack::new(dirt, 64);
    assert!(needed(
        &state,
        state.settlements.get(id).unwrap(),
        &executor::source_images(&state, id)
    ));
}

#[test]
fn further_spanish_capacity_uses_the_renewable_base_warehouse() {
    let (mut state, id, _) = fixture();
    let first = autonomous_catalog(&state, state.settlements.get(id).unwrap(), true)
        .find(|blueprint| blueprint.kind == BuildingKind::Storage)
        .unwrap();
    assert_ne!(first.id, "storage");
    certificate(&mut state, id);
    let next = autonomous_catalog(&state, state.settlements.get(id).unwrap(), true)
        .find(|blueprint| blueprint.kind == BuildingKind::Storage)
        .unwrap();
    assert_eq!(next.id, "storage");
}

#[test]
fn one_pending_or_active_storehouse_suppresses_another() {
    for active in [false, true] {
        let (mut state, id, _) = fixture();
        open_plan(&mut state);
        certificate(&mut state, id);
        fund(&mut state, "storage", 1);
        saturate(&mut state);
        if active {
            start(&mut state, id, "storage", 1, None);
        } else {
            request_build(&mut state, id, "storage").unwrap();
        }
        assert!(!needed(
            &state,
            state.settlements.get(id).unwrap(),
            &executor::source_images(&state, id)
        ));
    }
}

#[test]
fn a_later_fill_never_pays_for_the_first_cut_at_a_full_store() {
    let (mut state, id, _) = fixture();
    certificate(&mut state, id);
    open_plan(&mut state);
    saturate(&mut state);
    let record = state.settlements.get(id).unwrap();
    let blueprint = blueprints::get(&state.registry, "storage").unwrap();
    let images = executor::source_images(&state, id);
    let mut capacity = Capacity::for_blueprint(&state, record, blueprint, &images).unwrap();
    let result = candidate_fits(
        &state,
        [12, 200, 12],
        blueprint.footprint(Rotation::from_quarter_turns(0)),
        &[([12, 12], 201), ([13, 12], 195)],
        1,
        None,
        &mut capacity,
    );
    assert_eq!(result, Err(site_refusals::Reason::StorageFull));
}

fn search(state: &TickState, id: SettlementId) -> (FundedSite, u32, u32, u32) {
    let record = state.settlements.get(id).unwrap();
    let blueprint = blueprints::get(&state.registry, "storage").unwrap();
    let images = executor::source_images(state, id);
    let mut sweeps = site_refusals::Sweeps::default();
    let mut cursor = 0;
    let mut examined = 0;
    let mut refused = 0;
    for _ in 0..256 {
        let pass = site_project(
            state,
            record,
            std::iter::once(blueprint),
            &mut sweeps,
            cursor,
            64,
            &images,
        );
        examined += pass.report.examined;
        refused += pass.report.counts[site_refusals::Reason::StorageFull as usize];
        cursor = pass.cursor;
        if let Some(site) = pass.site {
            return (site, examined, refused, pass.report.frontages);
        }
        assert!(
            pass.running,
            "storage search exhausted without a safe alternative"
        );
    }
    panic!("bounded storage sweep never completed");
}

#[test]
fn full_store_skips_the_nearer_flat_tree_lot_once_and_selects_a_clean_one() {
    let (mut state, id, _) = fixture();
    certificate(&mut state, id);
    open_plan(&mut state);
    fund(&mut state, "storage", 1);
    saturate(&mut state);
    let near = site_tests::ranking_lot(&state, id, "storage");
    let tree = [
        near.lot.origin[0] + i32::from(near.size[0]) / 2,
        near.lot.origin[1] + 1,
        near.lot.origin[2] + i32::from(near.size[2]) / 2,
    ];
    let log = block(&state, "minecraft:birch_log", &[]);
    state.world.set_block(tree[0], tree[1], tree[2], log);
    state.world.fold_overlay();
    assert_eq!(
        site_tests::ranking_lot(&state, id, "storage").lot.origin,
        near.lot.origin,
        "without the capacity fence this nearer flat tree lot is cheapest"
    );
    let (site, examined, refused, frontages) = search(&state, id);
    assert_ne!(site.project.origin, near.lot.origin);
    assert!(
        refused > 0,
        "the unsafe candidate must be refused inside the sweep"
    );
    assert_eq!(
        examined, frontages,
        "each frontage is visited once, not restarted after a rejected winner"
    );
    assert!(
        site.cells
            .iter()
            .all(|cell| cell.portion != executor::PortionKind::Trees)
    );
    assert_eq!(
        state.world.block_at(tree[0], tree[1], tree[2]),
        Some(log),
        "admission fells nothing"
    );
}

#[test]
fn full_store_also_refuses_a_tree_on_the_outside_walking_corridor() {
    let (mut state, id, _, project, _, access) = crate::settlements::forestry::tests::forest();
    crate::settlements::forestry::tests::grown_world(&mut state, vec![access], "birch");
    certificate(&mut state, id);
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .planted_trees
        .insert(access);
    fund(&mut state, "storage", 1);
    saturate(&mut state);
    let blueprint = blueprints::get(&state.registry, "storage").unwrap();
    let size = blueprint.footprint(Rotation::from_quarter_turns(project.rotation));
    let (_, preparation) = planned_cells(&state, &project, "oak", &mut Default::default()).unwrap();
    assert!(
        preparation
            .iter()
            .any(|cell| cell.at == access && cell.portion == executor::PortionKind::Trees),
        "the native preparation really fells this outside-lot registered tree"
    );
    let record = state.settlements.get(id).unwrap();
    let images = executor::source_images(&state, id);
    let mut capacity = Capacity::for_blueprint(&state, record, blueprint, &images).unwrap();
    let columns: Vec<_> = (project.origin[0]..project.origin[0] + i32::from(size[0]))
        .flat_map(|x| {
            (project.origin[2]..project.origin[2] + i32::from(size[2]))
                .map(move |z| ([x, z], project.origin[1]))
        })
        .collect();
    assert_eq!(
        candidate_fits(
            &state,
            project.origin,
            size,
            &columns,
            0,
            None,
            &mut capacity
        ),
        Err(site_refusals::Reason::StorageFull)
    );
    let (clean, examined, refused, frontages) = search(&state, id);
    assert_ne!(clean.project.origin, project.origin);
    assert!(
        clean
            .cells
            .iter()
            .all(|cell| cell.portion != executor::PortionKind::Trees)
    );
    assert!(refused > 0);
    assert_eq!(examined, frontages);
}

fn inventory_and_drops(
    state: &TickState,
    id: SettlementId,
    builder: crate::entities::EntitySlot,
) -> BTreeMap<u32, i64> {
    let mut counts = BTreeMap::new();
    for image in executor::source_images(state, id) {
        for stack in image.slots.into_iter().filter(|stack| !stack.is_empty()) {
            *counts.entry(stack.item_id).or_default() += i64::from(stack.count);
        }
    }
    for stack in state
        .entities
        .row(builder)
        .villager
        .as_ref()
        .unwrap()
        .inventory
        .iter()
        .flatten()
    {
        *counts.entry(stack.item_id).or_default() += i64::from(stack.count);
    }
    for slot in state.entities.item_slots() {
        let stack = state.entities.row(slot).item.as_ref().unwrap();
        *counts.entry(stack.item_id).or_default() += i64::from(stack.count);
    }
    counts.retain(|_, count| *count != 0);
    counts
}

#[test]
fn native_autonomy_builds_more_capacity_before_housing_without_losing_or_duplicating_stock() {
    let (mut state, id, builder) = fixture();
    open_plan(&mut state);
    certificate(&mut state, id);
    let bill = fund(&mut state, "storage", 1);
    saturate(&mut state);
    let mut expected = inventory_and_drops(&state, id, builder);
    for (item, count) in bill {
        *expected.get_mut(&item).unwrap() -= i64::from(count);
    }
    expected.retain(|_, count| *count != 0);
    plan_until_project(&mut state, id);
    let project = state.settlements.get(id).unwrap().projects.first().unwrap();
    assert_eq!(
        project.blueprint, "storage",
        "the missing house must not take the emergency slot"
    );
    complete(&mut state, id, builder);
    let stores = state
        .settlements
        .get(id)
        .unwrap()
        .buildings
        .values()
        .filter(|building| building.kind == BuildingKind::Storage)
        .count();
    assert_eq!(stores, 2);
    assert!(
        executor::source_images(&state, id)
            .iter()
            .map(|image| image.slots.len())
            .sum::<usize>()
            > 27
    );
    assert_eq!(
        inventory_and_drops(&state, id, builder),
        expected,
        "physical storage + builder carry + world drops must equal the exact build debit"
    );
}
