//! Block-entity payloads, block states and banner/jukebox/shulker fields.

use super::*;

/// v1451.2 BlockEntityBlockStateFix (piston only): blockId/blockData ->
/// blockState via BlockStateData.getTag(id<<4 | data&15); fields removed.
/// `state()` returns MAP[0] (air) for invalid ids, exactly like vanilla.
pub(super) fn piston_block_state(tag: &mut Tag) -> bool {
    let id = nbt::get(tag, "blockId").and_then(nbt::int).unwrap_or(0);
    let data = nbt::get(tag, "blockData").and_then(nbt::int).unwrap_or(0) & 15;
    nbt::remove(tag, "blockId");
    nbt::remove(tag, "blockData");
    nbt::set(
        tag,
        "blockState",
        super::super::old_block_states::state(id << 4 | data),
    );
    true
}

/// v1451.3 EntityBlockStateFix falling block: TileID/Data/Tile + Block/Data
/// -> BlockState via BlockStateData.getTag; Data/TileID/Tile removed.
/// Numeric BlockStateData ids are POST-flattening: never run `upgrade` on the
/// freshly resolved state.
pub(super) fn falling_block(tag: &mut Tag) -> bool {
    if !named(nbt::get(tag, "id"), "minecraft:falling_block") {
        return false;
    }
    update_falling_block(tag)
}

/// Exact updateFallingBlock: prefer typed Block, otherwise numeric TileID,
/// otherwise the unsigned Tile byte; all three forms share the Data nibble.
fn update_falling_block(tag: &mut Tag) -> bool {
    let data = nbt::get(tag, "Data").and_then(nbt::int).unwrap_or(0) & 15;
    let id = if let Some(block) = nbt::take(tag, "Block") {
        match block {
            Tag::String(name) => super::super::old_block_states::block_id(&name),
            other => nbt::int(&other).unwrap_or(0),
        }
    } else {
        nbt::get(tag, "TileID")
            .and_then(nbt::number)
            .map(|value| value as i32)
            .unwrap_or_else(|| {
                nbt::get(tag, "Tile").and_then(nbt::number).unwrap_or(0.0) as i32 & 255
            })
    };
    for key in ["Data", "TileID", "Tile"] {
        nbt::remove(tag, key);
    }
    nbt::set(
        tag,
        "BlockState",
        super::super::old_block_states::state(id << 4 | data),
    );
    true
}

pub(super) fn enderman_carried(tag: &mut Tag) -> bool {
    if !named(nbt::get(tag, "id"), "minecraft:enderman") {
        return false;
    }
    block_to_state(tag, "carried", "carriedData", "carriedBlockState")
}

pub(super) fn arrow_in_block(tag: &mut Tag) -> bool {
    if !matches!(
        nbt::get(tag, "id").and_then(nbt::string),
        Some("minecraft:arrow" | "minecraft:spectral_arrow")
    ) {
        return false;
    }
    block_to_state(tag, "inTile", "inData", "inBlockState")
}

pub(super) fn remove_throwable_block(tag: &mut Tag) -> bool {
    matches!(
        nbt::get(tag, "id").and_then(nbt::string),
        Some(
            "minecraft:egg"
                | "minecraft:ender_pearl"
                | "minecraft:fireball"
                | "minecraft:potion"
                | "minecraft:small_fireball"
                | "minecraft:snowball"
                | "minecraft:wither_skull"
                | "minecraft:xp_bottle"
        )
    ) && nbt::take(tag, "inTile").is_some()
}

pub(super) fn block_to_state(
    tag: &mut Tag,
    block_key: &str,
    data_key: &str,
    state_key: &str,
) -> bool {
    let Some(block) = nbt::take(tag, block_key) else {
        return nbt::take(tag, data_key).is_some();
    };
    let data = nbt::get(tag, data_key).and_then(nbt::int).unwrap_or(0) & 15;
    let id = match block {
        Tag::String(name) => super::super::old_block_states::block_id(&name),
        other => nbt::int(&other).unwrap_or(0),
    };
    nbt::remove(tag, data_key);
    nbt::set(
        tag,
        state_key,
        super::super::old_block_states::state(id << 4 | data),
    );
    true
}
/// v1451.6 BlockEntityJukeboxFix: convert positive Record through the full
/// ItemIdFix/flattening tables and overwrite RecordItem only on a match.
/// Vanilla discards Dynamic.remove(Record)'s return value, preserving Record.
pub(super) fn jukebox(tag: &mut Tag) -> bool {
    let record = nbt::get(tag, "Record").and_then(nbt::int).unwrap_or(0);
    if record <= 0 {
        return false;
    }
    let Some(flat) = old_items::flattened_item(old_items::numeric_item(record), 0) else {
        return false;
    };
    nbt::set(
        tag,
        "RecordItem",
        nbt::compound(vec![("id", text(flat)), ("Count", Tag::Byte(1))]),
    );
    true
}

/// v1451 BlockEntityBannerColorFix: Base/Patterns[].Color 15-x -> int color.
pub(super) fn banner_colors(tag: &mut Tag) -> bool {
    let mut changed = false;
    if let Some(base) = nbt::get(tag, "Base").and_then(nbt::int) {
        nbt::set(tag, "Base", Tag::Int(15 - base));
        changed = true;
    }
    if let Some(patterns) = nbt::get_mut(tag, "Patterns").and_then(nbt::list_mut) {
        for pattern in patterns.iter_mut() {
            if let Some(color) = nbt::get(pattern, "Color").and_then(nbt::int) {
                nbt::set(pattern, "Color", Tag::Int(15 - color));
                changed = true;
            }
        }
    }
    changed
}

