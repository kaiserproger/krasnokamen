//! Armor/hand slots, drop chances, saddles and villager trade items.

use super::*;

const ARMOR_SLOTS: [&str; 4] = ["feet", "legs", "chest", "head"];
const HAND_SLOTS: [&str; 2] = ["mainhand", "offhand"];

pub(super) fn drop_chances(tag: &mut Tag) -> Result<bool, UpgradeError> {
    let affected = has_any(
        tag,
        &[
            "ArmorDropChances",
            "HandDropChances",
            "body_armor_drop_chance",
        ],
    );
    let mut chances = compound(vec![]);
    for (key, slots) in [
        ("ArmorDropChances", ARMOR_SLOTS.as_slice()),
        ("HandDropChances", HAND_SLOTS.as_slice()),
    ] {
        if let Some(values) = list_field(tag, key, 4293, "DropChancesFormatFix")? {
            for (value, slot) in values.iter().zip(slots) {
                let chance = number(value).map(|n| n as f32).unwrap_or(0.085);
                if chance != 0.085 {
                    set(&mut chances, slot, Tag::Float(chance));
                }
            }
        }
    }
    let body = get(tag, "body_armor_drop_chance")
        .and_then(number)
        .map(|n| n as f32)
        .unwrap_or(0.085);
    if body != 0.085 {
        set(&mut chances, "body", Tag::Float(body));
    }
    for key in [
        "ArmorDropChances",
        "HandDropChances",
        "body_armor_drop_chance",
    ] {
        remove(tag, key);
    }
    if fields(&chances).is_some_and(|fields| !fields.is_empty()) {
        set(tag, "drop_chances", chances);
    }
    Ok(affected)
}

pub(super) fn saddle(tag: &mut Tag) -> bool {
    let mut changed = false;
    if [
        "horse",
        "skeleton_horse",
        "zombie_horse",
        "donkey",
        "mule",
        "camel",
        "llama",
        "trader_llama",
    ]
    .iter()
    .any(|id| {
        get(tag, "id")
            .and_then(string)
            .is_some_and(|actual| actual.strip_prefix("minecraft:").unwrap_or(actual) == *id)
    }) {
        if get(tag, "SaddleItem").is_some() {
            rename(tag, "SaddleItem", "saddle");
            changed = true;
        }
    } else if named(get(tag, "id"), "minecraft:pig") || named(get(tag, "id"), "minecraft:strider") {
        let present = get(tag, "Saddle").is_some();
        let saddled = get(tag, "Saddle")
            .and_then(java_int)
            .is_some_and(|n| n != 0);
        remove(tag, "Saddle");
        if saddled {
            set(
                tag,
                "saddle",
                compound(vec![
                    ("id", text("minecraft:saddle")),
                    ("count", Tag::Int(1)),
                ]),
            );
            changed = true;
        } else {
            return present;
        }
    }
    if changed {
        let mut chances = take(tag, "drop_chances").unwrap_or_else(|| compound(vec![]));
        set(&mut chances, "saddle", Tag::Float(2.0));
        set(tag, "drop_chances", chances);
    }
    changed
}

pub(super) fn equipment(tag: &mut Tag) -> Result<bool, UpgradeError> {
    let affected = has_any(
        tag,
        &["ArmorItems", "HandItems", "body_armor_item", "saddle"],
    );
    if !affected {
        return Ok(false);
    }
    // These lists are schema-typed Item stacks. A missing id means the placeholder is empty;
    // body/saddle are optional stacks, not list placeholders, and remain even without an id.
    for key in ["ArmorItems", "HandItems"] {
        let _ = list_field(tag, key, 4301, "EquipmentFormatFix")?;
    }
    let mut result = compound(vec![]);
    for (key, slots) in [
        ("HandItems", HAND_SLOTS.as_slice()),
        ("ArmorItems", ARMOR_SLOTS.as_slice()),
    ] {
        if let Some(Tag::List(items)) = take(tag, key) {
            for (item, slot) in items.elements.into_iter().zip(slots) {
                if get(&item, "id").is_some() {
                    set(&mut result, slot, item);
                }
            }
        }
    }
    for (old, new) in [("body_armor_item", "body"), ("saddle", "saddle")] {
        if let Some(item) = take(tag, old) {
            set(&mut result, new, item);
        }
    }
    if fields(&result).is_some_and(|fields| !fields.is_empty()) {
        set(tag, "equipment", result);
    }
    Ok(true)
}

pub(super) fn player_equipment(tag: &mut Tag) -> Result<(), UpgradeError> {
    let mut equipment = compound(vec![]);
    if let Some(items) = list_field(tag, "Inventory", 4312, "PlayerEquipmentFix")? {
        let mut index = 0;
        while index < items.len() {
            let slot = match get(&items[index], "Slot").and_then(java_int).unwrap_or(-1) {
                100 => Some("feet"),
                101 => Some("legs"),
                102 => Some("chest"),
                103 => Some("head"),
                -106 => Some("offhand"),
                _ => None,
            };
            if let Some(slot) = slot {
                let mut item = items.remove(index);
                remove(&mut item, "Slot");
                set(&mut equipment, slot, item);
            } else {
                index += 1;
            }
        }
    }
    // Vanilla replaces equipment with an empty map as well when Inventory is absent.
    set(tag, "equipment", equipment);
    Ok(())
}

pub(super) fn empty_second_trade_item(
    tag: &mut Tag,
    report: &mut UpgradeReport,
) -> Result<(), UpgradeError> {
    let Some(offers) = get_mut(tag, "Offers") else {
        return Ok(());
    };
    let Some(recipes) = list_field(offers, "Recipes", 3828, "EmptyItemInVillagerTradeFix")? else {
        return Ok(());
    };
    for recipe in recipes {
        if let Some(item) = get(recipe, "buyB") {
            let id = get(item, "id").and_then(string).unwrap_or("minecraft:air");
            let count = get(item, "count").and_then(java_int).unwrap_or(0);
            if id == "air" || id == "minecraft:air" || count == 0 {
                remove(recipe, "buyB");
                report.applied(3828, "EmptyItemInVillagerTradeFix");
            }
        }
    }
    Ok(())
}
