//! Position fields, inline positions and entity position rewrites.

use super::*;

fn block_pos(value: &mut Tag) {
    if let (Some(x), Some(y), Some(z)) = (
        get(value, "X").and_then(java_int),
        get(value, "Y").and_then(java_int),
        get(value, "Z").and_then(java_int),
    ) {
        *value = Tag::IntArray(vec![x, y, z]);
    }
}

fn pos_field(tag: &mut Tag, old: &str, new: &str) -> bool {
    if let Some(value) = get_mut(tag, old) {
        block_pos(value);
        rename(tag, old, new);
        true
    } else {
        false
    }
}

pub(super) fn entity_positions(tag: &mut Tag) -> bool {
    let mut changed = false;
    if named(get(tag, "id"), "minecraft:bee") {
        changed |= pos_field(tag, "HivePos", "hive_pos");
        changed |= pos_field(tag, "FlowerPos", "flower_pos");
    }
    if named(get(tag, "id"), "minecraft:end_crystal") {
        changed |= pos_field(tag, "BeamTarget", "beam_target");
    }
    if named(get(tag, "id"), "minecraft:wandering_trader") {
        changed |= pos_field(tag, "WanderTarget", "wander_target");
    }
    if [
        "witch",
        "ravager",
        "pillager",
        "illusioner",
        "evoker",
        "vindicator",
    ]
    .iter()
    .any(|id| {
        get(tag, "id")
            .and_then(string)
            .is_some_and(|actual| actual.strip_prefix("minecraft:").unwrap_or(actual) == *id)
    }) {
        changed |= pos_field(tag, "PatrolTarget", "patrol_target");
    }
    changed | pos_field(tag, "Leash", "leash")
}

fn inline_pos(tag: &mut Tag, keys: [&str; 3], new: &str) -> bool {
    let values = keys.map(|key| get(tag, key).and_then(java_int));
    if let [Some(x), Some(y), Some(z)] = values {
        for key in keys {
            remove(tag, key);
        }
        set(tag, new, Tag::IntArray(vec![x, y, z]));
        true
    } else {
        false
    }
}

pub(super) fn inline_positions(tag: &mut Tag, player: bool) -> bool {
    let mut changed = inline_pos(tag, ["SleepingX", "SleepingY", "SleepingZ"], "sleeping_pos");
    if player {
        let values = ["SpawnX", "SpawnY", "SpawnZ"].map(|key| get(tag, key).and_then(java_int));
        if let [Some(x), Some(y), Some(z)] = values {
            let mut respawn = compound(vec![("pos", Tag::IntArray(vec![x, y, z]))]);
            for (old, new) in [
                ("SpawnAngle", "angle"),
                ("SpawnDimension", "dimension"),
                ("SpawnForced", "forced"),
            ] {
                if let Some(value) = take(tag, old) {
                    set(&mut respawn, new, value);
                }
            }
            for key in ["SpawnX", "SpawnY", "SpawnZ"] {
                remove(tag, key);
            }
            set(tag, "respawn", respawn);
            changed = true;
        }
        if let Some(position) = take(tag, "enteredNetherPosition") {
            let values = ["x", "y", "z"]
                .map(|key| Tag::Double(get(&position, key).and_then(number).unwrap_or(0.0)));
            set(
                tag,
                "entered_nether_pos",
                Tag::List(ListTag {
                    element_type: 6,
                    elements: Vec::from(values),
                }),
            );
            changed = true;
        }
    } else if named(get(tag, "id"), "minecraft:vex") {
        changed |= get(tag, "LifeTicks").is_some();
        rename(tag, "LifeTicks", "life_ticks");
        changed |= inline_pos(tag, ["BoundX", "BoundY", "BoundZ"], "bound_pos");
    } else if named(get(tag, "id"), "minecraft:phantom") {
        changed |= get(tag, "Size").is_some();
        rename(tag, "Size", "size");
        changed |= inline_pos(tag, ["AX", "AY", "AZ"], "anchor_pos");
    } else if named(get(tag, "id"), "minecraft:turtle") {
        changed |= has_any(tag, &["TravelPosX", "TravelPosY", "TravelPosZ", "HasEgg"]);
        for key in ["TravelPosX", "TravelPosY", "TravelPosZ"] {
            remove(tag, key);
        }
        rename(tag, "HasEgg", "has_egg");
        changed |= inline_pos(tag, ["HomePosX", "HomePosY", "HomePosZ"], "home_pos");
    } else if ["item_frame", "glow_item_frame", "painting", "leash_knot"]
        .iter()
        .any(|id| {
            get(tag, "id")
                .and_then(string)
                .is_some_and(|actual| actual.strip_prefix("minecraft:").unwrap_or(actual) == *id)
        })
    {
        changed |= inline_pos(tag, ["TileX", "TileY", "TileZ"], "block_pos");
    }
    changed
}
