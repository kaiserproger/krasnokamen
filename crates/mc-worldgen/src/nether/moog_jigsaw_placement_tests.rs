//! Hand-authored metadata-shaped fixtures, not extracted mod templates.
//! Each marker template has the selected MVS template's bounding dimensions,
//! one rigid start element and no connectors, matching the local input census.

use super::*;
use crate::vanilla_features::{BiomeTagIndex, BlockTagIndex, CacheTags};
use crate::village::VillageDecor;
use crate::village::closure::load_jigsaw_closure;
use crate::village::plan_source::VillagePlanSource;
use mc_data::village_data::VillageDataLoader;
use mc_nbt::{ListTag, Tag, tag_type};
use mc_world::native::{NativeDomainRecord, NativeStore};
use mc_world::{BlockPos, Chunk, ChunkGenerator, ChunkPos};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;

struct Fixture {
    name: &'static str,
    pool: &'static str,
    template: &'static str,
    size: [i32; 3],
    salt: i32,
    spacing: i32,
    separation: i32,
    terrain_range: Option<i32>,
}

const FIXTURES: [Fixture; 3] = [
    Fixture {
        name: "nether_well",
        pool: "well/wells/nether_well/start_pool",
        template: "1_21_9/well/wells/nether_well",
        size: [5, 7, 5],
        salt: 298952413,
        spacing: 48,
        separation: 30,
        terrain_range: None,
    },
    Fixture {
        name: "crimson_enchanting_table",
        pool: "other_decoration/crimson_enchanting_table/start_pool",
        template: "1_21_0/other_decoration/crimson_enchanting_table",
        size: [13, 3, 14],
        salt: 836845261,
        spacing: 77,
        separation: 58,
        terrain_range: Some(3),
    },
    Fixture {
        name: "nether_devil",
        pool: "other_decoration/nether_devil/start_pool",
        template: "1_21_0/other_decoration/nether_devil",
        size: [4, 14, 5],
        salt: 513238798,
        spacing: 64,
        separation: 16,
        terrain_range: Some(1),
    },
];

fn id(value: &str) -> mc_data::Identifier {
    mc_data::Identifier::parse(value).unwrap()
}

struct FixtureTags;
impl BlockTagIndex for FixtureTags {
    fn block_in_tag(&self, _: &mc_data::Identifier, _: &mc_data::Identifier) -> bool {
        false
    }
}
impl BiomeTagIndex for FixtureTags {
    fn biome_in_tag(&self, tag: &mc_data::Identifier, biome: &mc_data::Identifier) -> bool {
        match tag.as_str() {
            "c:is_nether" => super::super::landscape::BIOMES.contains(&biome.as_str()),
            "mvs:primary_wood_type/crimson" => biome.as_str() == "minecraft:crimson_forest",
            other => panic!("unexpected fixture biome tag {other}"),
        }
    }
}

fn write_json(root: &Path, path: &str, value: &Value) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}

fn structure(fixture: &Fixture) -> Value {
    let mut value = json!({
        "type": "moogs_structures:moogs_structures_generic_nether_jigsaw_structure",
        "start_pool": format!("mvs:{}", fixture.pool), "size": 1,
        "biomes": if fixture.name == "crimson_enchanting_table" { "#mvs:primary_wood_type/crimson" } else { "#c:is_nether" },
        "cannot_spawn_in_liquid": true, "land_search_direction": "HIGHEST_LAND",
        "project_start_to_heightmap": "WORLD_SURFACE_WG", "step": "surface_structures",
        "terrain_adaptation": "beard_thin", "start_height": {"absolute": 0}, "spawn_overrides": {}
    });
    if fixture.name == "nether_well" {
        value["liquid_settings"] = json!("ignore_waterlogging");
    }
    if let Some(range) = fixture.terrain_range {
        value["terrain_height_radius_check"] = json!(2);
        value["allowed_terrain_height_range"] = json!(range);
    }
    value
}

