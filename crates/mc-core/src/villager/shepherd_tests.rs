use super::mining_tests::{fixture, storage};
use super::{VillagerProfession, VillagerState, mining, shepherd};
use crate::boundary::ChangeSet;
use crate::entities::EntitySlot;
use crate::settlements::buildings::BuildingKind;
use crate::tick::TickState;
use mc_data::ItemStack;

fn setup() -> (TickState, EntitySlot, Box<VillagerState>) {
    let (state, slot, mut v) = fixture(BuildingKind::Mine);
    v.data.profession = VillagerProfession::Shepherd;
    let shears = mining::item_id(&state, "shears").unwrap();
    v.inventory[0] = Some(ItemStack::new(shears, 1));
    (state, slot, v)
}

fn sheep(state: &mut TickState, pos: [f64; 3], wool: u8) -> EntitySlot {
    let type_id = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse("minecraft:sheep").unwrap())
        .unwrap();
    let id = state.spawn_mob(type_id, pos, 8.0);
    let slot = state.entities.slot_of(id).unwrap();
    crate::tick::set_wool(
        std::sync::Arc::make_mut(&mut state.entities.row_mut(slot).appearance),
        wool,
    );
    slot
}

fn carried(v: &VillagerState, item: u32) -> i32 {
    v.inventory
        .iter()
        .flatten()
        .filter(|stack| stack.item_id == item)
        .map(|stack| stack.count)
        .sum()
}

#[test]
fn unemployed_resident_never_shears_without_tool_or_iron() {
    let (mut state, slot, mut v) = setup();
    v.data.profession = VillagerProfession::None;
    v.inventory = std::array::from_fn(|_| None);
    let sheep_slot = sheep(&mut state, [3.5, 201.0, 3.5], 0);
    state.tick = 20;
    shepherd::work(&mut state, slot, &mut v, &mut ChangeSet::default());
    assert_eq!(
        crate::tick::wool_of(&state.entities.row(sheep_slot).appearance),
        0
    );
    assert!(v.inventory.iter().all(Option::is_none));
}

#[test]
fn unemployed_worker_crafts_shears_from_stock_and_deposits_real_wool() {
    let (mut state, slot, mut v) = setup();
    v.data.profession = VillagerProfession::None;
    v.job_site = None;
    v.inventory = std::array::from_fn(|_| None);
    storage(&mut state, &v, [4, 201, 3]);
    let iron = mining::item_id(&state, "iron_ingot").unwrap();
    let shears = mining::item_id(&state, "shears").unwrap();
    let white = mining::item_id(&state, "white_wool").unwrap();
    state.block_entities.get_mut(&(4, 201, 3)).unwrap().slots[0] = ItemStack::new(iron, 2);
    let sheep_slot = sheep(&mut state, [3.5, 201.0, 3.5], 0);
    state.entities.set_pos(slot, [3.5, 201.0, 3.5]);
    let mut changes = ChangeSet::default();
    state.tick = 20;
    for _ in 0..5 {
        shepherd::work(&mut state, slot, &mut v, &mut changes);
    }
    assert_eq!(
        crate::tick::wool_of(&state.entities.row(sheep_slot).appearance),
        0x10
    );
    assert_eq!(carried(&v, shears), 1);
    assert_eq!(carried(&v, iron), 0);
    let stock = &state.block_entities[&(4, 201, 3)].slots;
    assert_eq!(
        stock
            .iter()
            .filter(|item| item.item_id == iron)
            .map(|item| item.count)
            .sum::<i32>(),
        0
    );
    assert!(
        stock
            .iter()
            .any(|item| item.item_id == white && item.count > 0)
    );
}

