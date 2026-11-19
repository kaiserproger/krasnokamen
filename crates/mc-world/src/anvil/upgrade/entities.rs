//! Entity/player remainder fixes. Evidence: `.analysis/old-world-upgrade/*.javap.txt`.
//! Only the explicitly named schema fields below are visited; opaque custom data is untouched.

use super::nbt::{
    compound, fields, get, get_mut, list_mut, number, remove, rename, set, string, take,
};
use super::{NodeKind, UpgradeError, UpgradeReport};
use mc_nbt::{ListTag, Tag};

mod attributes;
mod effects;
mod equipment;
mod particles;
mod positions;
mod variants;

use attributes::{attribute_format, attribute_prefixes, follow_range, legacy_attributes};
pub(super) use effects::fix_potion_effects;
use effects::{converted_effect_id, effect_list};
use equipment::{drop_chances, empty_second_trade_item, equipment, player_equipment, saddle};
use positions::{entity_positions, inline_positions};
use variants::{arrow_potion, cloud_potion, horse_armor, variant, wolf_health};

fn text(value: &str) -> Tag {
    Tag::String(value.to_owned())
}

// Java Number.intValue: integral narrowing wraps; floating-point narrowing saturates.
fn java_int(value: &Tag) -> Option<i32> {
    match value {
        Tag::Byte(v) => Some(i32::from(*v)),
        Tag::Short(v) => Some(i32::from(*v)),
        Tag::Int(v) => Some(*v),
        Tag::Long(v) => Some(*v as i32),
        Tag::Float(v) => Some(*v as i32),
        Tag::Double(v) => Some(*v as i32),
        _ => None,
    }
}

fn named(value: Option<&Tag>, expected: &str) -> bool {
    value.and_then(string).is_some_and(|value| {
        value == expected
            || (!value.contains(':') && expected.strip_prefix("minecraft:") == Some(value))
    })
}

fn has_any(tag: &Tag, keys: &[&str]) -> bool {
    keys.iter().any(|key| get(tag, key).is_some())
}

fn reject(version: i32, fix: &'static str, path: &str, reason: &str) -> UpgradeError {
    UpgradeError::unsupported(version, fix, path, reason)
}

fn list_field<'a>(
    tag: &'a mut Tag,
    key: &str,
    version: i32,
    fix: &'static str,
) -> Result<Option<&'a mut Vec<Tag>>, UpgradeError> {
    let Some(value) = get_mut(tag, key) else {
        return Ok(None);
    };
    let values = list_mut(value)
        .ok_or_else(|| reject(version, fix, key, "affected field is not an NBT list"))?;
    if !matches!(key, "ArmorDropChances" | "HandDropChances")
        && values.iter().any(|value| fields(value).is_none())
    {
        return Err(reject(
            version,
            fix,
            key,
            "typed list entry is not an NBT compound",
        ));
    }
    Ok(Some(values))
}

