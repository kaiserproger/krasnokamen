use std::collections::BTreeMap;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use mc_nbt::{Tag, tag_type};

use super::json::{self, Json};
use super::{
    ATTRIBUTE_MAPS, RegistryNetworkError, encode_entry, glob_matches, java_hash_bucket,
    java_hash_map_capacity,
};

fn decode(payload: &[u8]) -> Tag {
    let mut remaining = payload;
    let tag = mc_nbt::read_network_value(&mut remaining).expect("valid network NBT");
    assert!(remaining.is_empty(), "trailing bytes after the payload");
    tag
}

fn compound(tag: &Tag) -> &[(String, Tag)] {
    match tag {
        Tag::Compound(entries) => entries,
        other => panic!("expected a compound, got {other:?}"),
    }
}

fn keys(tag: &Tag) -> Vec<&str> {
    compound(tag).iter().map(|(key, _)| key.as_str()).collect()
}

fn field<'a>(tag: &'a Tag, name: &str) -> &'a Tag {
    compound(tag)
        .iter()
        .find_map(|(key, value)| (key == name).then_some(value))
        .unwrap_or_else(|| panic!("missing field {name}"))
}

fn encode(registry: &str, json: &str) -> Tag {
    decode(&encode_entry(registry, json).expect("encodes"))
}

#[test]
fn bucket_follows_java_string_hash_code() {
    // `String.hashCode` reference values: "scaling" = 1910897543,
    // "effects" = -1833928446, "é" = 233, "😀" (a surrogate pair) = 1772899.
    assert_eq!(java_hash_bucket("scaling", 16), 2);
    assert_eq!(java_hash_bucket("effects", 16), 2);
    assert_eq!(java_hash_bucket("exhaustion", 16), 8);
    assert_eq!(java_hash_bucket("message_id", 16), 10);
    assert_eq!(java_hash_bucket("é", 1 << 20), 233);
    assert_eq!(java_hash_bucket("😀", 1 << 20), 724_344);
}

#[test]
fn capacity_grows_past_three_quarters_full() {
    assert_eq!(java_hash_map_capacity(0), 16);
    assert_eq!(java_hash_map_capacity(12), 16);
    assert_eq!(java_hash_map_capacity(13), 32);
    assert_eq!(java_hash_map_capacity(24), 32);
    assert_eq!(java_hash_map_capacity(25), 64);
}

#[test]
fn keys_follow_hash_buckets_then_codec_order_within_a_bucket() {
    // Vanilla's 26.3 `damage_type/campfire` payload: `scaling` and `effects`
    // share bucket 2 and the codec inserts `scaling` first, although datagen
    // sorts `effects` first in the JSON.
    let tag = encode(
        "damage_type",
        r#"{"effects":"burning","exhaustion":0.1,"message_id":"inFire",
            "scaling":"when_caused_by_living_non_player"}"#,
    );
    assert_eq!(
        keys(&tag),
        ["scaling", "effects", "exhaustion", "message_id"]
    );
    assert_eq!(field(&tag, "exhaustion"), &Tag::Float(0.1));
}

#[test]
fn network_codec_omits_spawn_conditions_and_biome_generation() {
    let variant = encode(
        "cat_variant",
        r#"{"asset_id":"minecraft:entity/cat/tabby","spawn_conditions":[{"priority":0}]}"#,
    );
    assert_eq!(keys(&variant), ["asset_id"]);

    let biome = encode(
        "worldgen/biome",
        r##"{"carvers":[],"features":[],"downfall":0.5,"has_precipitation":false,
            "temperature":0.5,"effects":{"water_color":"#3f76e4"},
            "attributes":{"minecraft:gameplay/natural_mob_spawns":{"modifier":"overlay"}}}"##,
    );
    // The emptied map stays: biome presence is decided before the filter.
    assert_eq!(field(&biome, "attributes"), &Tag::Compound(Vec::new()));
    assert!(!keys(&biome).contains(&"carvers"));
    assert!(!keys(&biome).contains(&"features"));
}

