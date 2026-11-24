use super::*;
use mc_core::core::Core;
use mc_world::native::NativeReadOnlyStore;
use redb::ReadableTable;
use std::path::PathBuf;

const CACHE: usize = 16 * 1024 * 1024;

fn config(dir: &std::path::Path, seed: i64) -> CoreConfig {
    let mut config = CoreConfig::new(dir.to_path_buf(), seed);
    config.vanilla_data_dir =
        Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla"));
    config.checkpoint_every_ticks = u64::MAX;
    config
}

/// A real world, rewritten to record `revision` exactly as an older build left it.
fn world_at(revision: u32) -> (PathBuf, CoreConfig) {
    let dir =
        std::env::temp_dir().join(format!("mc-core-worldgen-upgrade-{}", uuid::Uuid::new_v4()));
    let config = config(&dir, 42);
    Core::open(config.clone()).expect("fresh world").shutdown();
    NativeStore::replace_generation_contract(
        &dir.join("native/state.redb"),
        &contract(&config, mc_worldgen::WORLDGEN_REVISION).unwrap(),
        &contract(&config, revision).unwrap(),
    )
    .unwrap();
    (dir, config)
}

fn stored_identity(dir: &std::path::Path) -> [u8; 32] {
    NativeStore::read_identity(&dir.join("native/state.redb")).unwrap()
}

