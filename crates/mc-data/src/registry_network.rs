//! Native `RegistryData` payloads: one synchronized registry entry's
//! data-pack JSON encoded as the Network NBT a vanilla server sends for it when
//! the client declines Known Packs (ADR 0022,
//! `docs/decisions/0022-content-without-java.md`).
//!
//! Vanilla decodes the entry with the registry's full codec and re-encodes it
//! with its network codec into `NbtOps`. The data-pack JSON already has the
//! codec's shape, so the payload follows from four protocol rules, each a table
//! below:
//!
//! - network codecs omit some fields (`NETWORK_OMITTED_FIELDS`) and the
//!   environment attributes vanilla never syncs (`SERVER_ONLY_ATTRIBUTES`,
//!   filtered from the maps in `ATTRIBUTE_MAPS`);
//! - a JSON integer is an `Int`, a JSON fraction a `Float`, a boolean a `Byte`;
//!   the few fields whose codec is wider are typed in `WIDE_FIELDS`;
//! - `CompoundTag` is a `java.util.HashMap` that started at capacity 16, so
//!   keys are written in bucket order of Java's `String.hashCode`, and keys
//!   sharing a bucket in insertion (codec field) order, given for every
//!   colliding pair in `COLLIDING_FIELD_ORDER`;
//! - numbers are rounded from their JSON text as `Float.parseFloat` and
//!   `Double.parseDouble` round it (the `json` reader keeps that text).
//!
//! Vanilla itself writes environment-attribute maps in a per-JVM-run order
//! (`Map.copyOf` of identity-hashed keys), so two captures of the same release
//! can differ in the order of attribute keys sharing a bucket; the encoder
//! writes those in data-pack order. The maintainer check in the tests compares
//! every payload with a vanilla capture byte for byte and accepts only that
//! permutation.

mod json;

use mc_nbt::{ListTag, Tag, tag_type};
use thiserror::Error;

use self::json::Json;
pub use self::json::JsonError;

/// Fields a registry's network codec leaves out of its data-pack form.
const NETWORK_OMITTED_FIELDS: &[(&str, &str)] = &[
    ("cat_variant", "spawn_conditions"),
    ("chicken_variant", "spawn_conditions"),
    ("cow_variant", "spawn_conditions"),
    ("frog_variant", "spawn_conditions"),
    ("pig_variant", "spawn_conditions"),
    ("wolf_variant", "spawn_conditions"),
    ("zombie_nautilus_variant", "spawn_conditions"),
    ("worldgen/biome", "carvers"),
    ("worldgen/biome", "creature_spawn_probability"),
    ("worldgen/biome", "features"),
    ("worldgen/biome", "spawn_costs"),
    ("worldgen/biome", "spawners"),
];

/// Environment attributes registered without `syncable()` in vanilla's
/// `EnvironmentAttributes` (26.1.2 and 26.3; the last three exist only in 26.3).
const SERVER_ONLY_ATTRIBUTES: &[&str] = &[
    "minecraft:gameplay/baby_villager_activity",
    "minecraft:gameplay/bed_rule",
    "minecraft:gameplay/bees_stay_in_hive",
    "minecraft:gameplay/can_pillager_patrol_spawn",
    "minecraft:gameplay/can_start_raid",
    "minecraft:gameplay/cat_waking_up_gift_chance",
    "minecraft:gameplay/eyeblossom_open",
    "minecraft:gameplay/increased_fire_burnout",
    "minecraft:gameplay/monsters_burn",
    "minecraft:gameplay/nether_portal_spawns_piglin",
    "minecraft:gameplay/respawn_anchor_works",
    "minecraft:gameplay/snow_golem_melts",
    "minecraft:gameplay/surface_slime_spawn_chance",
    "minecraft:gameplay/turtle_egg_hatch_chance",
    "minecraft:gameplay/villager_activity",
    "minecraft:gameplay/creature_world_gen_spawn_probability",
    "minecraft:gameplay/natural_mob_spawns",
    "minecraft:gameplay/straw_bed_rule",
];