#[test]
fn allocator_and_brain_allow_unemployed_resident_to_shear_and_store_wool() {
    let (mut state, slot, mut v) = setup();
    v.data.profession = VillagerProfession::None;
    v.job_site = None;
    v.inventory = std::array::from_fn(|_| None);
    state
        .settlements
        .get_mut(v.settlement.unwrap())
        .unwrap()
        .buildings
        .clear();
    storage(&mut state, &v, [4, 201, 3]);
    let shears = mining::item_id(&state, "shears").unwrap();
    let white = mining::item_id(&state, "white_wool").unwrap();
    state.block_entities.get_mut(&(4, 201, 3)).unwrap().slots[0] = ItemStack::new(shears, 1);
    let sheep_slot = sheep(&mut state, [3.5, 201.0, 3.5], 0);
    state.entities.set_pos(slot, [3.5, 201.0, 3.5]);
    state.entities.row_mut(slot).villager = Some(v);
    state.world.fold_overlay();
    for _ in 0..100 {
        state.tick += 20;
        state.tick_budget.reset(state.tick);
        let mut changes = ChangeSet::default();
        crate::settlements::allocator::tick(&mut state, &mut changes);
        crate::villager::brain::tick(&mut state, &mut changes);
        if state.block_entities[&(4, 201, 3)]
            .slots
            .iter()
            .any(|item| item.item_id == white && item.count > 0)
        {
            break;
        }
    }
    assert_eq!(
        crate::tick::wool_of(&state.entities.row(sheep_slot).appearance),
        0x10
    );
    assert!(
        state.block_entities[&(4, 201, 3)]
            .slots
            .iter()
            .any(|item| item.item_id == white && item.count > 0)
    );
    assert_eq!(
        state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .unwrap()
            .data
            .profession,
        VillagerProfession::None
    );
}

#[test]
fn miner_brain_crafts_shears_and_delivers_wool_for_house_bill() {
    let (mut state, slot, mut v) = setup();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla");
    state.registry = std::sync::Arc::new(crate::registry::CoreRegistry::load(Some(&root)).unwrap());
    v.data.profession = VillagerProfession::Miner;
    v.job_site = None;
    v.inventory = std::array::from_fn(|_| None);
    let town = v.settlement.unwrap();
    state.settlements.get_mut(town).unwrap().buildings.clear();
    storage(&mut state, &v, [4, 201, 3]);
    let iron = mining::item_id(&state, "iron_ingot").unwrap();
    let shears = mining::item_id(&state, "shears").unwrap();
    let white = mining::item_id(&state, "white_wool").unwrap();
    state.block_entities.get_mut(&(4, 201, 3)).unwrap().slots[0] = ItemStack::new(iron, 2);
    let sheep_slot = sheep(&mut state, [3.5, 201.0, 4.5], 0);
    state.entities.set_pos(slot, [3.5, 201.0, 3.5]);
    state.world.set_block(3, 201, 3, state.registry.states.air);
    state.entities.row_mut(slot).villager = Some(v);
    state.world.fold_overlay();
    crate::settlements::construction::request_build(&mut state, town, "house").unwrap();
    for _ in 0..2_000 {
        crate::tick::run_tick(&mut state, &[]);
        if state.block_entities[&(4, 201, 3)]
            .slots
            .iter()
            .any(|item| item.item_id == white && item.count > 0)
        {
            break;
        }
    }
    assert_eq!(
        crate::tick::wool_of(&state.entities.row(sheep_slot).appearance),
        0x10
    );
    let worker = state.entities.row(slot).villager.as_ref().unwrap();
    assert_eq!(worker.data.profession, VillagerProfession::Miner);
    assert_eq!(carried(worker, shears), 1);
    assert_eq!(carried(worker, iron), 0);
    let stock = &state.block_entities[&(4, 201, 3)].slots;
    assert!(
        !stock
            .iter()
            .any(|item| item.item_id == iron && item.count > 0)
    );
    assert!(
        stock
            .iter()
            .any(|item| item.item_id == white && (1..=3).contains(&item.count))
    );
}

