use super::*;
use crate::datapacks::{PackSet, Selection};
use mc_worldgen::vanilla_features::BiomeTagIndex;

fn write(root: &Path, path: &str, bytes: &[u8]) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}
fn pack(root: &Path, name: &str, resources: &[(&str, &[u8])]) {
    let root = root.join(name);
    write(
        &root,
        "pack.mcmeta",
        br#"{"pack":{"description":"overlay test"}}"#,
    );
    for (path, bytes) in resources {
        write(&root.join("data"), path, bytes);
    }
}
fn id(value: &str) -> mc_data::Identifier {
    mc_data::Identifier::parse(value).unwrap()
}
fn synthetic_cache(root: &Path) {
    write(root, "reports/registries.json", b"{}");
    write(root, "data/minecraft/worldgen/biome/plains.json", b"{}");
    write(root, "data/minecraft/worldgen/biome/forest.json", b"{}");
    write(
        root,
        "data/minecraft/worldgen/structure_set/collision.json",
        br#"{"layer":"vanilla"}"#,
    );
    write(
        root,
        "data/minecraft/tags/worldgen/biome/base.json",
        br##"{"values":["plains"]}"##,
    );
    // Minimal vanilla registry/tag fixture for the authored convention defaults.
    for (_, entries) in mc_data::biome_convention_tags::default_biome_tags() {
        for (value, _) in *entries {
            if let Some(tag) = value.strip_prefix("#minecraft:") {
                write(
                    root,
                    &format!("data/minecraft/tags/worldgen/biome/{tag}.json"),
                    br#"{"values":[]}"#,
                );
            } else if let Some(biome) = value.strip_prefix("minecraft:") {
                write(
                    root,
                    &format!("data/minecraft/worldgen/biome/{biome}.json"),
                    b"{}",
                );
            }
        }
    }
}
fn nbt() -> Vec<u8> {
    let mut out = Vec::new();
    mc_nbt::write_named(
        &mut out,
        "",
        &mc_nbt::Tag::Compound(vec![(
            "binary".into(),
            mc_nbt::Tag::ByteArray(vec![-1, 0, -128]),
        )]),
    )
    .unwrap();
    out
}