fn write_fixture(root: &Path, fixture: &Fixture) {
    write_json(
        root,
        "data/minecraft/worldgen/template_pool/empty.json",
        &json!({"name":"minecraft:empty", "fallback":"minecraft:empty", "elements":[]}),
    );
    write_json(
        root,
        "data/minecraft/worldgen/processor_list/empty.json",
        &json!({"processors":[]}),
    );
    write_json(
        root,
        &format!("data/mvs/worldgen/structure/{}.json", fixture.name),
        &structure(fixture),
    );
    write_json(
        root,
        &format!("data/mvs/worldgen/structure_set/{}.json", fixture.name),
        &json!({
            "structures": [{"structure": format!("mvs:{}", fixture.name), "weight": 1}],
            "placement": {"type": "moogs_structures:advanced_random_spread", "salt": fixture.salt,
                "spacing": fixture.spacing, "separation": fixture.separation, "min_distance_from_world_origin": 150}
        }),
    );
    write_json(
        root,
        &format!("data/mvs/worldgen/template_pool/{}.json", fixture.pool),
        &json!({
            "name": format!("mvs:{}", fixture.pool), "fallback": "minecraft:empty",
            "elements": [{"weight": 1, "element": {"element_type": "moogs_structures:versioned_single_pool_element",
                "location": format!("mvs:{}", fixture.template),
                "locations": {"1.21.9-26.3": format!("mvs:{}", fixture.template)},
                "processors": "minecraft:empty", "projection": "rigid"}}]
        }),
    );
    let list = |kind, elements| {
        Tag::List(ListTag {
            element_type: kind,
            elements,
        })
    };
    let template = Tag::Compound(vec![
        (
            "size".into(),
            list(tag_type::INT, fixture.size.map(Tag::Int).to_vec()),
        ),
        (
            "palette".into(),
            list(
                tag_type::COMPOUND,
                vec![Tag::Compound(vec![(
                    "Name".into(),
                    Tag::String("minecraft:diamond_block".into()),
                )])],
            ),
        ),
        (
            "blocks".into(),
            list(
                tag_type::COMPOUND,
                vec![Tag::Compound(vec![
                    ("pos".into(), list(tag_type::INT, vec![Tag::Int(0); 3])),
                    ("state".into(), Tag::Int(0)),
                ])],
            ),
        ),
        ("entities".into(), list(tag_type::COMPOUND, Vec::new())),
    ]);
    let path = root.join(format!("data/mvs/structure/{}.nbt", fixture.template));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut bytes = Vec::new();
    mc_nbt::write_named(&mut bytes, "", &template).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn build_source(root: &Path, fixture: &Fixture, generator: &NetherGenerator) -> VillagePlanSource {
    let loader = VillageDataLoader::new(root.join("data/minecraft/worldgen"));
    let set_id = id(&format!("mvs:{}", fixture.name));
    let set = loader.load_structure_set(&set_id).unwrap();
    let mut closure = load_jigsaw_closure(root, &set_id, set, &generator.registry).unwrap();
    closure.dimension_bounds = (0, 128);
    VillagePlanSource::new(
        Arc::new(closure),
        Arc::new(VillageDecor::empty()),
        generator.seed,
        generator.registry.clone(),
        Arc::new(FixtureTags),
        Arc::new(FixtureTags),
    )
    .unwrap()
}

fn fixture_cache_tags(blocks: Arc<mc_world::BlockRegistry>) -> Arc<CacheTags> {
    let data = Arc::new(mc_data::krasnokamen_required_data());
    let biomes = &data.registry("worldgen/biome").unwrap().entries;
    let tags = ["c:is_nether", "mvs:primary_wood_type/crimson"].map(|tag| {
        let tag = id(tag);
        let entries = biomes
            .iter()
            .enumerate()
            .filter(|(_, biome)| FixtureTags.biome_in_tag(&tag, biome))
            .map(|(raw, _)| i32::try_from(raw).unwrap())
            .collect();
        (tag, entries)
    });
    let tags = mc_data::tags::TagsData::from_registries(std::collections::BTreeMap::from([(
        id("minecraft:worldgen/biome"),
        tags.into(),
    )]));
    Arc::new(CacheTags::new(blocks, data, Arc::new(tags)))
}

fn assert_placement_and_reload(root: &Path, fixture: &Fixture, marker: &str) {
    let directory = tempfile::tempdir().unwrap();
    let blocks =
        Arc::new(
            mc_world::BlockRegistry::from_report(
                &mc_data::blocks::krasnokamen_required_blocks_report(),
            )
            .unwrap(),
        );
    let tags = fixture_cache_tags(blocks.clone());
    let mut generator = NetherGenerator::new(1234, blocks);
    generator.load_jigsaw_worldgen(root, tags).unwrap();
    let source = build_source(root, fixture, &generator);
    let query = NetherMoogQuery {
        generator: &generator,
    };
    let biome = |x, y, z| query.biome_at(x, y, z);
    let placement = source.closure().placement;
    let plan = (-8..=8)
        .flat_map(|z| (-8..=8).map(move |x| (x, z)))
        .find_map(|(x, z)| {
            let (cx, cz) = placement.potential_chunk(
                generator.seed,
                x * placement.spacing,
                z * placement.spacing,
            );
            source.plan_for_start_chunk(cx, cz, &query, &biome)
        })
        .expect("fixed-seed Nether fixture must find an admitted start");
    assert_eq!(
        plan.assembly().structure,
        id(&format!("mvs:{}", fixture.name))
    );
    assert_eq!(plan.assembly().pieces.len(), 1);
    let piece = &plan.assembly().pieces[0];
    let center = BlockPos {
        x: piece.bounds_min.x + (piece.bounds_max.x - piece.bounds_min.x + 1) / 2,
        y: 0,
        z: piece.bounds_min.z + (piece.bounds_max.z - piece.bounds_min.z + 1) / 2,
    };
    let ground = highest_land(&generator, center.x, center.z, 0, 128).unwrap();
    assert_eq!(
        piece.bounds_min.y, ground,
        "Moog moves the base onto occupied HIGHEST_LAND"
    );
    assert!(piece.bounds_min.y >= 0 && piece.bounds_max.y < 128);
    let rebuilt = build_source(root, fixture, &generator);
    assert_eq!(
        rebuilt.plan_for_start_chunk(plan.start_chunk().0, plan.start_chunk().1, &query, &biome),
        Some(plan.clone())
    );
    let site = generator
        .structure_sites_in_region(plan.start_chunk(), plan.start_chunk())
        .into_iter()
        .find(|site| site.structure == plan.assembly().structure)
        .expect("the attached Nether source must expose the same exact-ID start");
    assert_eq!(site.position, plan.assembly().origin);
    let mut chunks = Vec::new();
    let marker_state = generator.registry.block(&id(marker)).unwrap().default;
    let mut marker_count = 0;
    for z in piece.bounds_min.z.div_euclid(16)..=piece.bounds_max.z.div_euclid(16) {
        for x in piece.bounds_min.x.div_euclid(16)..=piece.bounds_max.x.div_euclid(16) {
            let chunk = generator.generate(ChunkPos { x, z });
            for lx in 0..16 {
                if !(piece.bounds_min.x..=piece.bounds_max.x).contains(&(x * 16 + lx as i32)) {
                    continue;
                }
                for lz in 0..16 {
                    if !(piece.bounds_min.z..=piece.bounds_max.z).contains(&(z * 16 + lz as i32)) {
                        continue;
                    }
                    for y in piece.bounds_min.y..=piece.bounds_max.y {
                        marker_count +=
                            usize::from(chunk.get_block(lx, y, lz) == Some(marker_state));
                    }
                }
            }
            let saved = mc_world::structure_starts::read(&chunk.extras, chunk.pos, false).unwrap();
            assert!(
                saved
                    .iter()
                    .any(|s| s.structure == plan.assembly().structure)
            );
            chunks.push(chunk);
        }
    }
    assert!(
        marker_count > 0,
        "the attached source must actually write {marker}"
    );
    persist_and_reload(directory.path(), &generator, &chunks);
    eprintln!(
        "{} seed=1234 start={:?} occupied_land_y={ground} bounds={:?}..{:?}; exact-ID native reload passed",
        fixture.name,
        plan.start_chunk(),
        piece.bounds_min,
        piece.bounds_max
    );
}

fn persist_and_reload(root: &Path, generator: &NetherGenerator, chunks: &[Chunk]) {
    let path = root.join("native.redb");
    let store = NativeStore::create_with_cache(&path, [9; 32], 1024 * 1024).unwrap();
    let dimensions = json!({
        "0": {"name":"minecraft:overworld", "min_y":-64,"height":384,"generator":"fixture","seed":1234,"revision":1},
        "1": {"name":"minecraft:the_nether", "min_y":0,"height":256,"generator":"fixture","seed":1234,"revision":1}
    });
    store
        .initialize_dimensions(&NativeDomainRecord {
            version: 1,
            captured_tick: 0,
            payload: serde_json::to_vec(&dimensions).unwrap(),
        })
        .unwrap();
    let items = mc_data::items::krasnokamen_required_items();
    store
        .fill_generated(1, chunks, &generator.registry, &items)
        .unwrap();
    drop(store);
    let reopened = NativeStore::open_with_cache(&path, [9; 32], 1024 * 1024).unwrap();
    for chunk in chunks {
        let restored = reopened
            .read_chunk(
                1,
                chunk.pos,
                &generator.registry,
                &items,
                &mc_data::loot::chest_26_1_2::ChestLootCatalog::new(),
                generator,
            )
            .unwrap()
            .unwrap();
        for x in 0..16 {
            for z in 0..16 {
                for y in NETHER_MIN_Y..=NETHER_CEILING_Y {
                    assert_eq!(
                        restored.get_block(x, y, z),
                        chunk.get_block(x, y, z),
                        "generated Nether blocks must survive native close/reopen at ({x}, {y}, {z})"
                    );
                }
            }
        }
        assert_eq!(
            restored.extras, chunk.extras,
            "all start metadata must survive native close/reopen unchanged"
        );
        assert_eq!(
            restored.block_entities, chunk.block_entities,
            "authored block entity identity and coordinates must survive native reload"
        );
        assert_eq!(
            mc_world::structure_starts::read(&restored.extras, restored.pos, false).unwrap(),
            mc_world::structure_starts::read(&chunk.extras, chunk.pos, false).unwrap()
        );
    }
}

fn check_handwritten_fixture(fixture: &Fixture) {
    let directory = tempfile::tempdir().unwrap();
    write_fixture(directory.path(), fixture);
    assert_placement_and_reload(directory.path(), fixture, "minecraft:diamond_block");
}

#[test]
fn nether_well_fixed_seed_placement_and_native_reload() {
    check_handwritten_fixture(&FIXTURES[0]);
}
#[test]
fn crimson_enchanting_table_fixed_seed_placement_and_native_reload() {
    check_handwritten_fixture(&FIXTURES[1]);
}
#[test]
fn nether_devil_fixed_seed_placement_and_native_reload() {
    check_handwritten_fixture(&FIXTURES[2]);
}

#[test]
#[ignore = "requires the locally licensed MVS Nether data; no mod data is tracked"]
fn licensed_mvs_nether_templates_place_and_reload_from_native_store() {
    let root = std::env::var_os("MVS_NETHER_INPUT_ROOT")
        .map(std::path::PathBuf::from)
        .expect("set MVS_NETHER_INPUT_ROOT to the selected local-only MVS data root");
    for (fixture, marker) in FIXTURES.iter().zip([
        "minecraft:nether_bricks",
        "minecraft:enchanting_table",
        "minecraft:nether_brick_slab",
    ]) {
        assert_placement_and_reload(&root, fixture, marker);
    }
}

#[test]
fn missing_land_refuses_the_start_instead_of_using_sea_level() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = &FIXTURES[0];
    write_fixture(directory.path(), fixture);
    let blocks =
        Arc::new(
            mc_world::BlockRegistry::from_report(
                &mc_data::blocks::krasnokamen_required_blocks_report(),
            )
            .unwrap(),
        );
    let generator = NetherGenerator::new(1234, blocks);
    let source = build_source(directory.path(), fixture, &generator);
    let query = NetherMoogQuery {
        generator: &generator,
    };
    let placement = source.closure().placement;
    let missing = (1..=8)
        .flat_map(|z| (1..=8).map(move |x| (x, z)))
        .map(|(x, z)| placement.potential_chunk(1234, x * placement.spacing, z * placement.spacing))
        .find(|&(x, z)| highest_land(&generator, x * 16, z * 16, 0, 128).is_none())
        .expect("fixture must include a genuine no-land candidate");
    assert!(
        source
            .plan_for_start_chunk(missing.0, missing.1, &query, &|x, y, z| query
                .biome_at(x, y, z))
            .is_none()
    );
}