/// Top-level attribute maps the network codec filters by
/// [`SERVER_ONLY_ATTRIBUTES`]: `(registry, field, kept when emptied)`. A biome
/// or dimension keeps an emptied map because the field's presence is decided
/// before the filter; a timeline filters first and omits an empty `tracks`.
pub(crate) const ATTRIBUTE_MAPS: &[(&str, &str, bool)] = &[
    ("dimension_type", "attributes", true),
    ("timeline", "tracks", false),
    ("worldgen/biome", "attributes", true),
];

/// An NBT type wider than the JSON form implies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wide {
    Double,
    Long,
    /// `Vec3i`/`BlockPos` codecs (`Codec.INT_STREAM`).
    IntArray,
}

/// Fields whose codec is a double, long or int stream. Paths are object keys
/// and `[]` for a list element; `**` matches any run of segments and `*` one.
const WIDE_FIELDS: &[(&str, &[&str], Wide)] = &[
    ("dimension_type", &["coordinate_scale"], Wide::Double),
    (
        "dimension_type",
        &[
            "attributes",
            "minecraft:audio/ambient_sounds",
            "mood",
            "offset",
        ],
        Wide::Double,
    ),
    (
        "worldgen/biome",
        &[
            "attributes",
            "minecraft:audio/ambient_sounds",
            "mood",
            "offset",
        ],
        Wide::Double,
    ),
    (
        "worldgen/biome",
        &[
            "attributes",
            "minecraft:audio/ambient_sounds",
            "additions",
            "tick_chance",
        ],
        Wide::Double,
    ),
    // Entity movement predicates: `minecraft:movement` in 26.3, `movement` in 26.1.2.
    (
        "enchantment",
        &["**", "minecraft:movement", "*", "min"],
        Wide::Double,
    ),
    (
        "enchantment",
        &["**", "minecraft:movement", "*", "max"],
        Wide::Double,
    ),
    ("enchantment", &["**", "movement", "*", "min"], Wide::Double),
    ("enchantment", &["**", "movement", "*", "max"], Wide::Double),
    // `apply_impulse` vectors.
    (
        "enchantment",
        &["**", "effect", "effects", "[]", "coordinate_scale", "[]"],
        Wide::Double,
    ),
    (
        "enchantment",
        &["**", "effect", "effects", "[]", "direction", "[]"],
        Wide::Double,
    ),
    ("enchantment", &["**", "effect", "offset"], Wide::IntArray),
    (
        "enchantment",
        &["**", "predicates", "[]", "offset"],
        Wide::IntArray,
    ),
    (
        "block_transformer",
        &["**", "predicates", "[]", "offset"],
        Wide::IntArray,
    ),
    (
        "sulfur_cube_archetype",
        &["attribute_modifiers", "[]", "amount"],
        Wide::Double,
    ),
    (
        "worldgen/block_state_provider",
        &["noise", "base_amplitude"],
        Wide::Double,
    ),
    (
        "worldgen/block_state_provider",
        &["slow_noise", "base_amplitude"],
        Wide::Double,
    ),
    ("worldgen/block_state_provider", &["seed"], Wide::Long),
];

