//! Entity variants, horse armor, wolf health, arrow/cloud potions.

use super::*;

pub(super) fn variant(
    tag: &mut Tag,
    old: &str,
    variants: &[&str],
    default: Option<&str>,
    version: i32,
) -> Result<(), UpgradeError> {
    if let Some(value) = get(tag, old) {
        // EntityVariantFix.updateAndRename sets variant but intentionally retains the old field.
        let replacement = if let Some(id) = java_int(value) {
            let mapped = usize::try_from(id)
                .ok()
                .and_then(|index| variants.get(index))
                .copied()
                .or(default)
                .ok_or_else(|| {
                    reject(
                        version,
                        "EntityVariantFix",
                        old,
                        "unregistered variant ID has no vanilla mapping",
                    )
                })?;
            text(mapped)
        } else {
            value.clone()
        };
        set(tag, "variant", replacement);
    }
    Ok(())
}

pub(super) fn horse_armor(tag: &mut Tag, horse: bool) -> Result<bool, UpgradeError> {
    let key = if horse { "ArmorItem" } else { "DecorItem" };
    if get(tag, key).is_none() {
        return Ok(false);
    }
    if horse {
        if let Some(Tag::List(chances)) = get(tag, "ArmorDropChances")
            && chances.elements.len() > 2
            && chances.element_type != 5
        {
            return Err(reject(
                3808,
                "HorseBodyArmorItemFix",
                "ArmorDropChances",
                "replacing a slot in a non-float list would create heterogeneous NBT",
            ));
        }
        if let Some(items) = list_field(tag, "ArmorItems", 3808, "HorseBodyArmorItemFix")?
            && let Some(item) = items.get_mut(2)
        {
            *item = compound(vec![]);
        }
        if let Some(chances) = list_field(tag, "ArmorDropChances", 3808, "HorseBodyArmorItemFix")?
            && let Some(chance) = chances.get_mut(2)
        {
            *chance = Tag::Float(0.085);
        }
    }
    if let Some(armor) = take(tag, key) {
        set(tag, "body_armor_item", armor);
        set(tag, "body_armor_drop_chance", Tag::Float(2.0));
    }
    Ok(true)
}

pub(super) fn wolf_health(tag: &mut Tag) -> Result<bool, UpgradeError> {
    let mut doubled = false;
    if let Some(attributes) = list_field(tag, "Attributes", 3812, "FixWolfHealth")? {
        for attribute in attributes {
            if named(get(attribute, "Name"), "minecraft:generic.max_health")
                && let Some(base) = get_mut(attribute, "Base")
                && number(base).unwrap_or(0.0) == 20.0
            {
                *base = Tag::Double(40.0);
                doubled = true;
            }
        }
    }
    if doubled && let Some(health) = get_mut(tag, "Health") {
        *health = Tag::Float((number(health).unwrap_or(0.0) as f32) * 2.0);
    }
    Ok(doubled)
}

pub(super) fn arrow_potion(tag: &mut Tag) -> Result<bool, UpgradeError> {
    if !has_any(tag, &["Potion", "custom_potion_effects", "Color"]) {
        return Ok(false);
    }
    if get(tag, "item").is_none_or(|item| fields(item).is_none()) {
        return Err(reject(
            3818,
            "TippedArrowPotionToItemFix",
            "item",
            "potion-bearing arrow lacks a typed stored item",
        ));
    }
    if get(tag, "item")
        .and_then(|item| get(item, "tag"))
        .is_some_and(|value| fields(value).is_none())
    {
        return Err(reject(
            3818,
            "TippedArrowPotionToItemFix",
            "item/tag",
            "stored item tag is not a compound",
        ));
    }
    let potion = take(tag, "Potion");
    let effects = take(tag, "custom_potion_effects");
    let color = take(tag, "Color");
    if let Some(item) = get_mut(tag, "item") {
        let mut item_tag = take(item, "tag").unwrap_or_else(|| compound(vec![]));
        if let Some(potion) = potion {
            set(&mut item_tag, "Potion", potion);
        }
        if let Some(effects) = effects {
            set(&mut item_tag, "custom_potion_effects", effects);
        }
        if let Some(color) = color {
            set(&mut item_tag, "CustomPotionColor", color);
        }
        set(item, "tag", item_tag);
    }
    Ok(true)
}

pub(super) fn cloud_potion(tag: &mut Tag) -> bool {
    if !has_any(tag, &["Color", "effects", "Potion"]) {
        return false;
    }
    let mut contents = compound(vec![]);
    for (old, new) in [
        ("Color", "custom_color"),
        ("effects", "custom_effects"),
        ("Potion", "potion"),
    ] {
        if let Some(value) = take(tag, old) {
            set(&mut contents, new, value);
        }
    }
    set(tag, "potion_contents", contents);
    true
}
