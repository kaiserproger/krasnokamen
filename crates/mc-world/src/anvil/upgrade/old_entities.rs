//! Pre-2586 entity and block-entity fixers reachable within PLAYER records.
//!
//! Bytecode authority: official 26.1.2 `server-26.1.2.jar`
//! (`.analysis/maps/falcons-rock/vanilla-hop/server/versions/26.1.2/server-26.1.2.jar`),
//! registrations in `.analysis/codex-logs/old-dataversion/DataFixers.javap`,
//! per-fixer evidence in `.analysis/codex-logs/old-dataversion/entity-*.javap`,
//! schema evidence in `.analysis/codex-logs/old-dataversion/schema-V*.javap`,
//! version/revision inventory in
//! `.analysis/codex-logs/old-dataversion/legacy-registration-inventory.tsv`.
//! Applies to `Entity` and `BlockEntity` nodes only; the parent wires version
//! order and the tree walk (walk.rs already visits Passengers, shoulder
//! entities, RootVehicle.Entity, and item EntityTag/BlockEntityTag payloads).
//! Player-root UUID/attribute/dimension fields (2514/2523/2537) belong to
//! `old_player_map`; this module handles only nested entity/block-entity
//! payloads. 2514 living-entity attribute UUIDs and 2523 attribute renames reuse
//! `old_player_map::{living_uuid, rename_attributes}` exactly.
use super::{NodeKind, UpgradeError, UpgradeReport, nbt, old_items, old_player_map};
use mc_nbt::Tag;

mod blocks;
mod display;
mod equipment;
mod ids;
mod uuids;
mod villagers;

use blocks::{
    arrow_in_block, banner_colors, colorless_shulker, enderman_carried, falling_block,
    furnace_recipes, jigsaw_properties, jukebox, ominous_banner, piston_block_state,
    remove_block_entity_data, remove_throwable_block, shulker_rotation,
};
use display::{
    custom_name_to_component, direction_2d_to_3d, minecart_display, painting_direction,
    painting_motive, sign_text,
};
use equipment::{
    equipment, health, horse_saddle, redundant_chances, riding, spawner_fields, spawner_identifiers,
};
use ids::{block_entity_id, entity_id, set_id, the_renamening};
use uuids::{block_entity_uuid, entity_uuid, gossip_uuid, projectile_owner};
use villagers::{
    cat_split, memory_expiry, remove_golem_gossip, villager_data, villager_follow_range,
    villager_rebuild, villager_trade, zombie_split, zombie_villager_rebuild, zombie_villager_type,
};

fn text(value: &str) -> Tag {
    Tag::String(value.into())
}

fn named(value: Option<&Tag>, expected: &str) -> bool {
    value.and_then(nbt::string).is_some_and(|value| {
        value == expected
            || (!value.contains(':') && expected.strip_prefix("minecraft:") == Some(value))
    })
}

