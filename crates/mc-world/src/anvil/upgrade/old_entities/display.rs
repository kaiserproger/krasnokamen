//! Entity geometry and text payloads: minecart, painting, sign, name.

use super::*;

use super::blocks::block_to_state;

pub(super) fn direction_2d_to_3d(facing: i8) -> i8 {
    match facing {
        0 => 3,
        1 => 4,
        3 => 5,
        _ => 2,
    }
}

/// v111 EntityPaintingItemFrameDirectionFix: the numeric Direction branch
/// shifts tiles and doubles ItemRotation only for item frames. Otherwise the
/// legacy Dir branch supplies Facing without shifting or changing rotation.
pub(super) fn painting_direction(tag: &mut Tag) -> bool {
    let id = nbt::get(tag, "id").and_then(nbt::string).unwrap_or("");
    if !matches!(id, "Painting" | "ItemFrame")
        || nbt::get(tag, "Facing").and_then(nbt::number).is_some()
    {
        return false;
    }
    let frame = id == "ItemFrame";
    if nbt::get(tag, "Direction").and_then(nbt::number).is_some() {
        shift_tiles(tag);
        if frame && let Some(rotation) = nbt::get(tag, "ItemRotation").and_then(nbt::number) {
            nbt::set(
                tag,
                "ItemRotation",
                Tag::Byte((rotation as i8).wrapping_mul(2)),
            );
        }
    } else {
        let dir = nbt::get(tag, "Dir").and_then(nbt::int).unwrap_or(0) as i8;
        nbt::remove(tag, "Dir");
        nbt::set(tag, "Facing", Tag::Byte(dir % 4));
    }
    true
}

/// Tile shift + Facing from Direction (shared by the painting path and the
/// frame Direction path).
fn shift_tiles(tag: &mut Tag) {
    let dir = nbt::get(tag, "Direction").and_then(nbt::int).unwrap_or(0) as i8;
    let index = dir.rem_euclid(4);
    const OFFSETS: [[i32; 3]; 4] = [[0, 0, 1], [-1, 0, 0], [0, 0, -1], [1, 0, 0]];
    let [dx, dy, dz] = OFFSETS[index as usize];
    for (key, delta) in [("TileX", dx), ("TileY", dy), ("TileZ", dz)] {
        let base = nbt::get(tag, key).and_then(nbt::int).unwrap_or(0);
        nbt::set(tag, key, Tag::Int(base.wrapping_add(delta)));
    }
    nbt::remove(tag, "Direction");
    nbt::set(tag, "Facing", Tag::Byte(index));
}

pub(super) fn minecart_display(tag: &mut Tag) -> bool {
    if !matches!(
        nbt::get(tag, "id").and_then(nbt::string),
        Some(
            "minecraft:minecart"
                | "minecraft:chest_minecart"
                | "minecraft:tnt_minecart"
                | "minecraft:spawner_minecart"
                | "minecraft:hopper_minecart"
                | "minecraft:furnace_minecart"
                | "minecraft:commandblock_minecart"
        )
    ) {
        return false;
    }
    block_to_state(tag, "DisplayTile", "DisplayData", "DisplayState")
}

/// v165 SignTextStrictJsonFix (BE Sign): Text1..4 through
/// rewriteFromLenient. Empty/"null" inputs become EMPTY_CONTENTS
/// (createTextComponentJson("") = {"text":""}); quoted/braced/bracketed
/// inputs parse as lenient JSON (primitives re-wrap as text, compounds
/// stabilize); unparseable inputs wrap as plain text.
pub(super) fn sign_text(tag: &mut Tag) -> bool {
    let mut changed = false;
    for key in ["Text1", "Text2", "Text3", "Text4"] {
        if rewrite_lenient(tag, key) {
            changed = true;
        }
    }
    changed
}

fn rewrite_lenient(tag: &mut Tag, key: &str) -> bool {
    let Some(Tag::String(current)) = nbt::get_mut(tag, key) else {
        return false;
    };
    *current = old_items::rewrite_from_lenient(current);
    true
}

/// v1458 plain-text CustomName -> JSON string {"text":"name"}; empty names
/// removed. (LegacyComponentDataFixUtils.createPlainTextComponent output is a
/// STRING holding JSON, which the 4290 text chain parses.)
pub(super) fn custom_name_to_component(tag: &mut Tag) -> bool {
    let Some(Tag::String(name)) = nbt::get_mut(tag, "CustomName") else {
        return false;
    };
    if name.is_empty() {
        nbt::remove(tag, "CustomName");
        return true;
    }
    *name = old_items::create_text_component_json(name);
    true
}

/// v1460 EntityPaintingMotiveFix: Motive lowercased, three legacy spellings
/// remapped, then namespaced.
pub(super) fn painting_motive(tag: &mut Tag) -> bool {
    let Some(motive) = nbt::get(tag, "Motive")
        .and_then(nbt::string)
        .map(str::to_owned)
    else {
        return false;
    };
    let lower = motive.to_ascii_lowercase();
    let mapped = match lower.as_str() {
        "donkeykong" => "donkey_kong",
        "burningskull" => "burning_skull",
        "skullandroses" => "skull_and_roses",
        _ => lower.as_str(),
    };
    let namespaced = if mapped.contains(':') {
        mapped.into()
    } else {
        format!("minecraft:{mapped}")
    };
    nbt::set(tag, "Motive", text(&namespaced));
    true
}
