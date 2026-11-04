//! Furnace tending must use the native furnace tick and conserve physical stock.
use super::{VillagerProfession, mining, mining_tests};
use crate::boundary::EntityId;
use crate::settlements::buildings::BuildingKind;
use crate::tick::TickState;
use mc_data::ItemStack;
use std::collections::BTreeMap;

const CHEST: (i32, i32, i32) = (4, 201, 3);
const FURNACE: (i32, i32, i32) = (11, 201, 3);

fn setup() -> (TickState, EntityId, EntityId) {
    let (mut state, slot, mut worker) = mining_tests::fixture(BuildingKind::Mine);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla");
    state.registry = std::sync::Arc::new(crate::registry::CoreRegistry::load(Some(&root)).unwrap());
    worker.data.profession = VillagerProfession::None;
    worker.job_site = None;
    worker.inventory = std::array::from_fn(|_| None);
    // The mining fixture spawns inside its job block; this unemployed resident
    // has no workstation and must start in walkable air, as in the disk fixture.
    state.world.set_block(3, 201, 3, state.registry.states.air);
    let town = worker.settlement.unwrap();
    state.settlements.get_mut(town).unwrap().buildings.clear();
    mining_tests::storage(&mut state, &worker, [CHEST.0, CHEST.1, CHEST.2]);
    // The existing certified-storage fixture owns this building's interior.
    state
        .settlements
        .get_mut(town)
        .unwrap()
        .buildings
        .values_mut()
        .next()
        .unwrap()
        .bounds = mc_script::ScriptWorldBounds::new([4, 201, 3], [12, 204, 5]).unwrap();
    state.world.set_block(
        FURNACE.0,
        FURNACE.1,
        FURNACE.2,
        mining_tests::block(&state, "furnace"),
    );
    state.block_entities.insert(
        FURNACE,
        crate::tick::BlockEntityState {
            slots: vec![ItemStack::EMPTY; 3],
            furnace: Some(Default::default()),
        },
    );
    let raw = mining::item_id(&state, "raw_iron").unwrap();
    let coal = mining::item_id(&state, "coal").unwrap();
    let cobble = mining::item_id(&state, "cobblestone").unwrap();
    state.block_entities.get_mut(&CHEST).unwrap().slots[..3].clone_from_slice(&[
        ItemStack::new(raw, 9),
        ItemStack::new(coal, 3),
        ItemStack::new(cobble, 8),
    ]);
    let sheep_type = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse("minecraft:sheep").unwrap())
        .unwrap();
    let sheep = state.spawn_mob(sheep_type, [3.5, 201.0, 4.5], 8.0);
    let sheep_slot = state.entities.slot_of(sheep).unwrap();
    state.entities.row_mut(sheep_slot).controls.no_ai = true;
    crate::tick::set_wool(
        std::sync::Arc::make_mut(&mut state.entities.row_mut(sheep_slot).appearance),
        0,
    );
    let worker_id = state.entities.body(slot).id;
    state.entities.row_mut(slot).villager = Some(worker);
    state.world.fold_overlay();
    crate::settlements::construction::request_build(&mut state, town, "house").unwrap();
    (state, worker_id, sheep)
}

/// Every physical location, including all residents and loose world drops.
fn counts(state: &TickState) -> BTreeMap<u32, i32> {
    let mut counts = BTreeMap::new();
    let mut add = |item: u32, count: i32| {
        if count > 0 {
            *counts.entry(item).or_default() += count;
        }
    };
    for stack in state
        .block_entities
        .values()
        .flat_map(|entity| &entity.slots)
    {
        add(stack.item_id, stack.count);
    }
    for slot in state.entities.slots() {
        if !state.entities.body(slot).alive {
            continue;
        }
        let row = state.entities.row(slot);
        if let Some(worker) = &row.villager {
            for stack in worker.inventory.iter().flatten() {
                add(stack.item_id, stack.count);
            }
        }
        if let Some(drop) = &row.item {
            add(drop.item_id, drop.count);
        }
    }
    counts
}

