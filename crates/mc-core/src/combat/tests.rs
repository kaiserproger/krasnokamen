use std::collections::BTreeMap;

use mc_data::items::{ItemRegistry, ItemReport};
use mc_data::tags::TagsData;
use mc_data::{Identifier, ItemStack};
use mc_inventory::ItemRules;
use mc_survival::combat::{Swing, below_minimum_charge};

fn id(name: &str) -> Identifier {
    Identifier::parse(name).unwrap()
}

fn swing(ticks: u32) -> Swing {
    Swing {
        ticks_since_reset: ticks,
        falling: false,
        on_ground: true,
        sprinting: false,
        crit_blocked: false,
        melee_bonus: 0.0,
        attack_speed_multiplier: 1.0,
    }
}

/// Vanilla `cannotAttackWithItem(stack, 5)` with the iron spear's
/// `minimum_attack_charge` 1.0 and 19-tick attack delay (embedded vanilla
/// facts): refused 13 ticks after the last attack, allowed from 14, from the
/// item's speed or the player's attribute alike. The sword has no minimum.
#[test]
fn spear_minimum_charge_refuses_until_fourteen_ticks() {
    let items = ItemRegistry::from_report(&[
        ItemReport {
            id: id("minecraft:iron_spear"),
            protocol_id: 1,
        },
        ItemReport {
            id: id("minecraft:iron_sword"),
            protocol_id: 2,
        },
    ]);
    let facts = mc_data::item_components::krasnokamen_required_item_facts();
    let tags = TagsData::from_registries(BTreeMap::new());
    let rules = ItemRules {
        items: &items,
        facts: &facts,
        tags: &tags,
        recipes: &[],
    };
    let spear = ItemStack::new(1, 1);
    let sword = ItemStack::new(2, 1);
    let refused = |held: &ItemStack, ticks: u32, attributes: Option<(f64, f64)>| {
        below_minimum_charge(&rules, held, held, swing(ticks), attributes)
    };
    let attribute = Some((3.0, 4.0 - 2.947_368_383_407_592_8));
    for attributes in [None, attribute] {
        assert!(refused(&spear, 13, attributes), "{attributes:?}");
        assert!(!refused(&spear, 14, attributes), "{attributes:?}");
    }
    assert!(!refused(&sword, 0, None));
}
