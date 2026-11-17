//! Offline prune/compact: rectangle arithmetic, dry run, regeneration, lease refusal.

use super::*;
use crate::pregenerate::{PregenerateProgress, PregenerateRequest, pregenerate};

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

fn no_progress(_: &PregenerateProgress) {}

fn prune_request(keep_radius: u32, dry_run: bool) -> PruneRequest {
    PruneRequest {
        keep_radius,
        center: Some([0, 0]),
        max_inhabited_ticks: 0,
        dry_run,
    }
}

#[test]
fn rectangle_covers_the_radius_in_chunks_including_negative_blocks() {
    assert_eq!(rectangle([0, 0], 16), ([-1, -1], [1, 1]));
    assert_eq!(rectangle([-1024, 2048], 256), ([-80, 112], [-48, 144]));
    assert_eq!(rectangle([5, -5], 1), ([0, -1], [0, -1]));
}

#[test]
fn prune_forgets_far_pregenerated_chunks_and_they_regenerate_on_demand() {
    let dir = world_dir("mc-core-prune");
    let config = config(&dir, 42);
    let request = PregenerateRequest {
        radius: 64,
        center: Some([0, 0]),
    };
    let first = pregenerate(&config, &request, &mut no_progress).expect("pregenerate");
    assert_eq!(first.chunks, 81);

    let dry = prune_world(&config, &prune_request(16, true)).expect("dry run");
    assert_eq!(dry.keep_chunk_min, [-1, -1]);
    assert_eq!(dry.keep_chunk_max, [1, 1]);
    assert_eq!(dry.chunks.examined, 81);
    assert_eq!(dry.chunks.pruned, 72);
    assert_eq!(dry.chunks.kept_inside_rectangle, 9);
    assert_eq!(
        dry.bytes_before, dry.bytes_after,
        "a dry run neither prunes nor compacts"
    );

    let report = prune_world(&config, &prune_request(16, false)).expect("prune");
    assert_eq!(report.chunks, dry.chunks);
    assert!(
        report.bytes_after < report.bytes_before,
        "{} -> {}",
        report.bytes_before,
        report.bytes_after
    );

    // The world still opens, keeps the inner chunks and regenerates the rest.
    let rerun = pregenerate(&config, &request, &mut no_progress).expect("rerun");
    assert_eq!(rerun.already_stored, 9);
    assert_eq!(rerun.generated, 72);
    std::fs::remove_dir_all(dir).ok();
}

fn published_checkpoint_tick(config: &CoreConfig) -> u64 {
    let store = mc_world::native::NativeReadOnlyStore::open_with_cache(
        &config.world_dir.join("native/state.redb"),
        mc_core::native_checkpoint::native_identity(config),
        16 * 1024 * 1024,
    )
    .expect("read-only store");
    let reader = store.checkpoint_reader().expect("reader");
    mc_core::native_checkpoint::published_manifest(config, &reader)
        .expect("manifest")
        .tick
}

/// Publish a journal batch above the checkpoint, as a stop whose final
/// checkpoint failed leaves one.
fn append_block_tail(config: &CoreConfig) {
    let durable = published_checkpoint_tick(config);
    let mut journal = mc_core::journal::JournalWriter::open(&config.world_dir, 1).expect("journal");
    journal
        .append(
            durable + 1,
            durable + 1,
            &[mc_core::journal::JournalEntry::Block {
                pos: [0, 64, 0],
                state: 1,
            }],
        )
        .expect("publish the tail");
}

/// A store whose published WAL tail the checkpoint does not cover is not
/// judged: prune refuses with the named error, dry run included, and classifies
/// or deletes nothing. The open/shutdown cycle `prune_world` runs folds the same
/// tail in and then proceeds.
#[test]
fn prune_refuses_a_published_tail_the_checkpoint_does_not_cover() {
    let dir = world_dir("mc-core-prune-uncheckpointed-tail");
    let config = config(&dir, 11);
    let request = PregenerateRequest::new(64);
    let first = pregenerate(&config, &request, &mut no_progress).expect("pregenerate");
    assert_eq!(first.chunks, 81);
    append_block_tail(&config);

    for dry_run in [false, true] {
        let error = prune_stopped(&config, &prune_request(16, dry_run), [0, 0])
            .expect_err("a store without its published tail is not judged");
        assert!(
            error
                .to_string()
                .contains(mc_core::native_checkpoint::UNCHECKPOINTED_TAIL),
            "{error}"
        );
    }

    // Nothing was classified or deleted: every chunk is still stored.
    let rerun = pregenerate(&config, &request, &mut no_progress).expect("rerun");
    assert_eq!(rerun.already_stored, first.chunks);
    assert_eq!(rerun.generated, 0);

    let report = prune_world(&config, &prune_request(16, false)).expect("recovered store");
    assert_eq!(report.chunks.pruned, 72);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn compact_world_keeps_every_chunk_and_never_grows_the_file() {
    let dir = world_dir("mc-core-compact");
    let config = config(&dir, 7);
    let request = PregenerateRequest::new(32);
    let first = pregenerate(&config, &request, &mut no_progress).expect("pregenerate");
    let report = compact_world(&config).expect("compact");
    assert_eq!(report.rewritten_rows, 0, "new rows are already packed");
    assert!(report.bytes_after <= report.bytes_before);
    let rerun = pregenerate(&config, &request, &mut no_progress).expect("rerun");
    assert_eq!(rerun.already_stored, first.chunks);
    assert_eq!(rerun.generated, 0);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn maintenance_refuses_a_missing_world_and_a_leased_one() {
    let missing = world_dir("mc-core-prune-missing");
    let error = prune_world(&config(&missing, 1), &prune_request(16, true))
        .expect_err("missing world")
        .to_string();
    assert!(error.contains("no native checkpoint"), "{error}");
    assert!(!missing.exists(), "must not create the world");
    assert!(compact_world(&config(&missing, 1)).is_err());

    let dir = world_dir("mc-core-prune-leased");
    let config = config(&dir, 3);
    Core::open(config.clone()).expect("fresh world").shutdown();
    let world = mc_world::storage::open_for_lease(&config.world_dir).unwrap();
    world.try_lock().expect("test holds the world lease");
    let error = compact_world(&config).expect_err("leased").to_string();
    assert!(error.contains("already leased"), "{error}");
    world.unlock().unwrap();
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn prune_refuses_a_negative_inhabited_limit() {
    let mut request = prune_request(16, true);
    request.max_inhabited_ticks = -1;
    assert!(prune_world(&config(&world_dir("mc-core-prune-neg"), 1), &request).is_err());
}