#[test]
fn miner_native_ticks_craft_shears_and_deliver_wool() {
    let (mut state, slot, mut v) = setup();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla");
    state.registry = std::sync::Arc::new(crate::registry::CoreRegistry::load(Some(&root)).unwrap());
    v.data.profession = VillagerProfession::Miner;
    v.job_site = None;
    v.inventory = std::array::from_fn(|_| None);
    let town = v.settlement.unwrap();
    state.settlements.get_mut(town).unwrap().buildings.clear();
    storage(&mut state, &v, [4, 201, 3]);
    let iron = mining::item_id(&state, "iron_ingot").unwrap();
    let shears = mining::item_id(&state, "shears").unwrap();
    let wool = mining::item_id(&state, "white_wool").unwrap();
    state.block_entities.get_mut(&(4, 201, 3)).unwrap().slots[0] = ItemStack::new(iron, 2);
    let sheep_slot = sheep(&mut state, [3.5, 201.0, 4.5], 0);
    state.entities.row_mut(sheep_slot).controls.no_ai = true;
    state.world.set_block(3, 201, 3, state.registry.states.air);
    state.entities.row_mut(slot).villager = Some(v);
    state.world.fold_overlay();
    crate::settlements::construction::request_build(&mut state, town, "house").unwrap();
    for _ in 0..2_000 {
        crate::tick::run_tick(&mut state, &[]);
        if state.block_entities[&(4, 201, 3)]
            .slots
            .iter()
            .any(|item| item.item_id == wool && item.count > 0)
        {
            break;
        }
    }
    assert_eq!(
        crate::tick::wool_of(&state.entities.row(sheep_slot).appearance),
        0x10
    );
    let worker = state.entities.row(slot).villager.as_ref().unwrap();
    assert_eq!(worker.data.profession, VillagerProfession::Miner);
    assert_eq!(carried(worker, shears), 1);
    assert_eq!(carried(worker, iron), 0);
    let stock = &state.block_entities[&(4, 201, 3)].slots;
    assert!(
        !stock
            .iter()
            .any(|item| item.item_id == iron && item.count > 0)
    );
    assert!(
        stock
            .iter()
            .any(|item| item.item_id == wool && (1..=3).contains(&item.count))
    );
}

#[test]
fn working_miner_crafts_shears_and_delivers_wool_during_house_demand() {
    let (mut state, slot, mut v) = fixture(BuildingKind::Mine);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla");
    state.registry = std::sync::Arc::new(crate::registry::CoreRegistry::load(Some(&root)).unwrap());
    v.inventory = std::array::from_fn(|_| None);
    let town = v.settlement.unwrap();
    storage(&mut state, &v, [4, 201, 3]);
    let iron = mining::item_id(&state, "iron_ingot").unwrap();
    let shears = mining::item_id(&state, "shears").unwrap();
    let wool = mining::item_id(&state, "white_wool").unwrap();
    state.block_entities.get_mut(&(4, 201, 3)).unwrap().slots[0] = ItemStack::new(iron, 2);
    let sheep_slot = sheep(&mut state, [3.5, 201.0, 4.5], 0);
    state.entities.row_mut(sheep_slot).controls.no_ai = true;
    state.entities.set_pos(slot, [2.5, 201.0, 3.5]);
    state.entities.row_mut(slot).villager = Some(v);
    state.world.fold_overlay();
    crate::settlements::construction::request_build(&mut state, town, "house").unwrap();
    for _ in 0..4_000 {
        crate::tick::run_tick(&mut state, &[]);
        if state.block_entities[&(4, 201, 3)]
            .slots
            .iter()
            .any(|item| item.item_id == wool && item.count > 0)
        {
            break;
        }
    }
    let worker = state.entities.row(slot).villager.as_ref().unwrap();
    assert_eq!(worker.data.profession, VillagerProfession::Miner);
    assert_ne!(
        worker.mining_progress.building, 0,
        "the certified mine was worked"
    );
    assert_eq!(
        crate::tick::wool_of(&state.entities.row(sheep_slot).appearance),
        0x10
    );
    assert_eq!(carried(worker, shears), 1);
    assert_eq!(carried(worker, iron), 0);
    let stock = &state.block_entities[&(4, 201, 3)].slots;
    assert!(
        !stock
            .iter()
            .any(|item| item.item_id == iron && item.count > 0)
    );
    assert!(
        stock
            .iter()
            .any(|item| item.item_id == wool && (1..=3).contains(&item.count))
    );
}