#[test]
fn unsupported_nether_layout_rules_keep_exact_refusal_messages() {
    let directory = tempfile::tempdir().unwrap();
    let loader = VillageDataLoader::new(directory.path().join("data/minecraft/worldgen"));
    let fixture = &FIXTURES[0];
    let entry = id("mvs:nether_well");
    for (field, value, expected) in [
        (
            "burying_type",
            json!("LOWEST_CORNER"),
            "structure mvs:nether_well has invalid value for burying_type: unsupported Moog burial: \"LOWEST_CORNER\"",
        ),
        (
            "burying_type",
            json!("LOWEST_SIDE"),
            "structure mvs:nether_well has invalid value for burying_type: unsupported Moog burial: \"LOWEST_SIDE\"",
        ),
        (
            "pools_that_ignore_boundaries",
            json!(["mvs:ignored"]),
            "structure mvs:nether_well has invalid value for pools_that_ignore_boundaries: unsupported Moog rule: [\"mvs:ignored\"]",
        ),
    ] {
        let mut value_with_rule = structure(fixture);
        value_with_rule[field] = value;
        write_json(
            directory.path(),
            "data/mvs/worldgen/structure/nether_well.json",
            &value_with_rule,
        );
        assert_eq!(
            loader.load_structure(&entry).unwrap_err().to_string(),
            expected
        );
    }
}