#[test]
fn server_only_attributes_are_filtered_and_empty_tracks_omitted() {
    let dimension = encode(
        "dimension_type",
        r#"{"attributes":{"minecraft:gameplay/bed_rule":{"can_sleep":"always"},
            "minecraft:visual/cloud_height":192.33},"coordinate_scale":1.0}"#,
    );
    assert_eq!(
        keys(field(&dimension, "attributes")),
        ["minecraft:visual/cloud_height"]
    );
    let timeline = encode(
        "timeline",
        r#"{"clock":"minecraft:overworld","tracks":{"minecraft:gameplay/villager_activity":{}}}"#,
    );
    assert_eq!(keys(&timeline), ["clock"]);
}

#[test]
fn json_kinds_map_to_vanilla_nbt_types() {
    let dimension = encode(
        "dimension_type",
        r#"{"coordinate_scale":8.0,"min_y":-64,"has_skylight":true,"timelines":[],
            "attributes":{"minecraft:visual/cloud_height":192.33}}"#,
    );
    assert_eq!(field(&dimension, "coordinate_scale"), &Tag::Double(8.0));
    assert_eq!(field(&dimension, "min_y"), &Tag::Int(-64));
    assert_eq!(field(&dimension, "has_skylight"), &Tag::Byte(1));
    assert!(matches!(
        field(&dimension, "timelines"),
        Tag::List(list) if list.element_type == tag_type::END && list.elements.is_empty()
    ));
    let Tag::Float(height) = field(
        field(&dimension, "attributes"),
        "minecraft:visual/cloud_height",
    ) else {
        panic!("cloud height is a float");
    };
    assert_eq!(height.to_bits(), 192.33f32.to_bits());

    let provider = encode(
        "worldgen/block_state_provider",
        r#"{"seed":2345,"noise":{"base_amplitude":1.0,"firstOctave":0}}"#,
    );
    assert_eq!(field(&provider, "seed"), &Tag::Long(2345));
    assert_eq!(
        field(field(&provider, "noise"), "base_amplitude"),
        &Tag::Double(1.0)
    );
    assert_eq!(
        field(field(&provider, "noise"), "firstOctave"),
        &Tag::Int(0)
    );
}

#[test]
fn block_offsets_are_int_arrays_and_movement_ranges_doubles() {
    let enchantment = encode(
        "enchantment",
        r#"{"effects":{"minecraft:tick":[{"requirements":{"condition":"minecraft:entity_properties",
            "predicate":{"minecraft:movement":{"horizontal_speed":{"min":1.0E-5}}}},
            "effect":{"type":"minecraft:replace_disk","offset":[0,-1,0],
            "predicate":{"type":"minecraft:all_of","predicates":[{"offset":[0,1,0]}]}}}]}}"#,
    );
    let Tag::List(ticks) = field(field(&enchantment, "effects"), "minecraft:tick") else {
        panic!("tick effects are a list");
    };
    let tick = &ticks.elements[0];
    let range = field(
        field(
            field(field(tick, "requirements"), "predicate"),
            "minecraft:movement",
        ),
        "horizontal_speed",
    );
    assert_eq!(field(range, "min"), &Tag::Double(1.0e-5));
    let effect = field(tick, "effect");
    assert_eq!(field(effect, "offset"), &Tag::IntArray(vec![0, -1, 0]));
    let Tag::List(predicates) = field(field(effect, "predicate"), "predicates") else {
        panic!("predicates are a list");
    };
    assert_eq!(
        field(&predicates.elements[0], "offset"),
        &Tag::IntArray(vec![0, 1, 0])
    );
}

