//! The full-data registry load feeds the mc-inventory furnace rules (moved from
//! mc-inventory's furnace tests: only the core registry loads full data).
use mc_data::ItemStack;
use mc_data::recipes::CookingKind;
use mc_inventory::furnace::{FUEL, FurnaceState, INPUT, OUTPUT};
use mc_inventory::{ClickMode, Menu, MenuKind, PlayerInventory};

fn stack(item: u32, count: i32) -> ItemStack {
    ItemStack::new(item, count)
}

#[test]
fn full_data_furnace_accepts_acacia_planks_and_cooks_charcoal() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla");
    let registry = crate::registry::CoreRegistry::load(Some(&dir)).expect("full registry loads");
    let rules = registry.item_rules();
    let item = |name| {
        registry
            .items
            .id_of(&mc_data::Identifier::parse(name).expect("item identifier"))
            .expect("vanilla item")
    };
    let log = item("minecraft:acacia_log");
    let planks = item("minecraft:acacia_planks");
    let charcoal = item("minecraft:charcoal");
    assert!(rules.tags.contains_raw_id(
        mc_data::tags::ITEM_REGISTRY,
        "minecraft:planks",
        planks as i32
    ));
    assert_eq!(
        rules.tags.fuel_values().burn_duration(planks),
        Some(300),
        "full-data startup must initialize fuel values, not just resolved tags"
    );
    let mut menu = Menu::new(1, MenuKind::Furnace(CookingKind::Furnace));
    let mut inventory = PlayerInventory::default();
    inventory.slots[0] = stack(log, 4);
    inventory.slots[1] = stack(planks, 7);
    let mut slots = vec![ItemStack::EMPTY; 3];
    for slot in [30, 0, 31, 1] {
        let state_id = menu.state_id;
        let outcome = menu.click(
            &rules,
            &mut inventory,
            &mut slots,
            state_id,
            slot,
            0,
            ClickMode::Pickup,
            false,
        );
        assert!(!outcome.resync, "current-state pickup must not need resync");
    }
    assert_eq!(slots[INPUT], stack(log, 4));
    assert_eq!(slots[FUEL], stack(planks, 7));
    assert!(
        menu.carried.is_empty(),
        "fuel placement must empty the cursor"
    );
    let mut state = FurnaceState::default();
    for _ in 0..200 {
        state.tick(&rules, CookingKind::Furnace, &mut slots);
    }
    assert_eq!(state.lit_total, 300);
    assert_eq!(slots[INPUT], stack(log, 3));
    assert_eq!(slots[FUEL], stack(planks, 6));
    assert_eq!(slots[OUTPUT], stack(charcoal, 1));
}