#[test]
fn binary_directory_and_zip_resources_survive_ordered_overlay_without_mutating_cache() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("cache");
    synthetic_cache(&cache);
    let packs = dir.path().join("datapacks");
    let binary = nbt();
    pack(
        &packs,
        "low",
        &[
            ("mod/structure/nested/piece.nbt", &binary),
            (
                "minecraft/worldgen/structure_set/collision.json",
                br#"{"layer":"low"}"#,
            ),
        ],
    );
    let mut zip = zip::ZipWriter::new(std::fs::File::create(packs.join("high.zip")).unwrap());
    for (path, bytes) in [
        (
            "pack.mcmeta",
            br#"{"pack":{"description":"zip"}}"#.as_slice(),
        ),
        ("data/mod/structure/zipped.nbt", binary.as_slice()),
        (
            "data/minecraft/worldgen/structure_set/collision.json",
            br#"{"layer":"high"}"#.as_slice(),
        ),
    ] {
        zip.start_file(path, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
    let selection = Selection {
        enabled: vec!["file/low".into(), "vanilla".into(), "file/high.zip".into()],
        disabled: Vec::new(),
    };
    let mut loaded = PackSet::load_known(&packs, selection).unwrap();
    let inputs = loaded.prepare_worldgen(dir.path(), Some(&cache)).unwrap();
    let root = inputs.root().unwrap().to_owned();
    assert_eq!(
        std::fs::read(root.join("data/mod/structure/nested/piece.nbt")).unwrap(),
        binary
    );
    assert_eq!(
        std::fs::read(root.join("data/mod/structure/zipped.nbt")).unwrap(),
        binary
    );
    assert_eq!(
        std::fs::read(root.join("data/minecraft/worldgen/structure_set/collision.json")).unwrap(),
        br#"{"layer":"high"}"#
    );
    assert_eq!(
        std::fs::read(cache.join("data/minecraft/worldgen/structure_set/collision.json")).unwrap(),
        br#"{"layer":"vanilla"}"#
    );
    drop(inputs);
    assert!(!root.exists());
    let mut below = PackSet::load_known(
        &packs,
        Selection {
            enabled: vec!["file/low".into(), "vanilla".into()],
            disabled: vec!["file/high.zip".into()],
        },
    )
    .unwrap();
    let inputs = below.prepare_worldgen(dir.path(), Some(&cache)).unwrap();
    assert_eq!(
        std::fs::read(
            inputs
                .root()
                .unwrap()
                .join("data/minecraft/worldgen/structure_set/collision.json")
        )
        .unwrap(),
        br#"{"layer":"vanilla"}"#
    );
}

#[test]
fn filters_hide_lower_namespaces_and_biome_tags_merge_replace_and_expand() {
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("cache");
    synthetic_cache(&cache);
    let packs = dir.path().join("datapacks");
    pack(
        &packs,
        "low",
        &[
            (
                "mod/tags/worldgen/biome/allowed.json",
                br##"{"values":["#base"]}"##,
            ),
            ("mod/worldgen/structure_set/remove.json", b"{}"),
        ],
    );
    pack(
        &packs,
        "high",
        &[(
            "mod/tags/worldgen/biome/allowed.json",
            br##"{"values":["minecraft:forest",{"id":"mod:missing","required":false}]}"##,
        )],
    );
    write(&packs.join("high"), "pack.mcmeta", br#"{"pack":{"description":"filter"},"filter":{"block":[{"namespace":"mod","path":"worldgen/structure_set/.*"},{"namespace":"minecraft","path":"worldgen/structure_set/.*"}]}}"#);
    let mut loaded = PackSet::load_known(
        &packs,
        Selection {
            enabled: vec!["vanilla".into(), "file/low".into(), "file/high".into()],
            disabled: vec![],
        },
    )
    .unwrap();
    let inputs = loaded.prepare_worldgen(dir.path(), Some(&cache)).unwrap();
    assert!(
        !inputs
            .root()
            .unwrap()
            .join("data/mod/worldgen/structure_set/remove.json")
            .exists()
    );
    assert!(
        !inputs
            .root()
            .unwrap()
            .join("data/minecraft/worldgen/structure_set/collision.json")
            .exists()
    );
    assert!(
        inputs
            .biome_tags
            .biome_in_tag(&id("mod:allowed"), &id("minecraft:plains"))
    );
    assert!(
        inputs
            .biome_tags
            .biome_in_tag(&id("mod:allowed"), &id("minecraft:forest"))
    );
    drop(inputs);
    write(
        &packs.join("high/data"),
        "mod/tags/worldgen/biome/allowed.json",
        br#"{"replace":true,"values":["minecraft:forest"]}"#,
    );
    let mut loaded = PackSet::load_known(
        &packs,
        Selection {
            enabled: vec!["vanilla".into(), "file/low".into(), "file/high".into()],
            disabled: vec![],
        },
    )
    .unwrap();
    let inputs = loaded.prepare_worldgen(dir.path(), Some(&cache)).unwrap();
    assert!(
        !inputs
            .biome_tags
            .biome_in_tag(&id("mod:allowed"), &id("minecraft:plains"))
    );
    assert!(
        inputs
            .biome_tags
            .biome_in_tag(&id("mod:allowed"), &id("minecraft:forest"))
    );
}

#[test]
fn malformed_resources_paths_limits_and_cyclic_biome_tags_fail_closed() {
    for path in [
        "mod/../escape.nbt",
        "Mod/structure/a.nbt",
        "mod/structure/./a.nbt",
        "mod/structure/\\a.nbt",
        "mod//a.json",
    ] {
        assert!(validate_path(path).is_err(), "{path}");
    }
    for (path, data) in [
        ("mod/structure/a.nbt", b"not NBT".as_slice()),
        ("mod/worldgen/structure_set/a.json", b"{".as_slice()),
        ("mod/worldgen/biome/a.json", b"{}".as_slice()),
        (
            "mod/worldgen/structure/unknown.json",
            br#"{"type":"example:jigsaw"}"#.as_slice(),
        ),
        (
            "mod/tags/worldgen/biome/a.json",
            br#"{"values":[4]}"#.as_slice(),
        ),
    ] {
        assert!(validate_resource(path, data).is_err(), "{path}");
    }
    let mut bytes = nbt();
    bytes.push(1);
    assert!(validate_resource("mod/structure/a.nbt", &bytes).is_err());
    let mut budget = 2;
    assert!(crate::datapacks::limited_bytes(&b"abc"[..], &mut budget).is_err());
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("cache");
    synthetic_cache(&cache);
    let packs = dir.path().join("datapacks");
    pack(
        &packs,
        "cycle",
        &[(
            "mod/tags/worldgen/biome/a.json",
            br##"{"values":["#mod:a"]}"##,
        )],
    );
    let mut loaded = PackSet::load(&packs, Selection::default()).unwrap();
    assert!(
        loaded
            .prepare_worldgen(dir.path(), Some(&cache))
            .err()
            .unwrap()
            .contains("Cyclic")
    );
}

#[test]
fn custom_moog_piece_count_domains_fail_closed_by_resource_name() {
    for domain in [
        "msl_pieces_spawn_counts",
        "msl_pieces_spawn_counts_additions",
    ] {
        let path = format!("mod/{domain}/must_not_ignore.json");
        assert!(is_resource(&path), "{path}");
        let dir = tempfile::tempdir().unwrap();
        pack(dir.path(), "custom", &[(path.as_str(), b"{}")]);
        let error = PackSet::load(dir.path(), Selection::default())
            .err()
            .expect("unsupported piece-count rules must refuse the pack");
        assert!(error.contains(&path), "{error}");
    }
}

#[test]
fn hot_worldgen_enable_disable_and_modified_reload_refuse_without_publication() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("datapacks");
    pack(&root, "wg", &[("mod/worldgen/structure_set/a.json", b"{}")]);
    let base = crate::registry::CoreRegistry::load(None).unwrap();
    let current = PackSet::load(&root, Selection::default()).unwrap();
    let disabled = Selection {
        enabled: vec!["vanilla".into()],
        disabled: vec!["file/wg".into()],
    };
    let without = PackSet::load_known(&root, disabled.clone()).unwrap();
    for (from, selected, job) in [
        (
            &current,
            disabled,
            crate::datapacks::Job::Activation {
                action: crate::datapacks::Action::Disable,
                id: "file/wg".into(),
            },
        ),
        (
            &without,
            current.selection.clone(),
            crate::datapacks::Job::Activation {
                action: crate::datapacks::Action::Enable,
                id: "file/wg".into(),
            },
        ),
    ] {
        let error = crate::datapacks::work(&root, selected, &job, Some(&base), from)
            .err()
            .unwrap()
            .into_text();
        assert!(error.contains("immutable generators"), "{error}");
    }
    write(
        &root.join("wg/data"),
        "mod/worldgen/structure_set/a.json",
        br#"{"changed":true}"#,
    );
    let error = crate::datapacks::work(
        &root,
        current.selection.clone(),
        &crate::datapacks::Job::Reload,
        Some(&base),
        &current,
    )
    .err()
    .unwrap()
    .into_text();
    assert!(error.contains("immutable generators"), "{error}");
    assert_eq!(current.selection.enabled, ["vanilla", "file/wg"]);
}

#[test]
fn saved_disabled_selection_and_wal_selection_are_resolved_before_pack_loading() {
    let dir = tempfile::tempdir().unwrap();
    let config = crate::CoreConfig::new(dir.path().to_owned(), 42);
    pack(
        &dir.path().join("datapacks"),
        "bad",
        &[("mod/worldgen/biome/unsupported.json", b"{}")],
    );
    let disabled = Selection {
        enabled: vec!["vanilla".into()],
        disabled: vec!["file/bad".into()],
    };
    crate::native_checkpoint::tests::write_manifest(
        &config,
        &crate::journal::CheckpointManifest {
            datapack_selection: Some(disabled.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
    let mut base = crate::registry::CoreRegistry::load(None).unwrap();
    crate::plugin_startup::apply(
        &mut base,
        &mc_plugin_host::DeploymentContribution::default(),
        None,
        &[],
    )
    .unwrap();
    let base = Arc::new(base);
    let (state, manifest, _) =
        crate::core::recover_tick_state(&config, &base, &store, false, None, None).unwrap();
    assert_eq!(state.datapacks.selection(), &disabled);
    drop(state);
    let mut writer = crate::journal::JournalWriter::open(dir.path(), 1).unwrap();
    writer
        .append(
            manifest.tick + 1,
            manifest.tick + 1,
            &[crate::journal::JournalEntry::DatapackSelection {
                value: Selection {
                    enabled: vec!["vanilla".into(), "file/bad".into()],
                    disabled: vec![],
                },
            }],
        )
        .unwrap();
    drop(writer);
    let error = crate::core::recover_tick_state(&config, &base, &store, false, None, None)
        .err()
        .unwrap();
    assert!(error.to_string().contains("unsupported.json"), "{error}");
}

#[test]
fn startup_identity_changes_are_diagnostic_and_invalidate_the_terrain_epoch() {
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("cache");
    synthetic_cache(&cache);
    let packs = dir.path().join("datapacks");
    pack(
        &packs,
        "wg",
        &[("mod/worldgen/structure_set/a.json", b"{}")],
    );
    let mut loaded = PackSet::load(&packs, Selection::default()).unwrap();
    let inputs = loaded.prepare_worldgen(dir.path(), Some(&cache)).unwrap();
    let config = crate::CoreConfig::new(dir.path().to_owned(), 42);
    let mut globals = crate::native_checkpoint::generation_records(&config).unwrap();
    globals.insert(
        "level_metadata".into(),
        crate::native_checkpoint::tag_record(0, &mc_nbt::Tag::Compound(vec![])).unwrap(),
    );
    assert!(inputs.stage_identity(&mut globals).unwrap().changed);
    assert!(!inputs.stage_identity(&mut globals).unwrap().changed);
    let previous_epoch = "0123456789abcdef0123456789abcdef";
    crate::terrain::stage_epoch(&mut globals, previous_epoch).unwrap();
    drop(inputs);
    write(
        &packs.join("wg/data"),
        "mod/worldgen/structure_set/a.json",
        br#"{"changed":true}"#,
    );
    let mut loaded = PackSet::load(&packs, loaded.selection).unwrap();
    let inputs = loaded.prepare_worldgen(dir.path(), Some(&cache)).unwrap();
    assert!(inputs.stage_identity(&mut globals).unwrap().changed);
    assert_ne!(
        crate::terrain::read_data_field(&globals, "KrasnokamenTerrainEpoch")
            .unwrap()
            .as_deref(),
        Some(previous_epoch)
    );
    assert!(!inputs.stage_identity(&mut globals).unwrap().changed);
}

#[test]
fn staging_recovers_interrupted_construction_but_never_deletes_unowned_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join(STAGE);
    std::fs::create_dir(&root).unwrap();
    let stage = Stage::create(dir.path(), &BTreeMap::new()).unwrap();
    write(&root, "data/partial.json", b"partial");
    std::mem::forget(stage); // Model a crash after the ownership marker.
    let stage = Stage::create(dir.path(), &BTreeMap::new()).unwrap();
    assert!(!root.join("data/partial.json").exists());
    drop(stage);
    assert!(
        dir.path().join(OWNER).is_file(),
        "cleanup retains durable ownership proof"
    );
    write(&root, "reports/partial-cleanup.json", b"partial cleanup");
    Stage::cleanup(dir.path()).unwrap();
    assert!(!root.exists(), "interrupted cleanup is recoverable");
    write(&root, "user-file", b"must survive");
    assert!(Stage::create(dir.path(), &BTreeMap::new()).is_err());
    assert_eq!(
        std::fs::read(root.join("user-file")).unwrap(),
        b"must survive"
    );
}

#[test]
#[ignore = "requires the local vanilla gameplay cache; run with --include-ignored"]
fn fresh_restart_and_wal_generated_chunks_share_selected_worldgen() {
    let dir = tempfile::tempdir().unwrap();
    let cache = std::env::var_os("KRASNOKAMEN_CONTENT_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla"));
    let mut config = crate::CoreConfig::new(dir.path().to_owned(), 81);
    config.vanilla_data_dir = Some(cache);
    let mut registry =
        crate::registry::CoreRegistry::load(config.vanilla_data_dir.as_deref()).unwrap();
    crate::plugin_startup::apply(
        &mut registry,
        &mc_plugin_host::DeploymentContribution::default(),
        None,
        &[],
    )
    .unwrap();
    let registry = Arc::new(registry);
    let baseline = crate::core::overworld_terrain(&config, &registry).unwrap();
    let site = baseline
        .village_sites_in_region((-68, -68), (101, 101))
        .into_iter()
        .next()
        .expect("baseline has villages");
    drop(baseline);
    let root = dir.path().join("datapacks");
    let tags: Vec<_> = ["plains", "desert", "savanna", "snowy", "taiga"]
        .into_iter()
        .map(|name| format!("minecraft/tags/worldgen/biome/has_structure/village_{name}.json"))
        .collect();
    let resources: Vec<_> = tags
        .iter()
        .map(|path| (path.as_str(), br#"{"replace":true,"values":[]}"#.as_slice()))
        .collect();
    pack(&root, "no-villages", &resources);
    let core = crate::Core::open(config.clone()).unwrap();
    core.shutdown();
    assert!(!dir.path().join(STAGE).exists());
    // A clean reopen must build the same selected generator and retain its identity.
    let core = crate::Core::open(config.clone()).unwrap();
    core.shutdown();
    let manifest = crate::native_checkpoint::tests::read_manifest(&config).unwrap();
    let x = site.start_chunk.0 * 16;
    let z = site.start_chunk.1 * 16;
    let selected = manifest.datapack_selection.clone().unwrap();
    let mut writer = crate::journal::JournalWriter::open(dir.path(), 1).unwrap();
    writer
        .append(
            manifest.tick + 1,
            manifest.tick + 1,
            &[
                crate::journal::JournalEntry::DatapackSelection {
                    value: selected.clone(),
                },
                crate::journal::JournalEntry::Block {
                    pos: [x, 200, z],
                    state: registry.states.air,
                },
            ],
        )
        .unwrap();
    drop(writer);
    let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
    let (state, _, _) =
        crate::core::recover_tick_state(&config, &registry, &store, false, None, None).unwrap();
    assert_eq!(state.datapacks.selection(), &selected);
    assert!(
        state
            .world
            .village_sites_in_region((-68, -68), (101, 101))
            .is_empty()
    );
    let pos = crate::boundary::ChunkPos {
        x: site.start_chunk.0,
        z: site.start_chunk.1,
    };
    let chunk = state
        .world
        .chunk(pos)
        .expect("WAL replay generated the missing chunk");
    let starts = mc_world::structure_starts::read(&chunk.extras, chunk.pos, false).unwrap();
    assert!(
        !starts
            .iter()
            .any(|s| s.structure.as_str().starts_with("minecraft:village_"))
    );
    assert_eq!(state.world.block_at(x, 200, z), Some(registry.states.air));
    drop(state);
    let original_wal = std::fs::read_dir(dir.path().join("journal"))
        .unwrap()
        .map(|e| {
            let path = e.unwrap().path();
            (path.clone(), std::fs::read(path).unwrap())
        })
        .collect::<Vec<_>>();
    write(
        &root.join("no-villages/data"),
        &tags[0],
        br#"{"replace":true,"values":["minecraft:plains"]}"#,
    );
    let (state, _, _) =
        crate::core::recover_tick_state(&config, &registry, &store, false, None, None).unwrap();
    assert_eq!(state.datapacks.selection(), &selected);
    assert_eq!(state.world.block_at(x, 200, z), Some(registry.states.air));
    drop(state);
    for (path, bytes) in original_wal {
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn vanilla_only_uses_original_cache_without_staging_and_missing_sha_adopts() {
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("cache");
    synthetic_cache(&cache);
    let mut packs = PackSet::load(&dir.path().join("datapacks"), Selection::default()).unwrap();
    let inputs = packs.prepare_worldgen(dir.path(), Some(&cache)).unwrap();
    assert_eq!(inputs.root(), Some(cache.as_path()));
    assert!(inputs.stage.is_none());
    let config = crate::CoreConfig::new(dir.path().to_owned(), 42);
    let mut globals = crate::native_checkpoint::generation_records(&config).unwrap();
    globals.insert(
        "level_metadata".into(),
        crate::native_checkpoint::tag_record(0, &mc_nbt::Tag::Compound(vec![])).unwrap(),
    );
    assert!(inputs.stage_identity(&mut globals).unwrap().changed);
    assert!(!inputs.stage_identity(&mut globals).unwrap().changed);
}

#[test]
#[ignore = "requires the local vanilla gameplay cache; run with --include-ignored"]
fn existing_world_without_sha_adopts_selected_worldgen_and_allows_pack_additions() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = crate::CoreConfig::new(dir.path().to_owned(), 81);
    config.vanilla_data_dir = Some(
        std::env::var_os("KRASNOKAMEN_CONTENT_CACHE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla")
            }),
    );
    pack(
        &dir.path().join("datapacks"),
        "adopt",
        &[(
            "minecraft/tags/worldgen/biome/has_structure/village_plains.json",
            br#"{"replace":true,"values":[]}"#,
        )],
    );
    let selected = Selection {
        enabled: vec!["vanilla".into(), "file/adopt".into()],
        disabled: vec![],
    };
    crate::native_checkpoint::tests::write_manifest(
        &config,
        &crate::journal::CheckpointManifest {
            datapack_selection: Some(selected.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    let core = crate::Core::open(config.clone()).unwrap();
    core.shutdown();
    let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
    assert!(
        crate::terrain::read_data_field(&store.read_checkpoint().unwrap().globals, IDENTITY)
            .unwrap()
            .is_some()
    );
    drop(store);
    let core = crate::Core::open(config.clone()).unwrap();
    core.shutdown();
    pack(
        &dir.path().join("datapacks"),
        "added",
        &[(
            "minecraft/tags/worldgen/biome/has_structure/village_desert.json",
            br#"{"replace":true,"values":[]}"#,
        )],
    );
    let core = crate::Core::open(config).unwrap();
    core.shutdown();
}

#[test]
fn existing_tick_zero_without_sha_and_crash_before_checkpoint_repeat_adoption() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = crate::CoreConfig::new(dir.path().to_owned(), 42);
    config.checkpoint_every_ticks = u64::MAX;
    let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
    let mut registry = crate::registry::CoreRegistry::load(None).unwrap();
    crate::plugin_startup::apply(
        &mut registry,
        &mc_plugin_host::DeploymentContribution::default(),
        None,
        &[],
    )
    .unwrap();
    let registry = Arc::new(registry);
    for _ in 0..2 {
        let (state, manifest, _) =
            crate::core::recover_tick_state(&config, &registry, &store, false, None, None).unwrap();
        assert_eq!(manifest.tick, 0);
        assert!(
            crate::terrain::read_data_field(&state.native_globals, IDENTITY)
                .unwrap()
                .is_some()
        );
        drop(state); // Crash before a checkpoint publishes the in-memory adoption.
        assert!(
            crate::terrain::read_data_field(&store.read_checkpoint().unwrap().globals, IDENTITY)
                .unwrap()
                .is_none()
        );
    }
    drop(store);
    let core = crate::Core::open(config.clone()).unwrap();
    let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
    assert_eq!(store.read_checkpoint().unwrap().tick, 0);
    assert!(
        crate::terrain::read_data_field(&store.read_checkpoint().unwrap().globals, IDENTITY)
            .unwrap()
            .is_none()
    );
    drop(store);
    core.subscribe().blocking_recv().unwrap();
    core.shutdown();
    let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
    assert!(store.read_checkpoint().unwrap().tick > 0);
    assert!(
        crate::terrain::read_data_field(&store.read_checkpoint().unwrap().globals, IDENTITY)
            .unwrap()
            .is_some()
    );
}

#[test]
#[ignore = "requires the local vanilla gameplay cache; run with --include-ignored"]
fn removed_pack_saved_start_survives_live_wal_checkpoint_restart_and_reactivation() {
    let dir = tempfile::tempdir().unwrap();
    let cache = std::env::var_os("KRASNOKAMEN_CONTENT_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla"));
    let mut config = crate::CoreConfig::new(dir.path().to_owned(), 81);
    config.vanilla_data_dir = Some(cache.clone());
    config.checkpoint_every_ticks = u64::MAX;
    config.journal_fsync_every_batches = 1;
    let definition =
        std::fs::read(cache.join("data/minecraft/worldgen/structure/village_plains.json")).unwrap();
    pack(
        &dir.path().join("datapacks"),
        "preserve",
        &[(
            "minecraft/worldgen/structure/preserved_start.json",
            &definition,
        )],
    );
    let structure = id("minecraft:preserved_start");
    let core = crate::Core::open(config.clone()).unwrap();
    assert!(core.registry().datapack_structures().contains(&structure));
    let registry = Arc::clone(core.registry());
    let generator = core.overworld_generator().unwrap();
    core.shutdown();
    let pos = crate::boundary::ChunkPos { x: 1800, z: 1900 };
    let mut chunk = generator.generate(pos);
    let anchor = mc_world::BlockPos {
        x: pos.x * 16,
        y: 200,
        z: pos.z * 16,
    };
    mc_world::structure_starts::insert(
        &mut chunk,
        &mc_world::chunk::GeneratedStructureSite {
            structure: structure.clone(),
            start_chunk: (pos.x, pos.z),
            position: anchor,
        },
        anchor,
        anchor,
    );
    let original = chunk
        .extras
        .iter()
        .find(|(key, _)| key == "structures")
        .unwrap()
        .1
        .clone();
    let mut original_bytes = Vec::new();
    mc_nbt::write_named(&mut original_bytes, "structures", &original).unwrap();
    let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
    store
        .fill_generated(0, &[chunk], &registry.blocks, &registry.items)
        .unwrap();
    drop(store);
    std::fs::rename(
        dir.path().join("datapacks/preserve"),
        dir.path().join("pack-offline"),
    )
    .unwrap();
    let core = crate::Core::open(config.clone()).unwrap();
    assert!(!core.registry().datapack_structures().contains(&structure));
    let catalog = core.registry().structure_catalog.as_ref().unwrap().clone();
    assert_eq!(catalog.owners[structure.as_str()], "file/preserve");
    let mut ticks = core.subscribe();
    core.add_ticket(crate::core::RemoteTicket {
        owner: "overlay:preserved-start".into(),
        chunk: pos,
        radius: 0,
        reason: "saved start regression".into(),
        budget_chunks: 1,
    })
    .unwrap();
    let mut admitted = false;
    for _ in 0..100 {
        let output = ticks.blocking_recv().unwrap();
        if output
            .snapshot
            .chunks
            .iter()
            .any(|chunk| chunk.pos.x == pos.x && chunk.pos.z == pos.z)
        {
            admitted = true;
            break;
        }
    }
    assert!(admitted, "ticket admits the saved-start chunk");
    let command = format!("setblock {} 200 {} minecraft:stone", anchor.x, anchor.z);
    let (command, completed) = crate::command_console::ConsoleCommand::new(command);
    assert!(core.submit(crate::boundary::PlayerInput::ConsoleCommand(command)));
    completed.blocking_recv().unwrap();
    assert!(catalog.warned.lock().contains(&structure));
    assert_eq!(
        catalog
            .warned
            .lock()
            .iter()
            .filter(|id| *id == &structure)
            .count(),
        1
    );
    assert!(core.checkpoint());
    let registry = Arc::clone(core.registry());
    let generator = core.overworld_generator().unwrap();
    core.shutdown(); // The real tick WAL and native checkpoint save the touched chunk.
    let core = crate::Core::open(config.clone()).unwrap();
    assert!(!core.registry().datapack_structures().contains(&structure));
    core.shutdown();
    let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
    let chunk = store
        .read_chunk(
            0,
            pos,
            &registry.blocks,
            &registry.items,
            &registry.chest_loot,
            generator.as_ref(),
        )
        .unwrap()
        .unwrap();
    let stored = chunk
        .extras
        .iter()
        .find(|(key, _)| key == "structures")
        .unwrap()
        .1
        .clone();
    let mut stored_bytes = Vec::new();
    mc_nbt::write_named(&mut stored_bytes, "structures", &stored).unwrap();
    assert_eq!(stored_bytes, original_bytes);
    let stone = registry
        .blocks
        .by_name_and_props(&id("minecraft:stone"), &[])
        .unwrap();
    assert_eq!(chunk.get_block(0, 200, 0), Some(stone));
    drop(store);
    std::fs::rename(
        dir.path().join("pack-offline"),
        dir.path().join("datapacks/preserve"),
    )
    .unwrap();
    let core = crate::Core::open(config.clone()).unwrap();
    assert!(core.registry().datapack_structures().contains(&structure));
    assert!(
        core.registry()
            .structure_catalog
            .as_ref()
            .unwrap()
            .active
            .contains(&structure)
    );
    let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
    let restored = store
        .read_chunk(
            0,
            pos,
            &registry.blocks,
            &registry.items,
            &registry.chest_loot,
            generator.as_ref(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        restored
            .extras
            .iter()
            .find(|(key, _)| key == "structures")
            .unwrap()
            .1,
        original
    );
    drop(store);
    core.shutdown();
}

#[test]
fn biome_payload_entry_namespace_is_separate_from_its_resource_path() {
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("cache");
    synthetic_cache(&cache);
    write(
        &cache,
        "reports/registry_network_nbt/minecraft/worldgen/biome/minecraft/wire_only.nbt",
        b"registry path fixture",
    );
    write(
        &cache,
        "data/test/tags/worldgen/biome/wire.json",
        br#"{"values":["minecraft:wire_only"]}"#,
    );
    let mut packs = PackSet::load(&dir.path().join("datapacks"), Selection::default()).unwrap();
    let inputs = packs.prepare_worldgen(dir.path(), Some(&cache)).unwrap();
    assert!(
        inputs
            .biome_tags
            .biome_in_tag(&id("test:wire"), &id("minecraft:wire_only"))
    );
}

#[test]
#[ignore = "requires KRASNOKAMEN_CRYSTAL_DATAPACK (external generation-only fixture) and vanilla cache"]
fn selected_namespaced_crystal_fixture_generates_and_persists_exact_id() {
    fn copy(source: &Path, destination: &Path) {
        std::fs::create_dir_all(destination).unwrap();
        for entry in std::fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            assert!(!kind.is_symlink());
            if kind.is_dir() {
                copy(&entry.path(), &destination.join(entry.file_name()));
            } else {
                std::fs::copy(entry.path(), destination.join(entry.file_name())).unwrap();
            }
        }
    }
    let source = PathBuf::from(
        std::env::var_os("KRASNOKAMEN_CRYSTAL_DATAPACK").expect("external crystal datapack"),
    );
    let dir = tempfile::tempdir().unwrap();
    copy(
        &source,
        &dir.path().join("datapacks/crystal-generation-only"),
    );
    let mut config = crate::CoreConfig::new(dir.path().to_owned(), 20261115);
    config.checkpoint_every_ticks = u64::MAX;
    config.vanilla_data_dir = Some(
        std::env::var_os("KRASNOKAMEN_CONTENT_CACHE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla")
            }),
    );
    let structure = id("mvs:crystal");
    let loader =
        mc_data::village_data::VillageDataLoader::new(source.join("data/minecraft/worldgen"));
    let set = loader.load_structure_set(&structure).unwrap();
    let mc_data::village_data::PlacementSpec::RandomSpread(spec) = set.placement;
    let placement = mc_worldgen::village::placement::RandomSpreadPlacement {
        spacing: spec.spacing,
        separation: spec.separation,
        salt: i64::from(spec.salt),
        triangular: matches!(
            spec.spread_type,
            mc_data::village_data::RandomSpreadType::Triangular
        ),
    };
    let core = crate::Core::open(config.clone()).unwrap();
    assert!(core.registry().datapack_structures().contains(&structure));
    let registry = Arc::clone(core.registry());
    let generator = core.overworld_generator().unwrap();
    let mut found = None;
    'search: for radius in 1_i32..=4 {
        for z in -radius..=radius {
            for x in -radius..=radius {
                if x.abs().max(z.abs()) != radius {
                    continue;
                }
                let candidate = placement.potential_chunk(
                    config.seed,
                    x * placement.spacing,
                    z * placement.spacing,
                );
                if generator
                    .structure_sites_in_region(candidate, candidate)
                    .iter()
                    .any(|site| site.structure == structure)
                {
                    found = Some(candidate);
                    break 'search;
                }
            }
        }
    }
    let candidate = found.expect("no crystal start in bounded placement-cell search");
    let pos = mc_world::ChunkPos {
        x: candidate.0,
        z: candidate.1,
    };
    let chunk = generator.generate(pos);
    assert!(
        mc_world::structure_starts::read(&chunk.extras, pos, false)
            .unwrap()
            .iter()
            .any(|site| site.structure == structure)
    );
    let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
    assert!(
        crate::terrain::read_data_field(&store.read_checkpoint().unwrap().globals, IDENTITY)
            .unwrap()
            .is_some()
    );
    store
        .fill_generated(0, &[chunk], &registry.blocks, &registry.items)
        .unwrap();
    drop(store);
    core.shutdown();
    let core = crate::Core::open(config.clone()).unwrap();
    let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
    let chunk = store
        .read_chunk(
            0,
            pos,
            &registry.blocks,
            &registry.items,
            &registry.chest_loot,
            generator.as_ref(),
        )
        .unwrap()
        .unwrap();
    assert!(
        mc_world::structure_starts::read(&chunk.extras, pos, false)
            .unwrap()
            .iter()
            .any(|site| site.structure == structure)
    );
    eprintln!(
        "crystal lifecycle: seed={} chunk=({}, {}) exact_id={} native persisted/reopened; generation-only fixture, no loot",
        config.seed, pos.x, pos.z, structure
    );
    drop(store);
    core.shutdown();
}

#[cfg(unix)]
#[path = "datapacks_worldgen_crash_tests.rs"]
mod crash;