fn stored_wool(state: &TickState) -> i32 {
    let white = mining::item_id(state, "white_wool").unwrap();
    state.block_entities[&CHEST]
        .slots
        .iter()
        .filter(|s| s.item_id == white)
        .map(|s| s.count)
        .sum()
}

/// Observe newly created wool at shearing, before any subsequent haul can lose
/// or duplicate it. Expected conservation must not be inferred from final stock.
fn shear_yield(state: &TickState, sheep: EntityId) -> Option<i32> {
    let slot = state.entities.slot_of(sheep)?;
    (crate::tick::wool_of(&state.entities.row(slot).appearance) & 0x10 != 0).then(|| {
        let white = mining::item_id(state, "white_wool").unwrap();
        counts(state).get(&white).copied().unwrap_or(0)
    })
}

#[test]
fn native_ticks_smelt_only_needed_iron_then_craft_shears_and_deliver_wool() {
    let (mut state, worker, sheep) = setup();
    let before = counts(&state);
    let mut saw_cooking = false;
    let mut saw_output = false;
    let mut saw_raw_carry = false;
    let mut saw_ingot_delivery = false;
    let mut generated_wool = None;
    for _ in 0..4_000 {
        crate::tick::run_tick(&mut state, &[]);
        let furnace = &state.block_entities[&FURNACE];
        saw_cooking |= furnace.furnace.as_ref().unwrap().cook_progress > 0;
        let iron = mining::item_id(&state, "iron_ingot").unwrap();
        let raw = mining::item_id(&state, "raw_iron").unwrap();
        saw_output |= furnace.slots[2].item_id == iron && furnace.slots[2].count > 0;
        let resident = state
            .entities
            .row(state.entities.slot_of(worker).unwrap())
            .villager
            .as_ref()
            .unwrap();
        saw_raw_carry |= resident
            .inventory
            .iter()
            .flatten()
            .any(|s| s.item_id == raw);
        saw_ingot_delivery |= state.block_entities[&CHEST]
            .slots
            .iter()
            .any(|s| s.item_id == iron && s.count > 0);
        if generated_wool.is_none() {
            generated_wool = shear_yield(&state, sheep);
        }
        if stored_wool(&state) > 0 {
            break;
        }
    }
    assert!(
        stored_wool(&state) > 0,
        "open wool bill must progress through real smelting: worker={:?}, furnace={:?}, counts={:?}",
        state
            .entities
            .row(state.entities.slot_of(worker).unwrap())
            .villager,
        state.block_entities[&FURNACE],
        counts(&state),
    );
    assert!(
        saw_cooking && saw_output && saw_raw_carry && saw_ingot_delivery,
        "must observe real cooking, output, ore carry and stored ingots"
    );
    let sheep_slot = state.entities.slot_of(sheep).unwrap();
    assert_eq!(
        crate::tick::wool_of(&state.entities.row(sheep_slot).appearance),
        0x10
    );
    let wool = generated_wool.expect("native shearing creates an observed wool lot");
    assert_eq!(
        stored_wool(&state),
        wool,
        "the entire sheared lot reaches storage"
    );
    assert!((1..=3).contains(&wool));
    let mut expected = before;
    // Two raw iron -> two ingots -> one shears; one coal burns; sheep yields wool.
    for (name, delta) in [
        ("raw_iron", -2),
        ("coal", -1),
        ("shears", 1),
        ("white_wool", wool),
    ] {
        *expected
            .entry(mining::item_id(&state, name).unwrap())
            .or_default() += delta;
    }
    expected.retain(|_, count| *count > 0);
    assert_eq!(
        counts(&state),
        expected,
        "full stock + carry + furnace + drops conservation"
    );
    let resident = state
        .entities
        .row(state.entities.slot_of(worker).unwrap())
        .villager
        .as_ref()
        .unwrap();
    assert!(
        resident
            .inventory
            .iter()
            .flatten()
            .any(|s| s.item_id == mining::item_id(&state, "shears").unwrap())
    );
}

#[path = "smelting_disk_tests.rs"]
mod disk;

#[path = "smelting_guard_tests.rs"]
mod guards;
