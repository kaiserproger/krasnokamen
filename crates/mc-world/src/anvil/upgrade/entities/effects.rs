//! Status-effect ids, names and durations in effect lists.

use super::*;

const EFFECTS: [&str; 33] = [
    "minecraft:speed",
    "minecraft:slowness",
    "minecraft:haste",
    "minecraft:mining_fatigue",
    "minecraft:strength",
    "minecraft:instant_health",
    "minecraft:instant_damage",
    "minecraft:jump_boost",
    "minecraft:nausea",
    "minecraft:regeneration",
    "minecraft:resistance",
    "minecraft:fire_resistance",
    "minecraft:water_breathing",
    "minecraft:invisibility",
    "minecraft:blindness",
    "minecraft:night_vision",
    "minecraft:hunger",
    "minecraft:weakness",
    "minecraft:poison",
    "minecraft:wither",
    "minecraft:health_boost",
    "minecraft:absorption",
    "minecraft:saturation",
    "minecraft:glowing",
    "minecraft:levitation",
    "minecraft:luck",
    "minecraft:unluck",
    "minecraft:slow_falling",
    "minecraft:conduit_power",
    "minecraft:dolphins_grace",
    "minecraft:bad_omen",
    "minecraft:hero_of_the_village",
    "minecraft:darkness",
];

/// EffectDurationFix3322 changes only FactorCalculationData on the immediate instance,
/// not HiddenEffect (the vanilla duration fixer does not recurse there).
pub(super) fn fix_effect_duration(effect: &mut Tag) {
    let duration = get(effect, "Duration").and_then(java_int).unwrap_or(-1);
    if let Some(factor) = get_mut(effect, "FactorCalculationData") {
        let timestamp = get(factor, "effect_changed_timestamp")
            .and_then(java_int)
            .unwrap_or(-1);
        remove(factor, "effect_changed_timestamp");
        set(
            factor,
            "ticks_active",
            Tag::Int(timestamp.wrapping_sub(duration)),
        );
    }
}

pub(super) fn converted_effect_id(value: &Tag, path: &str) -> Result<&'static str, UpgradeError> {
    let id = java_int(value).ok_or_else(|| {
        reject(
            3568,
            "MobEffectIdFix",
            path,
            "legacy effect ID is not numeric",
        )
    })?;
    usize::try_from(id.wrapping_sub(1))
        .ok()
        .and_then(|index| EFFECTS.get(index))
        .copied()
        .ok_or_else(|| {
            reject(
                3568,
                "MobEffectIdFix",
                path,
                "unregistered numeric effect ID cannot be losslessly upgraded",
            )
        })
}

fn rename_effect_id(tag: &mut Tag, old: &str, new: &str) -> Result<(), UpgradeError> {
    if let Some(value) = get(tag, old) {
        let id = converted_effect_id(value, old)?;
        remove(tag, old);
        set(tag, new, text(id));
    }
    Ok(())
}

pub(super) fn fix_effect_id(effect: &mut Tag) -> Result<(), UpgradeError> {
    if fields(effect).is_none() {
        return Err(reject(
            3568,
            "MobEffectIdFix",
            "HiddenEffect",
            "effect instance is not a compound",
        ));
    }
    rename_effect_id(effect, "Id", "id")?;
    for (old, new) in [
        ("Ambient", "ambient"),
        ("Amplifier", "amplifier"),
        ("Duration", "duration"),
        ("ShowParticles", "show_particles"),
        ("ShowIcon", "show_icon"),
    ] {
        rename(effect, old, new);
    }
    if let Some(hidden) = get_mut(effect, "HiddenEffect") {
        fix_effect_id(hidden)?;
    }
    rename(effect, "HiddenEffect", "hidden_effect");
    Ok(())
}

pub(super) fn effect_list(
    tag: &mut Tag,
    old: &str,
    new: &str,
    version: i32,
) -> Result<bool, UpgradeError> {
    let fix = if version == 3322 {
        "EffectDurationFix"
    } else {
        "MobEffectIdFix"
    };
    let Some(effects) = list_field(tag, old, version, fix)? else {
        return Ok(false);
    };
    for effect in effects {
        if fields(effect).is_none() {
            return Err(reject(
                version,
                fix,
                old,
                "effect instance is not a compound",
            ));
        }
        if version == 3322 {
            if get(effect, "FactorCalculationData").is_some_and(|factor| fields(factor).is_none()) {
                return Err(reject(
                    version,
                    fix,
                    "FactorCalculationData",
                    "factor calculation data is not a compound",
                ));
            }
            fix_effect_duration(effect);
        } else {
            fix_effect_id(effect)?;
        }
    }
    if old != new {
        rename(tag, old, new);
    }
    Ok(true)
}

/// Called by the Item slice with an entire Item stack, before componentization.
pub(in super::super) fn fix_potion_effects(
    tag: &mut Tag,
    version: i32,
    report: &mut UpgradeReport,
) -> Result<(), UpgradeError> {
    let potion = [
        "minecraft:potion",
        "minecraft:splash_potion",
        "minecraft:lingering_potion",
        "minecraft:tipped_arrow",
    ]
    .iter()
    .any(|id| named(get(tag, "id"), id));
    let stew = named(get(tag, "id"), "minecraft:suspicious_stew");
    let Some(item_tag) = get_mut(tag, "tag") else {
        return Ok(());
    };
    let changed = if potion {
        match version {
            3322 => effect_list(
                item_tag,
                "CustomPotionEffects",
                "CustomPotionEffects",
                version,
            )?,
            3568 => effect_list(
                item_tag,
                "CustomPotionEffects",
                "custom_potion_effects",
                version,
            )?,
            _ => false,
        }
    } else if stew && version == 3568 {
        if let Some(effects) = list_field(item_tag, "Effects", version, "MobEffectIdFix")? {
            for effect in effects {
                rename_effect_id(effect, "EffectId", "id")?;
                rename(effect, "EffectDuration", "duration");
            }
            rename(item_tag, "Effects", "effects");
            true
        } else {
            false
        }
    } else {
        false
    };
    if changed {
        report.applied(
            version,
            if version == 3322 {
                "EffectDurationFix"
            } else {
                "MobEffectIdFix"
            },
        );
    }
    Ok(())
}