/// Migrate one entity or block-entity node at one legacy version.
/// All other node kinds and versions are no-ops; the parent wires version
/// order and the tree walk. Typed BLOCK_STATE upgrades (1450 and the
/// block-rename/property paths) belong to `old_block_states::fix_node`;
/// this module only resolves numeric states via its lookup helpers.
pub(super) fn fix_node(
    tag: &mut Tag,
    kind: NodeKind,
    version: i32,
    report: &mut UpgradeReport,
) -> Result<(), UpgradeError> {
    if !matches!(kind, NodeKind::Entity | NodeKind::BlockEntity) {
        return Ok(());
    }
    let entity = kind == NodeKind::Entity;
    // Borrow-safe id access: read the id fresh in each arm; only arms that
    // mutate the id clone it locally (set_id takes &str, no eager alloc).
    match version {
        100 if entity => {
            if equipment(tag) {
                report.applied(version, "EntityEquipmentToArmorAndHandFix");
            }
        }
        101 if entity && nbt::get(tag, "id").and_then(nbt::string) == Some("Villager") => {
            nbt::set(tag, "CanPickUpLoot", Tag::Byte(1));
            report.applied(version, "VillagerSetCanPickUpLootFix");
        }
        106 if !entity && nbt::get(tag, "id").and_then(nbt::string) == Some("MobSpawner") => {
            if spawner_identifiers(tag) {
                report.applied(version, "MobSpawnerEntityIdentifiersFix");
            }
        }
        106 if entity
            && matches!(
                nbt::get(tag, "id").and_then(nbt::string),
                Some("MinecartSpawner") | Some("minecraft:spawner_minecart")
            ) =>
        {
            // UNTAGGED_SPAWNER is nested untyped inside the MinecartSpawner
            // entity (V99 registerEntities template [DisplayTile,
            // UNTAGGED_SPAWNER]); the outer entity id stays untouched, and the
            // pre-705 legacy id is kept until EntityIdFix renames it.
            if spawner_fields(tag) {
                report.applied(version, "MobSpawnerEntityIdentifiersFix");
            }
        }
        107 if entity && nbt::get(tag, "id").and_then(nbt::string) == Some("Minecart") => {
            let kind = match nbt::get(tag, "Type").and_then(nbt::int).unwrap_or(0) {
                1 => "MinecartChest",
                2 => "MinecartFurnace",
                _ => "MinecartRideable",
            };
            set_id(tag, kind);
            nbt::remove(tag, "Type");
            report.applied(version, "EntityMinecartIdentifiersFix");
        }
        108 if entity => {
            if let Some([a, b, c, d]) = nbt::get(tag, "UUID")
                .and_then(nbt::string)
                .and_then(old_items::parse_uuid)
            {
                let most = ((a as i64) << 32) | b as u32 as i64;
                let least = ((c as i64) << 32) | d as u32 as i64;
                nbt::remove(tag, "UUID");
                nbt::set(tag, "UUIDMost", Tag::Long(most));
                nbt::set(tag, "UUIDLeast", Tag::Long(least));
                report.applied(version, "EntityStringUuidFix");
            }
        }
        // ENTITIES set is dead in the jar (declaration + static init only,
        // zero reads); vanilla normalizes HealF/Health on any entity.
        109 if entity => {
            if health(tag) {
                report.applied(version, "EntityHealthFix");
            }
        }
        110 if entity && named(nbt::get(tag, "id"), "EntityHorse") => {
            if horse_saddle(tag) {
                report.applied(version, "EntityHorseSaddleFix");
            }
        }
        111 if entity => {
            if painting_direction(tag) {
                report.applied(version, "EntityPaintingItemFrameDirectionFix");
            }
        }
        113 if entity => {
            if redundant_chances(tag) {
                report.applied(version, "EntityRedundantChanceTagsFix");
            }
        }
        135 if entity => {
            if riding(tag) {
                report.applied(version, "EntityRidingToPassengersFix");
            }
        }
        143 if entity && nbt::get(tag, "id").and_then(nbt::string) == Some("TippedArrow") => {
            set_id(tag, "Arrow");
            report.applied(version, "EntityTippedArrowFix");
        }
        147 if entity && nbt::get(tag, "id").and_then(nbt::string) == Some("ArmorStand") => {
            let silent = nbt::get(tag, "Silent")
                .and_then(nbt::number)
                .is_some_and(|v| v as i8 != 0);
            let marker = nbt::get(tag, "Marker")
                .and_then(nbt::number)
                .is_some_and(|v| v as i8 != 0);
            // Vanilla: remove Silent only when silent && !marker.
            if silent && !marker {
                nbt::remove(tag, "Silent");
                report.applied(version, "EntityArmorStandSilentFix");
            }
        }
        // 165 SignTextStrictJsonFix (NamedEntityFix on BE Sign): Text1..4 via
        // rewriteFromLenient. Lenient/empty/null inputs become EMPTY_CONTENTS
        // ({"text":""}); JSON primitives re-wrap as text components.
        165 if !entity
            && matches!(
                nbt::get(tag, "id").and_then(nbt::string),
                Some("Sign") | Some("minecraft:sign")
            ) =>
        {
            if sign_text(tag) {
                report.applied(version, "SignTextStrictJsonFix");
            }
        }
        502 if entity && nbt::get(tag, "id").and_then(nbt::string) == Some("Zombie") => {
            if zombie_villager_type(tag) {
                report.applied(version, "EntityZombieVillagerTypeFix");
            }
        }
        700 if entity && nbt::get(tag, "id").and_then(nbt::string) == Some("Guardian") => {
            if nbt::get(tag, "Elder")
                .and_then(nbt::number)
                .is_some_and(|v| v as i8 != 0)
            {
                set_id(tag, "ElderGuardian");
                report.applied(version, "EntityElderGuardianSplitFix");
            }
        }
        701 if entity && nbt::get(tag, "id").and_then(nbt::string) == Some("Skeleton") => {
            let renamed = match nbt::get(tag, "SkeletonType")
                .and_then(nbt::int)
                .unwrap_or(0)
            {
                1 => Some("WitherSkeleton"),
                2 => Some("Stray"),
                _ => None,
            };
            // SimpleEntityRenameFix: name changes, remainder tag untouched
            // (SkeletonType retained).
            if let Some(name) = renamed {
                set_id(tag, name);
                report.applied(version, "EntitySkeletonSplitFix");
            }
        }
        702 if entity && nbt::get(tag, "id").and_then(nbt::string) == Some("Zombie") => {
            if zombie_split(tag) {
                report.applied(version, "EntityZombieSplitFix");
            }
        }
        703 if entity && nbt::get(tag, "id").and_then(nbt::string) == Some("EntityHorse") => {
            let name = match nbt::get(tag, "Type").and_then(nbt::int).unwrap_or(0) {
                1 => "Donkey",
                2 => "Mule",
                3 => "ZombieHorse",
                4 => "SkeletonHorse",
                _ => "Horse",
            };
            set_id(tag, name);
            nbt::remove(tag, "Type");
            report.applied(version, "EntityHorseSplitFix");
        }
        704 if !entity => {
            let mapped = nbt::get(tag, "id")
                .and_then(nbt::string)
                .and_then(block_entity_id);
            if let Some(mapped) = mapped {
                set_id(tag, mapped);
                report.applied(version, "BlockEntityIdFix");
            }
        }
        705 if entity => {
            let mapped = nbt::get(tag, "id")
                .and_then(nbt::string)
                .and_then(entity_id);
            if let Some(mapped) = mapped {
                set_id(tag, mapped);
                report.applied(version, "EntityIdFix");
            }
        }
        // 808r1 EntityShulkerColorFix (schema rev 1): runs from version 808.
        808 if entity && named(nbt::get(tag, "id"), "minecraft:shulker") => {
            if nbt::get(tag, "Color").is_none() {
                nbt::set(tag, "Color", Tag::Byte(10));
                report.applied(version, "EntityShulkerColorFix");
            }
        }
        813 if !entity && named(nbt::get(tag, "id"), "minecraft:shulker_box") => {
            if nbt::get(tag, "Color").is_some() {
                nbt::remove(tag, "Color");
                report.applied(version, "BlockEntityShulkerBoxColorFix");
            }
        }
        1451 if !entity && named(nbt::get(tag, "id"), "minecraft:piston") => {
            if piston_block_state(tag) {
                report.applied(version, "BlockEntityBlockStateFix");
            }
        }
        // 1451r3 EntityBlockStateFix and r5 EntityWolfColorFix share one entity
        // arm in registration order so no arm shadows another. VillagerTradeFix
        // (r7) runs ONLY in after_children below: trade ids created by the item
        // flattening (r4) need the post-child pass.
        1451 if entity => {
            if falling_block(tag)
                | minecart_display(tag)
                | enderman_carried(tag)
                | arrow_in_block(tag)
                | remove_throwable_block(tag)
            {
                report.applied(version, "EntityBlockStateFix");
            }
            if named(nbt::get(tag, "id"), "minecraft:wolf")
                && let Some(color) = nbt::get(tag, "CollarColor")
            {
                let mapped = 15 - nbt::int(color).unwrap_or(0);
                nbt::set(tag, "CollarColor", Tag::Byte(mapped as i8));
                report.applied(version, "EntityWolfColorFix");
            }
            // 1451r5 RemoveBlockEntityTagFix, ENTITY branch: falling_block
            // TileEntityData whose nested id is noteblock/flower_pot is
            // dropped (exact class semantics: the TAG's id gates, not the
            // outer entity). Runs after the numeric state rewrites above so
            // the TileEntityData id comparison sees post-1451 names.
            if named(nbt::get(tag, "id"), "minecraft:falling_block")
                && remove_block_entity_data(tag, "TileEntityData")
            {
                report.applied(version, "RemoveBlockEntityTagFix");
            }
        }
        1451 if !entity && named(nbt::get(tag, "id"), "minecraft:jukebox") => {
            if jukebox(tag) {
                report.applied(version, "BlockEntityJukeboxFix");
            }
        }
        1451 if !entity && named(nbt::get(tag, "id"), "minecraft:banner") => {
            if banner_colors(tag) {
                report.applied(version, "BlockEntityBannerColorFix");
            }
        }
        1456 if entity && named(nbt::get(tag, "id"), "minecraft:item_frame") => {
            let facing = nbt::get(tag, "Facing").and_then(nbt::int).unwrap_or(0) as i8;
            nbt::set(tag, "Facing", Tag::Byte(direction_2d_to_3d(facing)));
            report.applied(version, "EntityItemFrameDirectionFix");
        }
        1458 if entity
            && !named(nbt::get(tag, "id"), "minecraft:commandblock_minecart")
            && custom_name_to_component(tag) =>
        {
            report.applied(version, "EntityCustomNameToComponentFix");
        }
        1458 if !entity
            && matches!(
                nbt::get(tag, "id").and_then(nbt::string),
                Some(
                    "minecraft:banner"
                        | "minecraft:beacon"
                        | "minecraft:brewing_stand"
                        | "minecraft:chest"
                        | "minecraft:dispenser"
                        | "minecraft:dropper"
                        | "minecraft:enchanting_table"
                        | "minecraft:furnace"
                        | "minecraft:hopper"
                        | "minecraft:shulker_box"
                        | "minecraft:trapped_chest"
                )
            )
            && custom_name_to_component(tag) =>
        {
            report.applied(version, "BlockEntityCustomNameToComponentFix");
        }
        1460 if entity && named(nbt::get(tag, "id"), "minecraft:painting") => {
            if painting_motive(tag) {
                report.applied(version, "EntityPaintingMotiveFix");
            }
        }
        1474 if entity && named(nbt::get(tag, "id"), "minecraft:shulker") => {
            if colorless_shulker(tag) {
                report.applied(version, "ColorlessShulkerEntityFix");
            }
        }
        // 1488 anonymous DataFixers$1: command_block BE CustomName via
        // fixTagCustomName (same JSON-string conversion, pre-1458 nameables).
        1488 if !entity
            && named(nbt::get(tag, "id"), "minecraft:command_block")
            && custom_name_to_component(tag) =>
        {
            report.applied(version, "CommandBlockCustomNameFix");
        }
        // 1488 anonymous DataFixers$2: commandblock_minecart entity CustomName
        // plain-component conversion (the 1458 exclusion does not apply here).
        1488 if entity
            && named(nbt::get(tag, "id"), "minecraft:commandblock_minecart")
            && custom_name_to_component(tag) =>
        {
            report.applied(version, "CommandBlockMinecartCustomNameFix");
        }
        1483 if entity => {
            if named(nbt::get(tag, "id"), "minecraft:puffer_fish") {
                set_id(tag, "minecraft:pufferfish");
                report.applied(version, "EntityPufferfishRenameFix");
            }
        }
        1486 if entity => {
            if named(nbt::get(tag, "id"), "minecraft:cod_mob") {
                set_id(tag, "minecraft:cod");
                report.applied(version, "EntityCodSalmonFix");
            } else if named(nbt::get(tag, "id"), "minecraft:salmon_mob") {
                set_id(tag, "minecraft:salmon");
                report.applied(version, "EntityCodSalmonFix");
            }
        }
        1500 if !entity && nbt::get(tag, "id").and_then(nbt::string) == Some("DUMMY") => {
            nbt::set(tag, "keepPacked", Tag::Byte(1));
            report.applied(version, "BlockEntityKeepPacked");
        }
        1510 if entity => {
            // Exact rename(): strip minecraft:bred_ then prepend minecraft:,
            // then RENAMED_IDS.getOrDefault (12 pairs, counted from the static
            // block lines 42-76 of entity-EntityTheRenameningFix.javap).
            let current = nbt::get(tag, "id").and_then(nbt::string).unwrap_or("");
            let stripped = current
                .strip_prefix("minecraft:bred_")
                .map(|rest| format!("minecraft:{rest}"));
            let lookup = stripped.as_deref().unwrap_or(current);
            if let Some(mapped) = the_renamening(lookup) {
                set_id(tag, mapped);
                report.applied(version, "EntityTheRenameningFix");
            } else if let Some(stripped) = stripped {
                set_id(tag, &stripped);
                report.applied(version, "EntityTheRenameningFix");
            }
        }
        1904 if entity && named(nbt::get(tag, "id"), "minecraft:ocelot") => {
            if cat_split(tag) {
                report.applied(version, "EntityCatSplitFix");
            }
        }
        1917 if entity && named(nbt::get(tag, "id"), "minecraft:cat") => {
            if nbt::get(tag, "CatType").and_then(nbt::int).unwrap_or(0) == 9 {
                nbt::set(tag, "CatType", Tag::Int(10));
                report.applied(version, "CatTypeFix");
            }
        }
        1918 if entity
            && (named(nbt::get(tag, "id"), "minecraft:villager")
                || named(nbt::get(tag, "id"), "minecraft:zombie_villager")) =>
        {
            if villager_data(tag) {
                report.applied(version, "VillagerDataFix");
            }
        }
        1914 if !entity && named(nbt::get(tag, "id"), "minecraft:chest") => {
            if nbt::get(tag, "LootTable").and_then(nbt::string)
                == Some("minecraft:chests/village_blacksmith")
            {
                nbt::set(
                    tag,
                    "LootTable",
                    text("minecraft:chests/village/village_weaponsmith"),
                );
                report.applied(version, "WeaponSmithChestLootTableFix");
            }
        }
        1928 if entity && named(nbt::get(tag, "id"), "minecraft:illager_beast") => {
            set_id(tag, "minecraft:ravager");
            report.applied(version, "EntityRavagerRenameFix");
        }
        1955 if entity && named(nbt::get(tag, "id"), "minecraft:villager") => {
            if villager_rebuild(tag) {
                report.applied(version, "VillagerRebuildLevelAndXpFix");
            }
        }
        1955 if entity && named(nbt::get(tag, "id"), "minecraft:zombie_villager") => {
            if zombie_villager_rebuild(tag) {
                report.applied(version, "ZombieVillagerRebuildXpFix");
            }
        }
        1953 if !entity && named(nbt::get(tag, "id"), "minecraft:banner") => {
            if ominous_banner(tag) {
                report.applied(version, "OminousBannerBlockEntityRenameFix");
            }
        }
        1963 if entity && named(nbt::get(tag, "id"), "minecraft:villager") => {
            if remove_golem_gossip(tag) {
                report.applied(version, "RemoveGolemGossipFix");
            }
        }
        2501 if !entity
            && matches!(
                nbt::get(tag, "id").and_then(nbt::string),
                Some("minecraft:furnace" | "minecraft:blast_furnace" | "minecraft:smoker")
            ) =>
        {
            if furnace_recipes(tag) {
                report.applied(version, "FurnaceRecipeFix");
            }
        }
        2505 if entity && named(nbt::get(tag, "id"), "minecraft:villager") => {
            if memory_expiry(tag) {
                report.applied(version, "MemoryExpiryDataFix");
            }
        }
        2509 if entity && named(nbt::get(tag, "id"), "minecraft:zombie_pigman") => {
            set_id(tag, "minecraft:zombified_piglin");
            report.applied(version, "EntityZombifiedPiglinRenameFix");
        }
        2511 if entity => {
            if report.from < 2511
                && matches!(
                    nbt::get(tag, "id").and_then(nbt::string),
                    Some(
                        "minecraft:egg"
                            | "minecraft:ender_pearl"
                            | "minecraft:experience_bottle"
                            | "minecraft:snowball"
                            | "minecraft:potion"
                            | "minecraft:llama_spit"
                            | "minecraft:arrow"
                            | "minecraft:spectral_arrow"
                            | "minecraft:trident"
                    )
                )
                && projectile_owner(tag)
            {
                report.applied(version, "EntityProjectileOwnerFix");
            }
            // Revision1 casts the same optional typed value into V2511_1's
            // renamed field. It runs even for integer source2511 and after
            // a successful revision0 owner conversion.
            if named(nbt::get(tag, "id"), "minecraft:potion") && nbt::get(tag, "Potion").is_some() {
                nbt::rename(tag, "Potion", "Item");
                report.applied(version, "SplashPotionItemFieldRenameFix");
            }
        }
        2514 if entity => {
            if entity_uuid(tag) {
                report.applied(version, "EntityUUIDFix");
            }
        }
        2514 if !entity => {
            if block_entity_uuid(tag) {
                report.applied(version, "BlockEntityUUIDFix");
            }
        }
        2516 if entity
            && (named(nbt::get(tag, "id"), "minecraft:villager")
                || named(nbt::get(tag, "id"), "minecraft:zombie_villager"))
            && gossip_uuid(tag) =>
        {
            report.applied(version, "GossipUUIDFix");
        }
        // JigsawPropertiesFix targets the BE root; JigsawRotationFix targets
        // typed BLOCK_STATE and belongs to old_block_states::fix_node.
        2518 if !entity && named(nbt::get(tag, "id"), "minecraft:jigsaw") => {
            if jigsaw_properties(tag) {
                report.applied(version, "JigsawPropertiesFix");
            }
        }
        2523 if entity && old_player_map::rename_attributes(tag) => {
            report.applied(version, "AttributesRenameLegacy");
        }
        2529 if entity && named(nbt::get(tag, "id"), "minecraft:strider") => {
            if nbt::get(tag, "NoGravity")
                .and_then(nbt::number)
                .is_some_and(|v| v as i8 != 0)
            {
                nbt::set(tag, "NoGravity", Tag::Byte(0));
                report.applied(version, "StriderGravityFix");
            }
        }
        2533 if entity && named(nbt::get(tag, "id"), "minecraft:villager") => {
            if villager_follow_range(tag) {
                report.applied(version, "VillagerFollowRangeFix");
            }
        }
        2535 if entity
            && named(nbt::get(tag, "id"), "minecraft:shulker")
            && shulker_rotation(tag) =>
        {
            report.applied(version, "EntityShulkerRotationFix");
        }
        _ => {}
    }
    Ok(())
}

/// Post-child hook for the 1451 VillagerTradeFix (r7): runs AFTER the walker's
/// item children (including the r4 flattening) so trade ids created by that
/// flattening are visible. Vanilla targets the minecraft:villager choice.
pub(super) fn after_children(
    tag: &mut Tag,
    kind: NodeKind,
    version: i32,
    report: &mut UpgradeReport,
) -> Result<(), UpgradeError> {
    if version == 1451
        && kind == NodeKind::Entity
        && named(nbt::get(tag, "id"), "minecraft:villager")
        && villager_trade(tag)
    {
        report.applied(version, "VillagerTradeFix");
    }
    Ok(())
}

fn is_zero_float_list(tag: &Tag, key: &str, len: usize) -> bool {
    let Some(list) = nbt::get(tag, key).and_then(nbt::list) else {
        return false;
    };
    list.len() == len && list.iter().all(|v| nbt::number(v).unwrap_or(1.0) == 0.0)
}

#[cfg(test)]
mod tests;