/// `(registry, earlier, later)`: codec field order for every pair of keys that
/// share a `HashMap` bucket somewhere in that registry's payloads. Keys in one
/// bucket are written in insertion order, which the JSON (sorted by datagen)
/// does not carry.
const COLLIDING_FIELD_ORDER: &[(&str, &str, &str)] = &[
    (
        "block_transformer",
        "block_state_provider",
        "item_damage_per_use",
    ),
    ("block_transformer", "waterlogged", "half"),
    ("cat_sound_variant", "ambient_sound", "hurt_sound"),
    ("cat_sound_variant", "hiss_sound", "eat_sound"),
    ("chat_type", "translation_key", "style"),
    ("chicken_sound_variant", "ambient_sound", "hurt_sound"),
    ("cow_sound_variant", "ambient_sound", "hurt_sound"),
    ("damage_type", "scaling", "effects"),
    ("dialog", "title", "type"),
    (
        "dimension_type",
        "ambient_light",
        "monster_spawn_block_light_limit",
    ),
    ("dimension_type", "coordinate_scale", "logical_height"),
    ("dimension_type", "has_fixed_time", "infiniburn"),
    ("dimension_type", "min_y", "monster_spawn_light_level"),
    ("dimension_type", "skybox", "timelines"),
    ("dimension_type", "tick_delay", "offset"),
    ("enchantment", "anvil_cost", "exclusive_set"),
    ("enchantment", "description", "anvil_cost"),
    ("enchantment", "description", "exclusive_set"),
    ("enchantment", "description", "weight"),
    ("enchantment", "factor", "type"),
    ("enchantment", "id", "attribute"),
    ("enchantment", "immune_blocks", "block_interaction"),
    ("enchantment", "max_level", "min_cost"),
    (
        "enchantment",
        "minecraft:movement",
        "minecraft:movement_affected_by",
    ),
    ("enchantment", "numerator", "denominator"),
    ("enchantment", "predicate", "trigger_game_event"),
    ("enchantment", "primary_items", "effects"),
    ("enchantment", "primary_items", "slots"),
    ("enchantment", "radius", "type"),
    ("enchantment", "slots", "effects"),
    ("enchantment", "supported_items", "max_level"),
    ("enchantment", "supported_items", "min_cost"),
    ("enchantment", "to_apply", "min_duration"),
    ("enchantment", "vertical_position", "vertical_velocity"),
    ("enchantment", "weight", "anvil_cost"),
    ("enchantment", "weight", "exclusive_set"),
    ("instrument", "range", "description"),
    ("pig_sound_variant", "ambient_sound", "hurt_sound"),
    ("sulfur_cube_archetype", "amount", "attribute_to_source"),
    ("sulfur_cube_archetype", "attribute", "id"),
    ("sulfur_cube_archetype", "hit_sound", "push_sound_cooldown"),
    (
        "sulfur_cube_archetype",
        "hit_sound",
        "push_sound_impulse_threshold",
    ),
    ("sulfur_cube_archetype", "items", "sound_settings"),
    ("sulfur_cube_archetype", "power", "causes_fire"),
    (
        "sulfur_cube_archetype",
        "push_sound_impulse_threshold",
        "push_sound_cooldown",
    ),
    ("test_instance", "structure", "required"),
    ("wolf_sound_variant", "ambient_sound", "hurt_sound"),
    ("wolf_sound_variant", "growl_sound", "pant_sound"),
    ("worldgen/biome", "downfall", "attributes"),
    ("worldgen/biome", "mood", "additions"),
    ("worldgen/biome", "tick_delay", "offset"),
    ("worldgen/block_state_provider", "low_states", "type"),
    ("worldgen/block_state_provider", "seed", "default_state"),
    ("worldgen/block_state_provider", "slow_noise", "states"),
    ("worldgen/block_state_provider", "slow_scale", "noise"),
];

/// Initial table size of a default `java.util.HashMap`.
const JAVA_HASH_MAP_MIN_CAPACITY: usize = 16;

