//! Actual offline export of WAL-only chunks: ancestry refusal and selected Nether replay.
use crate::export_tests::read_exported_chunk;
use crate::test_support::{fixture_store, read_manifest};
use crate::{export_anvil, hashes, import_anvil};
use mc_core::dimensions::Dimension;
use mc_core::journal::{JournalEntry, JournalWriter};
use mc_core::{Core, CoreConfig};
use mc_nbt::{ListTag, Tag, tag_type};
use mc_world::{BlockStateId, ChunkPos};
use serde_json::json;
use std::path::{Path, PathBuf};

fn config(world: &Path) -> CoreConfig {
    let mut config = CoreConfig::new(world.to_owned(), 1234);
    config.vanilla_data_dir = Some(PathBuf::from(
        std::env::var_os("KRASNOKAMEN_CONTENT_CACHE").expect("local gameplay cache"),
    ));
    config
}

fn write_json(root: &Path, path: &str, value: serde_json::Value) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}

// Handwritten marker data, not extracted mod bytes. The production loader,
// dimension router, occupied-land solver and native/Anvil codecs consume it.
fn nether_pack(world: &Path) -> PathBuf {
    let root = world.join("datapacks/offline-nether");
    write_json(
        &root,
        "pack.mcmeta",
        json!({"pack":{"description":"offline regression"}}),
    );
    write_json(
        &root,
        "data/test/worldgen/structure/offline_nether.json",
        json!({
            "type":"moogs_structures:moogs_structures_generic_nether_jigsaw_structure",
            "start_pool":"test:offline_nether", "size":1,
            "biomes":["minecraft:nether_wastes","minecraft:crimson_forest","minecraft:warped_forest","minecraft:soul_sand_valley","minecraft:basalt_deltas"],
            "cannot_spawn_in_liquid":true, "land_search_direction":"HIGHEST_LAND",
            "project_start_to_heightmap":"WORLD_SURFACE_WG", "step":"surface_structures",
            "terrain_adaptation":"beard_thin", "start_height":{"absolute":0}, "spawn_overrides":{}
        }),
    );
    write_json(
        &root,
        "data/test/worldgen/structure_set/offline_nether.json",
        json!({
            "structures":[{"structure":"test:offline_nether","weight":1}],
            "placement":{"type":"minecraft:random_spread","salt":7341,"spacing":8,"separation":2}
        }),
    );
    write_json(
        &root,
        "data/test/worldgen/template_pool/offline_nether.json",
        json!({
            "name":"test:offline_nether", "fallback":"minecraft:empty",
            "elements":[{"weight":1,"element":{"element_type":"minecraft:single_pool_element",
            "location":"test:offline_nether","processors":"minecraft:empty","projection":"rigid"}}]
        }),
    );
    let list = |kind, elements| {
        Tag::List(ListTag {
            element_type: kind,
            elements,
        })
    };
    let template = Tag::Compound(vec![
        ("size".into(), list(tag_type::INT, vec![Tag::Int(1); 3])),
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
    let path = root.join("data/test/structure/offline_nether.nbt");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut bytes = Vec::new();
    mc_nbt::write_named(&mut bytes, "", &template).unwrap();
    std::fs::write(path, bytes).unwrap();
    root
}

fn append_nether_edit(config: &CoreConfig, pos: ChunkPos, state: u32) {
    let store = fixture_store(config).unwrap();
    let tick = read_manifest(&store).unwrap().tick + 1;
    drop(store);
    let mut writer = JournalWriter::open(
        &config.world_dir,
        1,
        mc_core::journal::test_wal_identity(&config.world_dir),
    )
    .unwrap();
    writer
        .append(
            tick,
            tick,
            &[JournalEntry::Dimension {
                dimension: Dimension::Nether,
                entries: vec![JournalEntry::Block {
                    pos: [pos.x * 16, 200, pos.z * 16],
                    state,
                }],
            }],
        )
        .unwrap();
    writer.flush().unwrap();
}

#[test]
#[ignore = "requires KRASNOKAMEN_CONTENT_CACHE; actual offline consumer"]
fn offline_export_refuses_unknown_wal_only_worldgen_ancestry() {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("native");
    let config = config(&source);
    let registry =
        mc_core::registry::CoreRegistry::load(config.vanilla_data_dir.as_deref()).unwrap();
    // A real pre-identity checkpoint, not a malformed JSON string or mock.
    append_nether_edit(
        &config,
        ChunkPos { x: 8192, z: 8192 },
        registry.states.stone,
    );
    let before = hashes(&source).unwrap();
    let destination = temporary.path().join("export");
    assert!(
        export_anvil(
            &config,
            &source,
            &destination,
            &temporary.path().join("receipt.json")
        )
        .is_err()
    );
    assert!(
        !destination.exists(),
        "unproven landscape must never publish"
    );
    assert_eq!(hashes(&source).unwrap(), before);
}

#[test]
#[ignore = "requires KRASNOKAMEN_CONTENT_CACHE; actual offline consumer"]
fn offline_export_refuses_changed_wal_only_selected_inputs() {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("native");
    let pack = nether_pack(&source);
    let config = config(&source);
    let core = Core::open(config.clone()).unwrap();
    let registry = core.registry().clone();
    core.shutdown();
    append_nether_edit(
        &config,
        ChunkPos { x: 8192, z: 8192 },
        registry.states.stone,
    );
    // Change real selected bytes, retaining a syntactically valid structure.
    let path = pack.join("data/test/worldgen/structure_set/offline_nether.json");
    let mut set: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    set["placement"]["salt"] = json!(7342);
    std::fs::write(path, serde_json::to_vec(&set).unwrap()).unwrap();
    let before = hashes(&source).unwrap();
    let destination = temporary.path().join("export");
    assert!(
        export_anvil(
            &config,
            &source,
            &destination,
            &temporary.path().join("receipt.json")
        )
        .is_err()
    );
    assert!(!destination.exists());
    assert_eq!(hashes(&source).unwrap(), before);
}

#[test]
#[ignore = "requires KRASNOKAMEN_CONTENT_CACHE; actual offline consumer"]
fn offline_export_same_inputs_preserve_selected_nether_geometry_and_reopen() {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("native");
    nether_pack(&source);
    let config = config(&source);
    let core = Core::open(config.clone()).unwrap();
    let registry = core.registry().clone();
    core.shutdown();
    let recovered = mc_core::native_checkpoint::recover_offline(&config).unwrap();
    let scratch = temporary.path().join("expected-inputs");
    std::fs::create_dir(&scratch).unwrap();
    let generators = recovered.selected_generators(&scratch).unwrap();
    let generator = &generators[&Dimension::Nether];
    let target = mc_data::Identifier::parse("test:offline_nether").unwrap();
    let mut found = None;
    'search: for ring in 0..=8 {
        for candidate in generator.structure_candidates_in_ring(
            std::slice::from_ref(&target),
            (8192, 8192),
            ring,
        ) {
            if let Some(site) = generator
                .structure_sites_at(std::slice::from_ref(&target), candidate)
                .into_iter()
                .next()
            {
                found = Some(site);
                break 'search;
            }
        }
    }
    let site = found.expect("selected Nether source must admit the handwritten marker");
    let pos = ChunkPos {
        x: site.start_chunk.0,
        z: site.start_chunk.1,
    };
    assert!(
        recovered
            .reader
            .read_chunk(
                1,
                pos,
                &registry.blocks,
                &registry.items,
                &registry.chest_loot
            )
            .unwrap()
            .is_none(),
        "exercise genuine WAL-only fallback, not physical-baseline readback"
    );
    let mut expected = generator.generate(pos);
    let diamond = registry
        .blocks
        .block(&mc_data::Identifier::parse("minecraft:diamond_block").unwrap())
        .unwrap()
        .default;
    // Like vanilla's jigsaw GenerationStub, the start site sits one ground-level
    // delta above the rigid 1×1×1 piece, whose only block is the marker.
    assert_eq!(
        expected.get_block(
            site.position.x.rem_euclid(16) as u8,
            site.position.y - 1,
            site.position.z.rem_euclid(16) as u8
        ),
        Some(diamond)
    );
    assert!(
        mc_world::structure_starts::read(&expected.extras, pos, false)
            .unwrap()
            .iter()
            .any(|start| start.structure == target)
    );
    assert!(
        expected
            .set_block(0, 200, 0, BlockStateId(registry.states.stone))
            .is_some()
    );
    drop(recovered);
    drop(generators);
    append_nether_edit(&config, pos, registry.states.stone);
    let before = hashes(&source).unwrap();
    let destination = temporary.path().join("export");
    export_anvil(
        &config,
        &source,
        &destination,
        &temporary.path().join("export.json"),
    )
    .unwrap();
    assert_eq!(hashes(&source).unwrap(), before);
    let actual = read_exported_chunk(&destination, Dimension::Nether, pos, &registry);
    assert_geometry(&expected, &actual);
    let imported = temporary.path().join("reimport");
    import_anvil(
        &config,
        &destination,
        &imported,
        &temporary.path().join("import.json"),
    )
    .unwrap();
    let mut reopened_config = config.clone();
    reopened_config.world_dir = imported;
    let reopened = mc_core::native_checkpoint::recover_offline(&reopened_config).unwrap();
    let actual = reopened
        .reader
        .read_chunk(
            1,
            pos,
            &registry.blocks,
            &registry.items,
            &registry.chest_loot,
        )
        .unwrap()
        .unwrap();
    assert_geometry(&expected, &actual);
}

fn assert_geometry(expected: &mc_world::Chunk, actual: &mc_world::Chunk) {
    assert_eq!(actual.geometry(), Dimension::Nether.geometry());
    assert_eq!(actual.pos, expected.pos);
    for y in 0..256 {
        for z in 0..16 {
            for x in 0..16 {
                assert_eq!(
                    actual.get_block(x, y, z),
                    expected.get_block(x, y, z),
                    "Nether cell {x},{y},{z}"
                );
            }
        }
    }
    assert_eq!(
        mc_world::structure_starts::read(&actual.extras, actual.pos, false).unwrap(),
        mc_world::structure_starts::read(&expected.extras, expected.pos, false).unwrap()
    );
    assert_eq!(actual.block_entities, expected.block_entities);
}