pub(super) fn fix_node(
    tag: &mut Tag,
    kind: NodeKind,
    version: i32,
    report: &mut UpgradeReport,
) -> Result<(), UpgradeError> {
    if !matches!(kind, NodeKind::Entity | NodeKind::Player) {
        return Ok(());
    }
    let player = matches!(kind, NodeKind::Player);
    match version {
        3086 if !player && named(get(tag, "id"), "minecraft:cat") => {
            variant(
                tag,
                "CatType",
                &[
                    "minecraft:tabby",
                    "minecraft:black",
                    "minecraft:red",
                    "minecraft:siamese",
                    "minecraft:british",
                    "minecraft:calico",
                    "minecraft:persian",
                    "minecraft:ragdoll",
                    "minecraft:white",
                    "minecraft:jellie",
                    "minecraft:all_black",
                ],
                Some("minecraft:tabby"),
                version,
            )?;
            report.applied(version, "EntityVariantFix");
        }
        3087 if !player && named(get(tag, "id"), "minecraft:frog") => {
            variant(
                tag,
                "Variant",
                &["minecraft:temperate", "minecraft:warm", "minecraft:cold"],
                None,
                version,
            )?;
            report.applied(version, "EntityVariantFix");
        }
        3090 if !player && named(get(tag, "id"), "minecraft:painting") => {
            rename(tag, "Motive", "variant");
            rename(tag, "Facing", "facing");
            report.applied(version, "EntityFieldsRenameFix");
        }
        3093 if !player && named(get(tag, "id"), "minecraft:goat") => {
            set(tag, "HasLeftHorn", Tag::Byte(1));
            set(tag, "HasRightHorn", Tag::Byte(1));
            report.applied(version, "EntityGoatMissingStateFix");
        }
        3097 if !player && named(get(tag, "id"), "minecraft:cat") => {
            if get(tag, "variant").and_then(string) == Some("minecraft:british") {
                set(tag, "variant", text("minecraft:british_shorthair"));
            }
            report.applied(version, "VariantRenameFix");
        }
        3322 => {
            let mut affected = false;
            for key in ["Effects", "ActiveEffects", "CustomPotionEffects"] {
                affected |= effect_list(tag, key, key, version)?;
            }
            if affected {
                report.applied(version, "EffectDurationFix");
            }
        }
        3568 => {
            let mut affected = effect_list(tag, "ActiveEffects", "active_effects", version)?;
            if !player && named(get(tag, "id"), "minecraft:arrow") {
                affected |=
                    effect_list(tag, "CustomPotionEffects", "custom_potion_effects", version)?;
            } else if !player && named(get(tag, "id"), "minecraft:area_effect_cloud") {
                affected |= effect_list(tag, "Effects", "effects", version)?;
            } else if !player && named(get(tag, "id"), "minecraft:mooshroom") {
                let mut stew = compound(vec![]);
                if let Some(id) = get(tag, "EffectId") {
                    set(&mut stew, "id", text(converted_effect_id(id, "EffectId")?));
                }
                if let Some(duration) = take(tag, "EffectDuration") {
                    set(&mut stew, "duration", duration);
                }
                affected |=
                    get(tag, "EffectId").is_some() || fields(&stew).is_some_and(|f| !f.is_empty());
                remove(tag, "EffectId");
                if fields(&stew).is_some_and(|f| !f.is_empty()) {
                    set(
                        tag,
                        "stew_effects",
                        Tag::List(ListTag {
                            element_type: 10,
                            elements: vec![stew],
                        }),
                    );
                }
            }
            if affected {
                report.applied(version, "MobEffectIdFix");
            }
        }
        3683 if !player && named(get(tag, "id"), "minecraft:tnt") => {
            set(
                tag,
                "block_state",
                compound(vec![("Name", text("minecraft:tnt"))]),
            );
            // The vanilla fixer copies Fuse, not renameField; the old remainder is retained.
            if let Some(fuse) = get(tag, "Fuse").cloned() {
                set(tag, "fuse", fuse);
            }
            report.applied(version, "PrimedTntBlockStateFixer");
        }
        3685 if !player => {
            if named(get(tag, "id"), "minecraft:arrow") {
                let tipped = get(tag, "Potion")
                    .and_then(string)
                    .is_some_and(|potion| potion != "minecraft:empty");
                set(
                    tag,
                    "item",
                    compound(vec![
                        (
                            "id",
                            text(if tipped {
                                "minecraft:tipped_arrow"
                            } else {
                                "minecraft:arrow"
                            }),
                        ),
                        ("Count", Tag::Int(1)),
                    ]),
                );
                report.applied(version, "FixProjectileStoredItem");
            } else if named(get(tag, "id"), "minecraft:spectral_arrow") {
                set(
                    tag,
                    "item",
                    compound(vec![
                        ("id", text("minecraft:spectral_arrow")),
                        ("Count", Tag::Int(1)),
                    ]),
                );
                report.applied(version, "FixProjectileStoredItem");
            } else if named(get(tag, "id"), "minecraft:trident") {
                rename(tag, "Trident", "item");
                report.applied(version, "FixProjectileStoredItem");
            }
        }
        3808 if !player => {
            let horse = named(get(tag, "id"), "minecraft:horse");
            if ((horse && report.from != 3808)
                || named(get(tag, "id"), "minecraft:llama")
                || named(get(tag, "id"), "minecraft:trader_llama"))
                && horse_armor(tag, horse)?
            {
                report.applied(version, "HorseBodyArmorItemFix");
            }
        }
        3809 if !player
            && ["donkey", "mule", "llama", "trader_llama"]
                .iter()
                .any(|id| {
                    get(tag, "id").and_then(string).is_some_and(|actual| {
                        actual.strip_prefix("minecraft:").unwrap_or(actual) == *id
                    })
                }) =>
        {
            if let Some(items) = list_field(
                tag,
                "Items",
                version,
                "ChestedHorsesInventoryZeroIndexingFix",
            )? {
                for item in items {
                    if let Some(slot) = get_mut(item, "Slot") {
                        *slot = Tag::Byte(java_int(slot).unwrap_or(2).wrapping_sub(2) as i8);
                    }
                }
                report.applied(version, "ChestedHorsesInventoryZeroIndexingFix");
            }
        }
        3812 if !player && named(get(tag, "id"), "minecraft:wolf") => {
            if wolf_health(tag)? {
                report.applied(version, "FixWolfHealth");
            }
        }
        3813 if !player => {
            if entity_positions(tag) {
                report.applied(version, "BlockPosFormatAndRenamesFix");
            }
        }
        3814 => {
            if legacy_attributes(tag)? {
                report.applied(version, "AttributesRenameLegacy");
            }
        }
        3818 if !player => {
            if named(get(tag, "id"), "minecraft:area_effect_cloud")
                && let Some(particle) = get_mut(tag, "Particle")
                && particles::unflatten(particle)?
            {
                report.applied(version, "ParticleUnflatteningFix");
            }
            if named(get(tag, "id"), "minecraft:arrow") && arrow_potion(tag)? {
                report.applied(version, "TippedArrowPotionToItemFix");
            }
            if named(get(tag, "id"), "minecraft:area_effect_cloud") && cloud_potion(tag) {
                report.applied(version, "AreaEffectCloudPotionFix");
            }
        }
        3828 if !player
            && [
                "minecraft:villager",
                "minecraft:zombie_villager",
                "minecraft:wandering_trader",
            ]
            .iter()
            .any(|id| named(get(tag, "id"), id)) =>
        {
            empty_second_trade_item(tag, report)?;
        }
        3938 if !player
            && (named(get(tag, "id"), "minecraft:arrow")
                || named(get(tag, "id"), "minecraft:spectral_arrow")) =>
        {
            // write/read identity: weapon becomes schema-typed; the parent walker upgrades it.
            if get(tag, "weapon").is_some() {
                report.applied(version, "ProjectileStoredWeaponFix");
            }
        }
        3945 => {
            if attribute_format(tag)? {
                report.applied(version, "AttributeModifierIdFix");
            }
        }
        4055 => {
            if attribute_prefixes(tag)? {
                report.applied(version, "AttributeIdPrefixFix");
            }
        }
        4067 if !player
            && (named(get(tag, "id"), "minecraft:boat")
                || named(get(tag, "id"), "minecraft:chest_boat")) =>
        {
            let chest = named(get(tag, "id"), "minecraft:chest_boat");
            let variant = get(tag, "Type").and_then(string).unwrap_or("oak");
            let wood = match variant {
                "spruce" | "birch" | "jungle" | "acacia" | "cherry" | "dark_oak" | "mangrove"
                | "bamboo" => variant,
                _ => "oak",
            };
            let id = format!(
                "minecraft:{wood}_{}",
                if wood == "bamboo" {
                    if chest { "chest_raft" } else { "raft" }
                } else if chest {
                    "chest_boat"
                } else {
                    "boat"
                }
            );
            remove(tag, "Type");
            set(tag, "id", Tag::String(id));
            report.applied(version, "BoatSplitFix");
        }
        4081 if !player && named(get(tag, "id"), "minecraft:salmon") => {
            if get(tag, "type").and_then(string) != Some("large") {
                set(tag, "type", text("medium"));
            }
            report.applied(version, "EntitySalmonSizeFix");
        }
        4173 if !player && named(get(tag, "id"), "minecraft:tnt_minecart") => {
            rename(tag, "TNTFuse", "fuse");
            report.applied(version, "EntityFieldsRenameFix");
        }
        4187 if !player => {
            let ranges = [
                ("minecraft:villager", 48.0, 16.0),
                ("minecraft:bee", 48.0, 16.0),
                ("minecraft:allay", 48.0, 16.0),
                ("minecraft:llama", 40.0, 16.0),
                ("minecraft:piglin_brute", 16.0, 12.0),
                ("minecraft:warden", 16.0, 24.0),
            ];
            for (id, old, new) in ranges {
                if named(get(tag, "id"), id) && follow_range(tag, old, new)? {
                    report.applied(version, "EntityAttributeBaseValueFix");
                }
            }
        }
        4293 if !player => {
            if drop_chances(tag)? {
                report.applied(version, "DropChancesFormatFix");
            }
        }
        4296 if !player && named(get(tag, "id"), "minecraft:area_effect_cloud") => {
            set(tag, "potion_duration_scale", Tag::Float(0.25));
            report.applied(version, "AreaEffectCloudDurationScaleFix");
        }
        4300 if !player => {
            if saddle(tag) {
                report.applied(version, "SaddleEquipmentSlotFix");
            }
        }
        4301 if !player => {
            if equipment(tag)? {
                report.applied(version, "EquipmentFormatFix");
            }
        }
        4303 => {
            if let Some(distance) = take(tag, "FallDistance") {
                set(
                    tag,
                    "fall_distance",
                    Tag::Double(f64::from(number(&distance).unwrap_or(0.0) as f32)),
                );
                report.applied(version, "EntityFallDistanceFloatToDoubleFix");
            }
        }
        4306 if !player && named(get(tag, "id"), "minecraft:potion") => {
            let lingering = get(tag, "Item")
                .and_then(|item| get(item, "id"))
                .is_some_and(|id| named(Some(id), "minecraft:lingering_potion"));
            set(
                tag,
                "id",
                text(if lingering {
                    "minecraft:lingering_potion"
                } else {
                    "minecraft:splash_potion"
                }),
            );
            report.applied(version, "ThrownPotionSplitFix");
        }
        4312 if player => {
            player_equipment(tag)?;
            report.applied(version, "PlayerEquipmentFix");
        }
        4314 => {
            if inline_positions(tag, player) {
                report.applied(version, "InlineBlockPosFormatFix");
            }
        }
        4420 if !player && named(get(tag, "id"), "minecraft:area_effect_cloud") => {
            if particles::custom_particle(tag)? {
                report.applied(version, "NamedEntityConvertUncheckedFix");
            }
        }
        4424 => {
            // AddFieldFix's disassembly returns a modified nested Dynamic instead of
            // reinserting it in its parent. Refuse affected data rather than fabricate
            // a modern icon or discard the entity/player root.
            if get(tag, "locator_bar_icon").is_some() {
                return Err(reject(
                    version,
                    "AddFieldFix",
                    "locator_bar_icon",
                    "nested vanilla default insertion cannot be applied losslessly",
                ));
            }
        }
        4535 if !player && named(get(tag, "id"), "minecraft:copper_golem") => {
            if let Some(weather) = get_mut(tag, "weather_state") {
                *weather = text(match java_int(weather).unwrap_or(0) {
                    1 => "exposed",
                    2 => "weathered",
                    3 => "oxidized",
                    _ => "unaffected",
                });
                report.applied(version, "CopperGolemWeatherStateFix");
            }
        }
        4763 if !player => {
            if named(get(tag, "id"), "minecraft:villager") {
                set(tag, "VillagerDataFinalized", Tag::Byte(1));
                report.applied(version, "VillagerSetVillagerDataFinalized");
            } else if named(get(tag, "id"), "minecraft:zombie_villager") {
                set(tag, "VillagerDataFinalized", Tag::Byte(1));
                report.applied(version, "ZombieVillagerSetVillagerDataFinalized");
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests;