#[test]
fn active_miner_job_takes_precedence_over_shearing() {
    let (mut state, slot, mut v) = setup();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla");
    state.registry = std::sync::Arc::new(crate::registry::CoreRegistry::load(Some(&root)).unwrap());
    v.data.profession = VillagerProfession::Miner;
    let town = v.settlement.unwrap();
    let sheep_slot = sheep(&mut state, [3.5, 201.0, 4.5], 0);
    state.entities.row_mut(sheep_slot).controls.no_ai = true;
    state.entities.set_pos(slot, [2.5, 201.0, 3.5]);
    state.entities.row_mut(slot).villager = Some(v);
    state.world.fold_overlay();
    crate::settlements::construction::request_build(&mut state, town, "house").unwrap();
    let mut mined = false;
    for _ in 0..500 {
        crate::tick::set_wool(
            std::sync::Arc::make_mut(&mut state.entities.row_mut(sheep_slot).appearance),
            0,
        );
        let (output, _, _) = crate::tick::run_tick(&mut state, &[]);
        let worker = state.entities.row(slot).villager.as_ref().unwrap();
        if worker.mining_progress.building != 0 && !output.changes.blocks.is_empty() {
            assert_eq!(
                crate::tick::wool_of(&state.entities.row(sheep_slot).appearance),
                0,
                "shearing must not run in the same decision as mining"
            );
            mined = true;
            break;
        }
    }
    assert!(mined, "the certified miner must actually mutate a block");
}

#[test]
fn excluded_roles_and_children_never_shear_even_with_tools() {
    for (profession, age) in [
        (VillagerProfession::Nitwit, 0),
        (VillagerProfession::Builder, 0),
        (VillagerProfession::Soldier, 0),
        (VillagerProfession::Miner, -1),
    ] {
        let (mut state, slot, mut v) = setup();
        v.data.profession = profession;
        v.age = age;
        let sheep_slot = sheep(&mut state, [3.5, 201.0, 3.5], 0);
        let before = v.inventory.clone();
        state.tick = 20;
        shepherd::work(&mut state, slot, &mut v, &mut ChangeSet::default());
        assert_eq!(
            crate::tick::wool_of(&state.entities.row(sheep_slot).appearance),
            0
        );
        assert_eq!(v.inventory, before);
    }
}

#[test]
fn employed_worker_does_not_shear_when_wool_bill_is_satisfied() {
    let (mut state, slot, mut v) = setup();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla");
    state.registry = std::sync::Arc::new(crate::registry::CoreRegistry::load(Some(&root)).unwrap());
    v.data.profession = VillagerProfession::Miner;
    v.job_site = None;
    let town = v.settlement.unwrap();
    state.settlements.get_mut(town).unwrap().buildings.clear();
    storage(&mut state, &v, [4, 201, 3]);
    let wool = mining::item_id(&state, "white_wool").unwrap();
    state.block_entities.get_mut(&(4, 201, 3)).unwrap().slots[0] = ItemStack::new(wool, 64);
    let sheep_slot = sheep(&mut state, [3.5, 201.0, 3.5], 0);
    crate::settlements::construction::request_build(&mut state, town, "house").unwrap();
    let before = v.inventory.clone();
    v.harvest_at = 10;
    state.tick = 20;
    shepherd::work(&mut state, slot, &mut v, &mut ChangeSet::default());
    assert_eq!(
        crate::tick::wool_of(&state.entities.row(sheep_slot).appearance),
        0
    );
    assert_eq!(v.inventory, before);
    assert_eq!(
        v.harvest_at, 10,
        "wool demand must not delay the miner's own work"
    );
}