#[derive(Debug, Error)]
pub enum RegistryNetworkError {
    #[error("entry JSON is invalid: {0}")]
    Json(#[from] JsonError),
    #[error("{path}: JSON null has no network encoding")]
    Null { path: String },
    #[error("{path}: {value} does not fit its NBT type")]
    OutOfRange { path: String, value: String },
    #[error("{path}: the JSON value does not fit its {wide} field")]
    WideMismatch { path: String, wide: &'static str },
    #[error("{path}: a list of mixed NBT types is not modelled")]
    MixedList { path: String },
    #[error("encoding the payload failed: {0}")]
    Nbt(#[from] mc_nbt::NbtError),
}

/// Encode the data-pack JSON of one entry of `registry` (for example
/// `worldgen/biome`) as the Network NBT value vanilla sends for it in
/// `RegistryData`.
pub fn encode_entry(registry: &str, json: &str) -> Result<Vec<u8>, RegistryNetworkError> {
    let mut value = json::parse(json)?;
    apply_network_codec(registry, &mut value);
    let mut path = Vec::new();
    let tag = to_tag(registry, &value, &mut path)?;
    let mut payload = Vec::new();
    mc_nbt::write_network_value(&mut payload, &tag)?;
    Ok(payload)
}

/// Drop what the registry's network codec does not send.
fn apply_network_codec(registry: &str, value: &mut Json<'_>) {
    let Json::Object(fields) = value else {
        return;
    };
    fields.retain(|(key, _)| {
        !NETWORK_OMITTED_FIELDS
            .iter()
            .any(|(owner, field)| *owner == registry && field == key)
    });
    for (_, field, kept_when_emptied) in ATTRIBUTE_MAPS
        .iter()
        .filter(|(owner, _, _)| *owner == registry)
    {
        let Some(Json::Object(map)) = fields
            .iter_mut()
            .find_map(|(key, value)| (key == field).then_some(value))
        else {
            continue;
        };
        map.retain(|(key, _)| !SERVER_ONLY_ATTRIBUTES.contains(&key.as_str()));
        if map.is_empty() && !kept_when_emptied {
            fields.retain(|(key, _)| key != field);
        }
    }
}

fn to_tag<'a>(
    registry: &str,
    value: &'a Json<'_>,
    path: &mut Vec<&'a str>,
) -> Result<Tag, RegistryNetworkError> {
    let wide = WIDE_FIELDS
        .iter()
        .find(|(owner, pattern, _)| *owner == registry && glob_matches(pattern, path))
        .map(|(_, _, wide)| *wide);
    match (value, wide) {
        (Json::Number(number), Some(Wide::Double)) => number
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .map(Tag::Double)
            .ok_or_else(|| out_of_range(path, number)),
        (Json::Number(number), Some(Wide::Long)) => json::is_integer(number)
            .then(|| number.parse::<i64>().ok())
            .flatten()
            .map(Tag::Long)
            .ok_or_else(|| out_of_range(path, number)),
        (Json::Array(items), Some(Wide::IntArray)) => items
            .iter()
            .map(|item| match item {
                Json::Number(number) => json::is_integer(number)
                    .then(|| number.parse::<i32>().ok())
                    .flatten()
                    .ok_or_else(|| out_of_range(path, number)),
                _ => Err(wide_mismatch(path, Wide::IntArray)),
            })
            .collect::<Result<_, _>>()
            .map(Tag::IntArray),
        (_, Some(wide)) => Err(wide_mismatch(path, wide)),
        (Json::Null, None) => Err(RegistryNetworkError::Null {
            path: path.join("/"),
        }),
        (Json::Bool(flag), None) => Ok(Tag::Byte(i8::from(*flag))),
        (Json::String(text), None) => Ok(Tag::String(text.clone())),
        (Json::Number(number), None) => number_tag(number, path),
        (Json::Array(items), None) => {
            path.push("[]");
            let elements = items
                .iter()
                .map(|item| to_tag(registry, item, path))
                .collect::<Result<Vec<_>, _>>();
            path.pop();
            list_tag(elements?, path)
        }
        (Json::Object(fields), None) => {
            let mut entries = Vec::with_capacity(fields.len());
            for (key, field) in fields {
                path.push(key);
                let tag = to_tag(registry, field, path);
                path.pop();
                entries.push((key.clone(), tag?));
            }
            Ok(Tag::Compound(java_hash_map_order(registry, entries)))
        }
    }
}

fn wide_mismatch(path: &[&str], wide: Wide) -> RegistryNetworkError {
    RegistryNetworkError::WideMismatch {
        path: path.join("/"),
        wide: match wide {
            Wide::Double => "Double",
            Wide::Long => "Long",
            Wide::IntArray => "IntArray",
        },
    }
}

/// A JSON integer is an `Int` and a JSON fraction a `Float`.
fn number_tag(number: &str, path: &[&str]) -> Result<Tag, RegistryNetworkError> {
    let tag = if json::is_integer(number) {
        number.parse::<i32>().ok().map(Tag::Int)
    } else {
        number
            .parse::<f32>()
            .ok()
            .filter(|value| value.is_finite())
            .map(Tag::Float)
    };
    tag.ok_or_else(|| out_of_range(path, number))
}

fn list_tag(elements: Vec<Tag>, path: &[&str]) -> Result<Tag, RegistryNetworkError> {
    let Some(first) = elements.first() else {
        return Ok(Tag::List(ListTag::empty()));
    };
    let element_type = first.type_id();
    if elements.iter().any(|tag| tag.type_id() != element_type) {
        return Err(RegistryNetworkError::MixedList {
            path: path.join("/"),
        });
    }
    debug_assert_ne!(element_type, tag_type::END);
    Ok(Tag::List(ListTag {
        element_type,
        elements,
    }))
}

fn out_of_range(path: &[&str], number: &str) -> RegistryNetworkError {
    RegistryNetworkError::OutOfRange {
        path: path.join("/"),
        value: number.to_owned(),
    }
}

/// Whether `pattern` matches the whole `path`; `**` matches any run of
/// segments and `*` exactly one.
fn glob_matches(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => (0..=path.len()).any(|skip| glob_matches(rest, &path[skip..])),
        Some((segment, rest)) => path.split_first().is_some_and(|(first, tail)| {
            (*segment == "*" || segment == first) && glob_matches(rest, tail)
        }),
    }
}

/// `entries` in the order a `java.util.HashMap` of default capacity iterates
/// them: by bucket, then by insertion within a bucket.
fn java_hash_map_order(registry: &str, entries: Vec<(String, Tag)>) -> Vec<(String, Tag)> {
    let capacity = java_hash_map_capacity(entries.len());
    let mut by_bucket: Vec<(usize, (String, Tag))> = entries
        .into_iter()
        .map(|entry| (java_hash_bucket(&entry.0, capacity), entry))
        .collect();
    by_bucket.sort_by_key(|(bucket, _)| *bucket);
    let mut ordered: Vec<(String, Tag)> = Vec::with_capacity(by_bucket.len());
    let mut bucket_start = 0;
    let mut current_bucket = None;
    for (bucket, entry) in by_bucket {
        if current_bucket != Some(bucket) {
            current_bucket = Some(bucket);
            bucket_start = ordered.len();
        }
        let position = ordered[bucket_start..]
            .iter()
            .position(|(key, _)| inserted_before(registry, &entry.0, key))
            .map_or(ordered.len(), |offset| bucket_start + offset);
        ordered.insert(position, entry);
    }
    ordered
}

fn inserted_before(registry: &str, key: &str, other: &str) -> bool {
    COLLIDING_FIELD_ORDER
        .iter()
        .any(|(owner, earlier, later)| *owner == registry && *earlier == key && *later == other)
}

/// The table size a default `HashMap` has grown to after `len` insertions
/// (load factor 0.75).
pub(crate) fn java_hash_map_capacity(len: usize) -> usize {
    let mut capacity = JAVA_HASH_MAP_MIN_CAPACITY;
    while len * 4 > capacity * 3 {
        capacity *= 2;
    }
    capacity
}

/// `HashMap`'s bucket for a `String` key: `String.hashCode` over UTF-16 code
/// units, spread by `h ^ (h >>> 16)`.
pub(crate) fn java_hash_bucket(key: &str, capacity: usize) -> usize {
    let hash = key.encode_utf16().fold(0u32, |hash, unit| {
        hash.wrapping_mul(31).wrapping_add(u32::from(unit))
    });
    (hash ^ (hash >> 16)) as usize & (capacity - 1)
}

#[cfg(test)]
#[path = "registry_network_tests.rs"]
mod tests;