/// v1953 OminousBannerBlockEntityRenameFix (banner BE): CustomName text
/// component with translate block.minecraft.illager_banner -> ominous_banner.
pub(super) fn ominous_banner(tag: &mut Tag) -> bool {
    let Some(name) = nbt::get(tag, "CustomName")
        .and_then(nbt::string)
        .map(str::to_owned)
    else {
        return false;
    };
    if !name.contains("block.minecraft.illager_banner") {
        return false;
    }
    nbt::set(
        tag,
        "CustomName",
        Tag::String(name.replace(
            "block.minecraft.illager_banner",
            "block.minecraft.ominous_banner",
        )),
    );
    true
}

/// v1474 ColorlessShulkerEntityFix: Color 10 -> 16.
pub(super) fn colorless_shulker(tag: &mut Tag) -> bool {
    if nbt::get(tag, "Color").and_then(nbt::int).unwrap_or(-1) == 10 {
        nbt::set(tag, "Color", Tag::Byte(16));
        return true;
    }
    false
}

/// v1451r5 RemoveBlockEntityTagFix, ENTITY falling_block branch: drop
/// TileEntityData (key `field`) when its nested `id` is minecraft:noteblock
/// or minecraft:flower_pot. The nested tag's id gates removal; by this version
/// its pre704 discriminator has already been upgraded. The structure-block
/// branch is not PLAYER-reachable.
pub(super) fn remove_block_entity_data(tag: &mut Tag, field: &str) -> bool {
    let drop = matches!(
        nbt::get(tag, field)
            .and_then(|data| nbt::get(data, "id"))
            .and_then(nbt::string),
        Some("minecraft:noteblock" | "minecraft:flower_pot")
    );
    if !drop {
        return false;
    }
    nbt::remove(tag, field);
    true
}

/// v2501 FurnaceRecipeFix (furnace/blast_furnace/smoker BEs):
/// RecipesUsedSize N + RecipeLocation{i}/RecipeAmount{i} (i in 0..N) fold
/// into RecipesUsed{location: amount}; RecipesUsedSize removed. Only
/// positive amounts with a present location string are kept
/// (vanilla reads the amount with default 0 and skips <= 0, and skips
/// missing locations via Optional.ifPresent). Indexed keys are removed
/// unconditionally once consumed (vanilla removes both keys per index even
/// when the entry is skipped). RECIPE-typed conversion is schema-level
/// (no payload shape change here).
pub(super) fn furnace_recipes(tag: &mut Tag) -> bool {
    // Vanilla updateFurnaceContents runs on EVERY furnace/blast_furnace/smoker
    // BE at 2501 (fixTypeEverywhereTyped over the three named choices): the
    // size key is always removed and RecipesUsed always set, even when empty.
    let size = nbt::get(tag, "RecipesUsedSize")
        .and_then(nbt::int)
        .unwrap_or(0);
    nbt::remove(tag, "RecipesUsedSize");
    let mut used: Vec<(String, Tag)> = Vec::new();
    for i in 0..size.max(0) {
        let location_key = format!("RecipeLocation{i}");
        let amount_key = format!("RecipeAmount{i}");
        let location = nbt::get(tag, &location_key)
            .and_then(nbt::string)
            .map(str::to_owned);
        let amount = nbt::get(tag, &amount_key).and_then(nbt::int).unwrap_or(0);
        nbt::remove(tag, &location_key);
        nbt::remove(tag, &amount_key);
        if amount <= 0 {
            continue;
        }
        if let Some(location) = location {
            used.push((location, Tag::Int(amount)));
        }
    }
    nbt::set(tag, "RecipesUsed", Tag::Compound(used));
    true
}

/// v2518 JigsawPropertiesFix: name=attachement_type ALWAYS (unconditional
/// overwrite), target=attachement_type, pool=target_pool; missing names
/// default to minecraft:empty.
pub(super) fn jigsaw_properties(tag: &mut Tag) -> bool {
    let attachment = nbt::get(tag, "attachement_type")
        .and_then(nbt::string)
        .unwrap_or("minecraft:empty")
        .to_owned();
    let pool = nbt::get(tag, "target_pool")
        .and_then(nbt::string)
        .unwrap_or("minecraft:empty")
        .to_owned();
    nbt::set(tag, "name", text(&attachment));
    nbt::set(tag, "target", text(&attachment));
    nbt::remove(tag, "attachement_type");
    nbt::set(tag, "pool", text(&pool));
    nbt::remove(tag, "target_pool");
    true
}

/// v2535 EntityShulkerRotationFix: rebuild the complete list as doubles and
/// subtract 180 from its first element (non-numeric elements default to 180).
pub(super) fn shulker_rotation(tag: &mut Tag) -> bool {
    let Some(Tag::List(rotation)) = nbt::get_mut(tag, "Rotation") else {
        return false;
    };
    if rotation.elements.is_empty() {
        return false;
    }
    for (index, value) in rotation.elements.iter_mut().enumerate() {
        let angle = nbt::number(value).unwrap_or(180.0);
        *value = Tag::Double(if index == 0 { angle - 180.0 } else { angle });
    }
    rotation.element_type = mc_nbt::tag_type::DOUBLE;
    true
}
