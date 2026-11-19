//! Legacy attribute maps, modifier formats and follow range.

use super::*;

pub(super) fn legacy_attributes(tag: &mut Tag) -> Result<bool, UpgradeError> {
    let Some(attributes) = list_field(tag, "Attributes", 3814, "AttributesRenameLegacy")? else {
        return Ok(false);
    };
    for attribute in attributes {
        if named(get(attribute, "Name"), "minecraft:horse.jump_strength") {
            set(attribute, "Name", text("minecraft:generic.jump_strength"));
        }
    }
    Ok(true)
}

pub(super) fn attribute_format(tag: &mut Tag) -> Result<bool, UpgradeError> {
    let Some(attributes) = list_field(tag, "Attributes", 3945, "AttributeModifierIdFix")? else {
        return Ok(false);
    };
    for attribute in attributes {
        rename(attribute, "Name", "id");
        rename(attribute, "Base", "base");
        if let Some(modifiers) = list_field(attribute, "Modifiers", 3945, "AttributeModifierIdFix")?
        {
            for modifier in modifiers {
                for (old, new) in [("UUID", "uuid"), ("Name", "name"), ("Amount", "amount")] {
                    rename(modifier, old, new);
                }
                if let Some(operation) = take(modifier, "Operation") {
                    let op = match java_int(&operation).unwrap_or(0) {
                        1 => "add_multiplied_base",
                        2 => "add_multiplied_total",
                        0 => "add_value",
                        _ => "invalid",
                    };
                    set(modifier, "operation", text(op));
                }
            }
            if let Some(modifiers) = get_mut(attribute, "Modifiers") {
                super::super::items::fix_modifiers(modifiers)?;
            }
        }
        rename(attribute, "Modifiers", "modifiers");
    }
    rename(tag, "Attributes", "attributes");
    Ok(true)
}

pub(super) fn attribute_prefixes(tag: &mut Tag) -> Result<bool, UpgradeError> {
    let Some(attributes) = list_field(tag, "attributes", 4055, "AttributeIdPrefixFix")? else {
        return Ok(false);
    };
    for attribute in attributes {
        if let Some(Tag::String(id)) = get_mut(attribute, "id") {
            let local = id
                .strip_prefix("minecraft:")
                .or_else(|| (!id.contains(':')).then_some(id.as_str()));
            if let Some(local) = local {
                for prefix in ["generic.", "horse.", "player.", "zombie."] {
                    if let Some(suffix) = local.strip_prefix(prefix) {
                        *id = format!("minecraft:{suffix}");
                        break;
                    }
                }
            }
        }
    }
    Ok(true)
}

pub(super) fn follow_range(tag: &mut Tag, old: f64, new: f64) -> Result<bool, UpgradeError> {
    let Some(attributes) = list_field(tag, "attributes", 4187, "EntityAttributeBaseValueFix")?
    else {
        return Ok(false);
    };
    for attribute in attributes {
        if named(get(attribute, "id"), "minecraft:follow_range") {
            let base = get(attribute, "base").and_then(number).unwrap_or(0.0);
            set(
                attribute,
                "base",
                Tag::Double(if base == old { new } else { base }),
            );
        }
    }
    Ok(true)
}