#[test]
fn economy_reserved_farmer_cannot_shear() {
    use crate::settlements::{commands::EconomyDomain, economy_commands};
    let (mut state, slot, mut v) = fixture(BuildingKind::Workshop(VillagerProfession::Farmer));
    crate::plugin_startup::apply(
        std::sync::Arc::make_mut(&mut state.registry),
        &mc_plugin_host::DeploymentContribution::default(),
        None,
        &[],
    )
    .unwrap();
    v.data.profession = VillagerProfession::Farmer;
    let town = v.settlement.unwrap();
    let worker = state.entities.row(slot).uuid;
    let shears = mining::item_id(&state, "shears").unwrap();
    v.inventory[0] = Some(ItemStack::new(shears, 1));
    state.entities.row_mut(slot).villager = Some(v);
    economy_commands::execute(
        &mut state,
        town,
        EconomyDomain::Production,
        &[
            "start".into(),
            "shearing-reservation".into(),
            worker.to_string(),
            "solaris-settlements:bread".into(),
            "1".into(),
            ((town.0 << 32) | 1).to_string(),
            "3,201,3".into(),
            "3,201,4".into(),
        ],
    )
    .unwrap();
    assert!(crate::settlements::economy::worker_reserved(
        &state,
        Some(town),
        worker
    ));
    let sheep_slot = sheep(&mut state, [3.5, 201.0, 3.5], 0);
    let mut v = state.entities.row_mut(slot).villager.take().unwrap();
    let before = v.inventory.clone();
    state.tick = 20;
    shepherd::work(&mut state, slot, &mut v, &mut ChangeSet::default());
    assert_eq!(
        crate::tick::wool_of(&state.entities.row(sheep_slot).appearance),
        0
    );
    assert_eq!(v.inventory, before);
}

#[test]
fn shepherd_shears_adult_sheep_for_matching_wool_once() {
    let (mut state, slot, mut v) = setup();
    let sheep_slot = sheep(&mut state, [3.5, 201.0, 3.5], 14);
    state.entities.set_pos(slot, [3.5, 201.0, 3.5]);
    let mut changes = ChangeSet::default();
    state.tick = 20;
    assert_eq!(
        shepherd::work(&mut state, slot, &mut v, &mut changes),
        Some(None)
    );
    let red = mining::item_id(&state, "red_wool").unwrap();
    let yield_count = carried(&v, red);
    assert!(
        (1..=3).contains(&yield_count),
        "vanilla 1..=3 colored yield, got {yield_count}"
    );
    assert_eq!(
        crate::tick::wool_of(&state.entities.row(sheep_slot).appearance),
        0x10 | 14,
        "color kept, sheared bit set"
    );
    assert!(
        state
            .appearance_changed
            .contains(&state.entities.body(sheep_slot).id),
        "shear journals the appearance change"
    );
    // A sheared sheep pays nothing until native grazing regrows the wool.
    state.tick = 120;
    assert_eq!(
        shepherd::work(&mut state, slot, &mut v, &mut changes),
        Some(None)
    );
    assert_eq!(carried(&v, red), yield_count);
    assert_eq!(
        crate::tick::wool_of(&state.entities.row(sheep_slot).appearance),
        0x10 | 14
    );
}