#[test]
fn unmodelled_values_fail_closed() {
    assert!(matches!(
        encode_entry("dialog", r#"{"body":["text",{"type":"plain"}]}"#),
        Err(RegistryNetworkError::MixedList { .. })
    ));
    assert!(matches!(
        encode_entry("dialog", r#"{"title":null}"#),
        Err(RegistryNetworkError::Null { .. })
    ));
    for json in [
        r#"{"min_y":3000000000}"#,
        r#"{"min_y":18446744073709551615}"#,
    ] {
        assert!(matches!(
            encode_entry("dimension_type", json),
            Err(RegistryNetworkError::OutOfRange { .. })
        ));
    }
    assert!(matches!(
        encode_entry("dimension_type", r#"{"coordinate_scale":"1"}"#),
        Err(RegistryNetworkError::WideMismatch { .. })
    ));
}

#[test]
fn glob_matches_whole_paths() {
    let pattern = ["**", "predicates", "[]", "offset"];
    assert!(glob_matches(&pattern, &["predicates", "[]", "offset"]));
    assert!(glob_matches(
        &pattern,
        &["[]", "rules", "[]", "if_true", "predicates", "[]", "offset"]
    ));
    assert!(!glob_matches(
        &pattern,
        &["predicates", "[]", "offset", "x"]
    ));
    assert!(glob_matches(&["a", "*", "c"], &["a", "b", "c"]));
    assert!(!glob_matches(&["a", "*", "c"], &["a", "c"]));
}

#[test]
fn doubles_round_from_their_json_text() {
    // Vanilla's 26.3 `sulfur_cube_archetype/fast_sliding` modifier. A
    // best-effort float parser lands one ulp away (0xBFEE_6666_6600_0001).
    let archetype = encode(
        "sulfur_cube_archetype",
        r#"{"attribute_modifiers":[{"amount":-0.9499999992549419}]}"#,
    );
    let Tag::List(modifiers) = field(&archetype, "attribute_modifiers") else {
        panic!("modifiers are a list");
    };
    let Tag::Double(amount) = field(&modifiers.elements[0], "amount") else {
        panic!("amount is a double");
    };
    assert_eq!(amount.to_bits(), 0xBFEE_6666_6600_0000);
}

#[test]
fn json_reader_keeps_number_text_order_and_escapes() {
    assert_eq!(
        json::parse(r#" {"b":[1.50,-0,2e-3],"a":"\u00e9\ud83d\ude00\n\"","c":null} "#),
        Ok(Json::Object(vec![
            (
                "b".into(),
                Json::Array(vec![
                    Json::Number("1.50"),
                    Json::Number("-0"),
                    Json::Number("2e-3"),
                ])
            ),
            ("a".into(), Json::String("é😀\n\"".into())),
            ("c".into(), Json::Null),
        ]))
    );
    assert!(json::is_integer("-12"));
    assert!(!json::is_integer("1.0"));
    assert!(!json::is_integer("1E5"));
    for invalid in [
        r#"{"a":1,"a":2}"#,
        r#"{"a":01}"#,
        r#"{"a":1.}"#,
        r#""\ud83d""#,
        "[1] 2",
        "[1,]",
        "\"tab\there\"",
    ] {
        assert!(json::parse(invalid).is_err(), "{invalid} must be rejected");
    }
}

// --- maintainer check against a vanilla capture -----------------------------

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/mc-data sits two levels below the workspace")
        .to_path_buf()
}

/// The release's own jar from a Mojang server bundle (or the jar itself).
fn release_jar(path: &Path) -> zip::ZipArchive<Cursor<Vec<u8>>> {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut outer = zip::ZipArchive::new(Cursor::new(bytes)).expect("server jar is a zip");
    let inner = match outer.by_name("META-INF/versions.list") {
        Ok(mut list) => {
            let mut text = String::new();
            list.read_to_string(&mut text).expect("versions.list");
            let line = text
                .lines()
                .next()
                .expect("versions.list names the release jar");
            let jar = line
                .split('\t')
                .nth(2)
                .expect("versions.list: hash, id, path");
            Some(format!("META-INF/versions/{jar}"))
        }
        Err(_) => None,
    };
    let Some(inner) = inner else {
        return outer;
    };
    let mut bytes = Vec::new();
    outer
        .by_name(&inner)
        .expect("bundle holds the release jar")
        .read_to_end(&mut bytes)
        .expect("read the release jar");
    zip::ZipArchive::new(Cursor::new(bytes)).expect("release jar is a zip")
}

/// `registry -> entry -> bytes` under `<capture>/reports/registry_network_nbt/minecraft`.
fn captured_payloads(capture: &Path) -> BTreeMap<String, BTreeMap<String, Vec<u8>>> {
    let root = capture
        .join(crate::NETWORK_REGISTRY_PAYLOAD_DIR)
        .join("minecraft");
    let mut files = Vec::new();
    let mut pending = vec![root.clone()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|ext| ext == "nbt") {
                files.push(path);
            }
        }
    }
    let mut payloads: BTreeMap<String, BTreeMap<String, Vec<u8>>> = BTreeMap::new();
    for path in files {
        let relative = path
            .strip_prefix(&root)
            .expect("under the payload root")
            .to_string_lossy()
            .replace('\\', "/");
        let (registry, entry) = relative
            .split_once("/minecraft/")
            .expect("<registry>/minecraft/<entry>.nbt");
        let entry = entry.strip_suffix(".nbt").expect(".nbt").to_owned();
        let bytes = std::fs::read(&path).expect("payload");
        payloads
            .entry(registry.to_owned())
            .or_default()
            .insert(entry, bytes);
    }
    payloads
}

/// Every data-pack entry of `registries` in the release jar: `registry -> entry -> JSON`.
fn data_pack_entries(
    jar: &mut zip::ZipArchive<Cursor<Vec<u8>>>,
    registries: &[&String],
) -> BTreeMap<String, BTreeMap<String, String>> {
    let names: Vec<String> = jar.file_names().map(str::to_owned).collect();
    let mut entries: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for name in names {
        let Some(rest) = name
            .strip_prefix("data/minecraft/")
            .and_then(|rest| rest.strip_suffix(".json"))
        else {
            continue;
        };
        let Some((registry, entry)) = registries.iter().find_map(|registry| {
            rest.strip_prefix(registry.as_str())
                .and_then(|entry| entry.strip_prefix('/'))
                .map(|entry| (registry.as_str(), entry))
        }) else {
            continue;
        };
        let mut json = String::new();
        jar.by_name(&name)
            .expect("listed entry")
            .read_to_string(&mut json)
            .expect("UTF-8 JSON");
        entries
            .entry(registry.to_owned())
            .or_default()
            .insert(entry.to_owned(), json);
    }
    entries
}

/// `None` when `ours` is the same vanilla encoding as the capture: identical
/// bytes, or identical except for the order of keys sharing a bucket inside an
/// environment-attribute map, which vanilla writes in a per-run order.
fn vanilla_encoding_difference(registry: &str, ours: &[u8], captured: &[u8]) -> Option<String> {
    if ours == captured {
        return None;
    }
    fn walk(registry: &str, ours: &Tag, theirs: &Tag, path: &mut Vec<String>) -> Option<String> {
        let at = |path: &[String]| format!("/{}", path.join("/"));
        match (ours, theirs) {
            (Tag::Compound(a), Tag::Compound(b)) => {
                let keys_a: Vec<&str> = a.iter().map(|(key, _)| key.as_str()).collect();
                let keys_b: Vec<&str> = b.iter().map(|(key, _)| key.as_str()).collect();
                if keys_a != keys_b {
                    let attribute_map = path.len() == 1
                        && ATTRIBUTE_MAPS
                            .iter()
                            .any(|(owner, field, _)| *owner == registry && *field == path[0]);
                    let capacity = java_hash_map_capacity(keys_a.len());
                    let buckets = |keys: &[&str]| -> Vec<usize> {
                        keys.iter()
                            .map(|key| java_hash_bucket(key, capacity))
                            .collect()
                    };
                    let mut sorted_a = keys_a.clone();
                    let mut sorted_b = keys_b.clone();
                    sorted_a.sort_unstable();
                    sorted_b.sort_unstable();
                    if !attribute_map
                        || sorted_a != sorted_b
                        || buckets(&keys_a) != buckets(&keys_b)
                    {
                        return Some(format!("{}: keys {keys_a:?} vs {keys_b:?}", at(path)));
                    }
                }
                for (key, value) in a {
                    let other = b
                        .iter()
                        .find_map(|(other, v)| (other == key).then_some(v))
                        .expect("same key set");
                    path.push(key.clone());
                    let difference = walk(registry, value, other, path);
                    path.pop();
                    if difference.is_some() {
                        return difference;
                    }
                }
                None
            }
            (Tag::List(a), Tag::List(b)) => {
                if a.element_type != b.element_type || a.elements.len() != b.elements.len() {
                    return Some(format!("{}: list shape differs", at(path)));
                }
                path.push("[]".to_owned());
                let difference = a
                    .elements
                    .iter()
                    .zip(&b.elements)
                    .find_map(|(x, y)| walk(registry, x, y, path));
                path.pop();
                difference
            }
            (Tag::Float(a), Tag::Float(b)) if a.to_bits() == b.to_bits() => None,
            (Tag::Double(a), Tag::Double(b)) if a.to_bits() == b.to_bits() => None,
            (Tag::Float(_) | Tag::Double(_), _) => {
                Some(format!("{}: {ours:?} vs {theirs:?}", at(path)))
            }
            (a, b) if a == b => None,
            (a, b) => Some(format!("{}: {a:?} vs {b:?}", at(path))),
        }
    }
    walk(registry, &decode(ours), &decode(captured), &mut Vec::new())
}

/// The maintainer proof behind ADR 0022: every captured payload of a release
/// is reproduced from that release's data pack. Run once per release, e.g.
///
/// ```text
/// SOLARIS_REGISTRY_JAR=<26.3 server.jar> SOLARIS_REGISTRY_CAPTURE=data/vanilla \
///   cargo test -p mc-data --lib registry_network -- --ignored --nocapture
/// ```
#[test]
#[ignore = "maintainer check: needs a Mojang server jar (SOLARIS_REGISTRY_JAR) and its \
            RegistryData capture (SOLARIS_REGISTRY_CAPTURE, default data/vanilla)"]
fn native_payloads_match_the_vanilla_capture() {
    let jar_path = std::env::var_os("SOLARIS_REGISTRY_JAR")
        .map(PathBuf::from)
        .expect("set SOLARIS_REGISTRY_JAR to the release's server jar (bundle or inner jar)");
    let capture = std::env::var_os("SOLARIS_REGISTRY_CAPTURE")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace().join("data/vanilla"));
    let captured = captured_payloads(&capture);
    assert!(
        !captured.is_empty(),
        "no payloads under {}",
        capture.display()
    );
    let registries: Vec<&String> = captured.keys().collect();
    let mut jar = release_jar(&jar_path);
    let data_pack = data_pack_entries(&mut jar, &registries);

    let mut failures = Vec::new();
    let (mut exact, mut permuted) = (0usize, Vec::new());
    for (registry, payloads) in &captured {
        let empty = BTreeMap::new();
        let definitions = data_pack.get(registry).unwrap_or(&empty);
        for entry in definitions
            .keys()
            .filter(|entry| !payloads.contains_key(*entry))
        {
            failures.push(format!(
                "{registry}/{entry}: data-pack entry without a payload"
            ));
        }
        for (entry, captured_bytes) in payloads {
            let Some(json) = definitions.get(entry) else {
                failures.push(format!(
                    "{registry}/{entry}: payload without a data-pack entry"
                ));
                continue;
            };
            let ours = match encode_entry(registry, json) {
                Ok(ours) => ours,
                Err(error) => {
                    failures.push(format!("{registry}/{entry}: {error}"));
                    continue;
                }
            };
            if ours == *captured_bytes {
                exact += 1;
            } else if let Some(difference) =
                vanilla_encoding_difference(registry, &ours, captured_bytes)
            {
                failures.push(format!("{registry}/{entry}: {difference}"));
            } else {
                permuted.push(format!("{registry}/{entry}"));
            }
        }
    }
    let total: usize = captured.values().map(BTreeMap::len).sum();
    eprintln!(
        "registry_network: {} registries, {total} payloads: {exact} byte-identical, {} differing \
         only in vanilla's per-run attribute-map order {permuted:?}, {} failures",
        captured.len(),
        permuted.len(),
        failures.len(),
    );
    assert!(failures.is_empty(), "{failures:#?}");
}
