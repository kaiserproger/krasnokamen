//! Offline pregeneration: rectangle coverage, resumability, lease refusal.

use super::*;

fn world_dir(slug: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("{slug}-{}", uuid::Uuid::new_v4()))
}

fn config(dir: &std::path::Path, seed: i64) -> CoreConfig {
    let mut config = CoreConfig::new(dir.to_path_buf(), seed);
    config.vanilla_data_dir =
        Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla"));
    config.checkpoint_every_ticks = u64::MAX;
    config
}

fn fresh_world(seed: i64) -> (std::path::PathBuf, CoreConfig) {
    let dir = world_dir("mc-core-pregenerate");
    let config = config(&dir, seed);
    (dir, config)
}

fn stored_positions(config: &CoreConfig) -> HashSet<ChunkPos> {
    let store = NativeStore::open(
        &config.world_dir.join("native/state.redb"),
        mc_core::native_checkpoint::native_identity(config),
    )
    .expect("store opens after pregeneration");
    let mut positions = HashSet::new();
    store
        .visit_chunk_positions(|dimension, pos| {
            if dimension == mc_core::native_checkpoint::dimension_id(Dimension::Overworld) {
                positions.insert(pos);
            }
            Ok(())
        })
        .unwrap();
    positions
}

fn no_progress(_: &PregenerateProgress) {}

#[test]
fn pregenerate_stores_the_rectangle_then_a_rerun_skips_every_stored_chunk() {
    let (dir, config) = fresh_world(42);
    let request = PregenerateRequest {
        radius: 32,
        center: Some([0, 0]),
    };
    let report = pregenerate(&config, &request, &mut no_progress).expect("pregenerate");
    assert_eq!(report.center, [0, 0]);
    assert_eq!(report.radius, 32);
    assert_eq!(report.chunk_min, [-2, -2]);
    assert_eq!(report.chunk_max, [2, 2]);
    assert_eq!(report.chunks, 25);
    assert_eq!(
        report.generated + report.already_stored,
        report.chunks,
        "every rectangle chunk is either generated now or already stored"
    );
    assert!(report.generated > 0, "a fresh world stores new chunks");
    let stored = stored_positions(&config);
    for z in -2..=2 {
        for x in -2..=2 {
            assert!(
                stored.contains(&ChunkPos { x, z }),
                "chunk ({x},{z}) must be persisted"
            );
        }
    }

    // Rerun: the same rectangle is fully covered, so nothing regenerates.
    let rerun = pregenerate(&config, &request, &mut no_progress).expect("rerun");
    assert_eq!(rerun.already_stored, 25);
    assert_eq!(rerun.generated, 0, "rerun must skip existing chunks");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn pregenerate_resumes_a_partial_rectangle_instead_of_regenerating_it() {
    let (dir, config) = fresh_world(7);
    let inner = PregenerateRequest {
        radius: 16,
        center: None,
    };
    let outer = PregenerateRequest {
        radius: 32,
        center: inner.center,
    };
    let first = pregenerate(&config, &inner, &mut no_progress).expect("inner");
    assert_eq!(first.chunks, 9);
    assert!(first.already_stored + first.generated == 9);

    let grown = pregenerate(&config, &outer, &mut no_progress).expect("grown");
    assert_eq!(grown.chunks, 25);
    assert_eq!(
        grown.already_stored, first.chunks,
        "the inner rectangle is reused, not regenerated"
    );
    assert_eq!(grown.generated, 25 - first.chunks);
    assert!(stored_positions(&config).len() >= 25);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn pregenerate_reports_progress_until_the_last_chunk() {
    let (dir, config) = fresh_world(11);
    let mut samples = Vec::new();
    let report = pregenerate(
        &config,
        &PregenerateRequest {
            radius: 24,
            center: Some([100, -100]),
        },
        &mut |progress| samples.push(*progress),
    )
    .unwrap();
    assert!(!samples.is_empty());
    let last = *samples.last().expect("final sample");
    assert_eq!(last.chunks, report.chunks);
    assert_eq!(last.generated, report.generated);
    assert_eq!(last.already_stored, report.already_stored);
    assert_eq!(report.chunk_min, [4, -8]);
    assert_eq!(report.chunk_max, [7, -5]);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn pregenerate_refuses_a_zero_radius_without_creating_the_world() {
    let dir = world_dir("mc-core-pregenerate-zero");
    let config = config(&dir, 3);
    let error = pregenerate(&config, &PregenerateRequest::new(0), &mut no_progress)
        .expect_err("zero radius")
        .to_string();
    assert!(error.contains("at least one block"), "{error}");
    assert!(
        !config.world_dir.exists(),
        "refusal happens before any open"
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn pregenerate_refuses_a_world_that_a_running_server_holds() {
    let (dir, config) = fresh_world(42);
    Core::open(config.clone()).expect("fresh world").shutdown();
    let world = mc_world::storage::open_for_lease(&config.world_dir).unwrap();
    world.try_lock().expect("test holds the world lease");
    let error = pregenerate(&config, &PregenerateRequest::new(16), &mut no_progress)
        .expect_err("leased world")
        .to_string();
    world.unlock().unwrap();
    assert!(error.contains("leased"), "{error}");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn pregenerate_uses_the_admitted_selected_worldgen_instead_of_rebuilding_vanilla() {
    let (baseline_dir, baseline_config) = fresh_world(81);
    let baseline = Core::open(baseline_config).unwrap();
    let site = baseline
        .overworld_generator()
        .unwrap()
        .village_sites_in_region((-68, -68), (101, 101))
        .into_iter()
        .next()
        .expect("baseline village");
    baseline.shutdown();
    std::fs::remove_dir_all(baseline_dir).unwrap();
    let (dir, config) = fresh_world(81);
    let pack = dir.join("datapacks/no-villages");
    let tags = pack.join("data/minecraft/tags/worldgen/biome/has_structure");
    std::fs::create_dir_all(&tags).unwrap();
    std::fs::write(
        pack.join("pack.mcmeta"),
        br#"{"pack":{"description":"selected worldgen"}}"#,
    )
    .unwrap();
    for name in ["plains", "desert", "savanna", "snowy", "taiga"] {
        std::fs::write(
            tags.join(format!("village_{name}.json")),
            br#"{"replace":true,"values":[]}"#,
        )
        .unwrap();
    }
    let pos = ChunkPos {
        x: site.start_chunk.0,
        z: site.start_chunk.1,
    };
    let request = PregenerateRequest {
        radius: 1,
        center: Some([pos.x * 16 + 8, pos.z * 16 + 8]),
    };
    let report = pregenerate(&config, &request, &mut no_progress).unwrap();
    assert_eq!(report.generated, 1);
    let core = Core::open(config.clone()).unwrap();
    let registry = Arc::clone(core.registry());
    core.shutdown();
    let store = NativeStore::open(
        &dir.join("native/state.redb"),
        mc_core::native_checkpoint::native_identity(&config),
    )
    .unwrap();
    let chunk = store
        .read_chunk(
            0,
            pos,
            &registry.blocks,
            &registry.items,
            &registry.chest_loot,
        )
        .unwrap()
        .unwrap();
    let starts = mc_world::structure_starts::read(&chunk.extras, pos, false).unwrap();
    assert!(
        !starts
            .iter()
            .any(|start| start.structure.as_str().starts_with("minecraft:village_"))
    );
    drop(store);
    std::fs::remove_dir_all(dir).unwrap();
}