#[test]
fn shepherd_refuses_baby_foreign_and_distant_sheep() {
    let (mut state, slot, mut v) = setup();
    state.entities.set_pos(slot, [3.5, 201.0, 3.5]);
    let baby = sheep(&mut state, [4.5, 201.0, 4.5], 0);
    state.entities.row_mut(baby).entity_life.child_ticks = 200;
    let cow_type = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse("minecraft:cow").unwrap())
        .unwrap();
    state.spawn_mob(cow_type, [3.5, 201.0, 4.5], 10.0);
    let far = sheep(&mut state, [108.5, 201.0, 108.5], 0);
    let appearances = state.appearance_changed.clone();
    let mut changes = ChangeSet::default();
    state.tick = 20;
    assert_eq!(shepherd::work(&mut state, slot, &mut v, &mut changes), None);
    assert_eq!(v.inventory.iter().flatten().count(), 1);
    assert_eq!(
        crate::tick::wool_of(&state.entities.row(baby).appearance),
        0,
        "baby keeps its wool"
    );
    assert_eq!(
        crate::tick::wool_of(&state.entities.row(far).appearance),
        0,
        "distant sheep keeps its wool"
    );
    assert_eq!(state.appearance_changed, appearances);
}

#[test]
fn shepherd_full_carry_keeps_sheep_wool() {
    let (mut state, slot, mut v) = setup();
    let sheep_slot = sheep(&mut state, [3.5, 201.0, 3.5], 0);
    state.entities.set_pos(slot, [3.5, 201.0, 3.5]);
    let cobble = mining::item_id(&state, "cobblestone").unwrap();
    v.inventory = std::array::from_fn(|_| Some(ItemStack::new(cobble, 64)));
    let before = v.inventory.clone();
    let mut changes = ChangeSet::default();
    state.tick = 20;
    // No storage certified: the deposit has nowhere to go.
    assert_eq!(shepherd::work(&mut state, slot, &mut v, &mut changes), None);
    assert_eq!(
        crate::tick::wool_of(&state.entities.row(sheep_slot).appearance),
        0,
        "no room means no shearing"
    );
    assert_eq!(v.inventory, before, "carry preserved without storage");
}

#[test]
fn shepherd_stock_deposit_conserves_wool() {
    let (mut state, slot, mut v) = setup();
    storage(&mut state, &v, [4, 201, 3]);
    let white = mining::item_id(&state, "white_wool").unwrap();
    state.block_entities.get_mut(&(4, 201, 3)).unwrap().slots[0] = ItemStack::new(white, 10);
    v.inventory = std::array::from_fn(|_| Some(ItemStack::new(white, 64)));
    state.entities.set_pos(slot, [4.5, 201.0, 3.5]);
    let mut changes = ChangeSet::default();
    state.tick = 20;
    assert_eq!(
        shepherd::work(&mut state, slot, &mut v, &mut changes),
        Some(None)
    );
    assert!(v.inventory.iter().all(Option::is_none));
    let chest = state.block_entities.get(&(4, 201, 3)).unwrap();
    let total: i32 = chest.slots.iter().map(|stack| stack.count).sum();
    assert_eq!(total, 10 + 8 * 64, "stock plus carry, nothing duplicated");
}

#[test]
fn shepherd_uses_a_simulated_sheep_not_a_closer_inactive_one() {
    let (mut state, slot, mut v) = setup();
    let inactive_chunk = crate::boundary::ChunkPos { x: 1, z: 0 };
    state
        .world
        .ensure_blocking(&rustc_hash::FxHashMap::from_iter([(inactive_chunk, 0)]));
    assert!(state.world.is_active(0, 0));
    assert!(!state.world.is_active(1, 0));
    state.entities.set_pos(slot, [15.5, 201.0, 3.5]);
    let inactive = sheep(&mut state, [16.5, 201.0, 3.5], 0);
    let active = sheep(&mut state, [13.5, 201.0, 3.5], 0);
    state.tick = 20;
    shepherd::work(&mut state, slot, &mut v, &mut ChangeSet::default());
    assert_eq!(
        crate::tick::wool_of(&state.entities.row(inactive).appearance),
        0,
        "a worker must not route into an inactive chunk and stop ticking"
    );
    assert_eq!(
        crate::tick::wool_of(&state.entities.row(active).appearance),
        0x10
    );
}