#[test]
fn interrupted_writer_world_upgrades_and_retry_is_idempotent() {
    const CHILD: &str = "KRASNOKAMEN_WORLDGEN_UPGRADE_CRASH_CHILD";
    if let Some(path) = std::env::var_os(CHILD) {
        let database = redb::Database::open(path).unwrap();
        let tx = database.begin_write().unwrap();
        {
            let mut globals = tx
                .open_table(redb::TableDefinition::<&str, &[u8]>::new("global"))
                .unwrap();
            let scoreboard = globals.get("scoreboard").unwrap().unwrap().value().to_vec();
            globals.insert("scoreboard", scoreboard.as_slice()).unwrap();
        }
        tx.commit().unwrap();
        // Keep the committed writer open, just as a killed upgrade process does.
        std::process::exit(39);
    }

    let old = mc_worldgen::WORLDGEN_REVISION - 1;
    let (dir, config) = world_at(old);
    let path = dir.join("native/state.redb");
    let before =
        NativeStore::inspect_read_only(&path, native_identity_at(&config, old), CACHE).unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "worldgen_upgrade::tests::interrupted_writer_world_upgrades_and_retry_is_idempotent",
            "--nocapture",
        ])
        .env(CHILD, &path)
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(39));

    let report = upgrade_worldgen_revision(&config).expect("recover and upgrade interrupted world");
    assert_eq!(report.from_revision, old);
    assert_eq!(report.to_revision, mc_worldgen::WORLDGEN_REVISION);
    assert!(report.rewritten);
    let after = NativeStore::inspect_read_only(
        &path,
        native_identity_at(&config, mc_worldgen::WORLDGEN_REVISION),
        CACHE,
    )
    .unwrap();
    assert_eq!(format!("{before:?}"), format!("{after:?}"));
    Core::open(config.clone())
        .expect("upgraded world opens")
        .shutdown();
    let again = upgrade_worldgen_revision(&config).unwrap();
    assert_eq!(again.from_revision, mc_worldgen::WORLDGEN_REVISION);
    assert!(!again.rewritten);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn older_revision_world_is_refused_then_upgraded_and_opens() {
    let old = mc_worldgen::WORLDGEN_REVISION - 1;
    let (dir, config) = world_at(old);
    let path = dir.join("native/state.redb");
    let old_identity = native_identity_at(&config, old);
    assert_eq!(stored_identity(&dir), old_identity);

    let refusal = Core::open(config.clone())
        .expect_err("default open fails closed")
        .to_string();
    assert!(refusal.starts_with("storage: invalid native record: identity mismatch"));
    assert!(refusal.contains("world upgrade-worldgen"), "{refusal}");
    // The rejected open leaves the older header identity in place.
    assert_eq!(stored_identity(&dir), old_identity);

    let before = NativeStore::inspect_read_only(&path, old_identity, CACHE).unwrap();
    let report = upgrade_worldgen_revision(&config).unwrap();
    assert_eq!(
        report,
        WorldgenUpgrade {
            from_revision: old,
            to_revision: mc_worldgen::WORLDGEN_REVISION,
            rewritten: true
        }
    );
    let identity = native_identity_at(&config, mc_worldgen::WORLDGEN_REVISION);
    assert_eq!(stored_identity(&dir), identity);
    // Stored chunks, entities and gameplay rows are exactly what they were.
    let after = NativeStore::inspect_read_only(&path, identity, CACHE).unwrap();
    assert_eq!(format!("{before:?}"), format!("{after:?}"));

    Core::open(config.clone())
        .expect("upgraded world opens")
        .shutdown();
    let store = NativeReadOnlyStore::open_with_cache(&path, identity, CACHE).unwrap();
    let image = store
        .checkpoint_reader()
        .unwrap()
        .read_checkpoint()
        .unwrap();
    let expected = mc_core::native_checkpoint::generation_records(&config).unwrap();
    for key in ["generation", "dimensions"] {
        assert_eq!(image.globals[key].payload, expected[key].payload);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn different_seed_is_refused_and_leaves_the_world_untouched() {
    let old = mc_worldgen::WORLDGEN_REVISION - 1;
    let (dir, config) = world_at(old);
    let other = self::config(&dir, config.seed + 1);
    let error = upgrade_worldgen_revision(&other).unwrap_err().to_string();
    assert!(
        error.contains("not only in the worldgen revision"),
        "{error}"
    );
    assert_eq!(stored_identity(&dir), native_identity_at(&config, old));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn newer_revision_is_refused_and_leaves_the_world_untouched() {
    let newer = mc_worldgen::WORLDGEN_REVISION + 1;
    let (dir, config) = world_at(newer);
    let error = upgrade_worldgen_revision(&config).unwrap_err().to_string();
    assert!(error.contains("refusing to downgrade"), "{error}");
    assert_eq!(stored_identity(&dir), native_identity_at(&config, newer));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn upgrade_is_idempotent() {
    let (dir, config) = world_at(mc_worldgen::WORLDGEN_REVISION - 1);
    assert!(upgrade_worldgen_revision(&config).unwrap().rewritten);
    let again = upgrade_worldgen_revision(&config).unwrap();
    assert_eq!(again.from_revision, mc_worldgen::WORLDGEN_REVISION);
    assert!(!again.rewritten);
    Core::open(config).expect("still opens").shutdown();
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn running_world_is_refused() {
    let dir =
        std::env::temp_dir().join(format!("mc-core-worldgen-upgrade-{}", uuid::Uuid::new_v4()));
    let config = config(&dir, 42);
    let core = Core::open(config.clone()).expect("fresh world");
    let error = upgrade_worldgen_revision(&config).unwrap_err().to_string();
    assert!(error.contains("already leased"), "{error}");
    core.shutdown();
    std::fs::remove_dir_all(dir).unwrap();
}

/// A world as v0.0.8 left it: its checkpoint has no `bossbars` global domain.
fn drop_bossbars(path: &std::path::Path) {
    let database = redb::Database::open(path).unwrap();
    let tx = database.begin_write().unwrap();
    {
        let mut global = tx
            .open_table(redb::TableDefinition::<&str, &[u8]>::new("global"))
            .unwrap();
        assert!(global.remove("bossbars").unwrap().is_some());
    }
    tx.commit().unwrap();
}

#[test]
fn older_storage_format_is_refused_with_export_import_guidance_and_not_written() {
    let (dir, config) = world_at(mc_worldgen::WORLDGEN_REVISION - 1);
    let path = dir.join("native/state.redb");
    drop_bossbars(&path);

    let guided = |error: &str| {
        assert!(
            error.contains("older Krasnokamen storage format"),
            "{error}"
        );
        assert!(error.contains("world export-anvil"), "{error}");
        assert!(error.contains("world import-anvil"), "{error}");
        assert!(!error.contains("upgrade-worldgen"), "{error}");
        assert!(!error.contains("missing required domain"), "{error}");
    };
    // The startup hint must not point at the upgrade that would only lead to a dead end.
    guided(&Core::open(config.clone()).unwrap_err().to_string());

    let before = std::fs::read(&path).unwrap();
    guided(&upgrade_worldgen_revision(&config).unwrap_err().to_string());
    assert_eq!(std::fs::read(&path).unwrap(), before, "refusal wrote bytes");
    assert_eq!(
        stored_identity(&dir),
        native_identity_at(&config, mc_worldgen::WORLDGEN_REVISION - 1)
    );

    // A world already rewritten to the current revision gets the same startup guidance.
    let (current_dir, current) = world_at(mc_worldgen::WORLDGEN_REVISION);
    drop_bossbars(&current_dir.join("native/state.redb"));
    guided(&Core::open(current).unwrap_err().to_string());
    std::fs::remove_dir_all(dir).unwrap();
    std::fs::remove_dir_all(current_dir).unwrap();
}

/// Main ruling R2: the worldgen-upgrade pre-check is a pure pair verification.
/// A store newer than its marker (a stale marker) passes it, and an upgrade that
/// then refuses (a different seed) leaves the marker and the committed records
/// exactly as they were.
#[test]
fn a_stale_marker_with_a_refused_upgrade_leaves_the_world_unchanged() {
    let dir = std::env::temp_dir().join(format!("mc-core-worldgen-stale-{}", uuid::Uuid::new_v4()));
    let mut config = config(&dir, 42);
    config.checkpoint_every_ticks = 1;
    let core = Core::open(config.clone()).expect("fresh world");
    let mut rx = core.subscribe();
    loop {
        if rx.blocking_recv().is_ok_and(|output| output.tick >= 1) {
            break;
        }
    }
    core.shutdown();
    drop(core);

    let marker = mc_world::store_marker::read(&dir)
        .unwrap()
        .expect("a Core world has a marker");
    assert!(marker.checkpoint_tick >= 1, "{marker:?}");
    mc_world::store_marker::write(
        &dir,
        marker.store_uuid,
        mc_world::store_marker::ROOT_BRANCH,
        marker.checkpoint_tick - 1,
    )
    .unwrap();
    let stale = mc_world::store_marker::read(&dir).unwrap().unwrap();
    let fingerprint = checkpoint_fingerprint(&config);
    let marker_bytes = std::fs::read(mc_world::store_marker::path(&dir)).unwrap();

    let mut wrong_seed = config.clone();
    wrong_seed.seed = 43;
    let error = upgrade_worldgen_revision(&wrong_seed)
        .expect_err("a different seed refuses")
        .to_string();
    assert!(error.contains("identity differs"), "{error}");

    assert_eq!(
        mc_world::store_marker::read(&dir).unwrap(),
        Some(stale),
        "a refused upgrade must not rewrite the stale marker"
    );
    assert_eq!(
        std::fs::read(mc_world::store_marker::path(&dir)).unwrap(),
        marker_bytes,
        "the marker file stays byte-identical"
    );
    assert_eq!(
        checkpoint_fingerprint(&config),
        fingerprint,
        "and no committed record changes"
    );
    std::fs::remove_dir_all(dir).ok();
}

/// Committed checkpoint records of the world at `config`, so a refusal can be
/// checked without depending on redb's own header rewrite on any writable open.
fn checkpoint_fingerprint(config: &CoreConfig) -> Vec<(String, u64, usize)> {
    let store = NativeReadOnlyStore::open_with_cache(
        &config.world_dir.join("native/state.redb"),
        mc_core::native_checkpoint::native_identity(config),
        CACHE,
    )
    .expect("read-only store");
    let image = store
        .checkpoint_reader()
        .expect("reader")
        .read_checkpoint_metadata()
        .expect("metadata");
    image
        .globals
        .into_iter()
        .map(|(key, record)| (key, record.captured_tick, record.payload.len()))
        .collect()
}

/// Main correction: the revision upgrade creates no checkpoint and the marker
/// holds only (store id, checkpoint tick), both unchanged by the rewrite. A
/// successful upgrade therefore leaves the marker byte-identical, and the world
/// opens at the current revision afterwards.
#[test]
fn a_successful_upgrade_leaves_the_marker_identical_and_the_world_opens() {
    let old = mc_worldgen::WORLDGEN_REVISION - 1;
    let (dir, config) = world_at(old);
    let marker_path = mc_world::store_marker::path(&dir);
    let before = std::fs::read(&marker_path).expect("marker");
    let report = upgrade_worldgen_revision(&config).expect("a matching pair upgrades");
    assert_eq!(report.from_revision, old);
    assert_eq!(report.to_revision, mc_worldgen::WORLDGEN_REVISION);
    assert!(report.rewritten);
    assert_eq!(
        std::fs::read(&marker_path).unwrap(),
        before,
        "the upgrade publishes no marker"
    );
    Core::open(config)
        .expect("the upgraded world opens")
        .shutdown();
    std::fs::remove_dir_all(dir).ok();
}