#[test]
fn unsupported_spawner_features_keep_exact_refusal_messages() {
    let directory = tempfile::tempdir().unwrap();
    let loader = VillageDataLoader::new(directory.path().join("data/minecraft/worldgen"));
    let entry = id("mvs:unsupported");
    let base = json!({"processor_type":"moogs_structures:spawner_randomizing_processor",
        "weighted_entities":[{"entity":"minecraft:blaze","weight":1.0}]});
    let mut mob_nbt = base.clone();
    mob_nbt["weighted_entities"][0]["nbt"] = json!({"NoAI":true});
    let mut replacements = base.clone();
    replacements["replacement_blocks"] = json!([]);
    let capped = json!({"processor_type":"minecraft:capped","delegate":base,"limit":1});
    for (processor, expected) in [
        (
            mob_nbt,
            "structure processor mvs:unsupported has invalid value for weighted_entities: unsupported entity fields",
        ),
        (
            replacements,
            "structure processor mvs:unsupported has invalid value for processor_type: unsupported Moog spawner field replacement_blocks",
        ),
        (
            capped,
            "structure processor mvs:unsupported has invalid value for delegate: capped Moog spawner processors are unsupported",
        ),
    ] {
        write_json(
            directory.path(),
            "data/mvs/worldgen/processor_list/unsupported.json",
            &json!({"processors":[processor]}),
        );
        assert_eq!(
            loader.load_processor_list(&entry).unwrap_err().to_string(),
            expected
        );
    }
}
