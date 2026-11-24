use super::*;
use mc_world::native::{NativeReadOnlyStore, NativeStore};

/// A point where an interruption test stops a restore the way a killed process
/// would: [`interruption_point`] panics there, so unwinding skips every refusal
/// path's cleanup and only drops the leases, as the kernel drops a dead
/// process's locks.
#[derive(Debug)]
pub(super) enum Step<'a> {
    /// A file was copied and synced.
    Copied(&'a Path),
    /// A batch of the copied tail is about to move to the copy's branch.
    Rebinding,
    /// The staged world is complete and durable; the target is untouched.
    Publishing,
    /// The staged world was renamed over the target.
    Published,
}

type Interruption = Box<dyn FnMut(&Step<'_>) -> bool>;

thread_local! {
    static INTERRUPTION: std::cell::RefCell<Option<Interruption>> =
        const { std::cell::RefCell::new(None) };
}

/// Stop here when the armed interruption says so (once).
pub(super) fn interruption_point(step: &Step<'_>) {
    let stop = INTERRUPTION.with(|armed| {
        let mut armed = armed.borrow_mut();
        let stop = armed.as_mut().is_some_and(|stops| stops(step));
        if stop {
            *armed = None;
        }
        stop
    });
    assert!(!stop, "interrupted at {step:?}");
}

/// Run `run_restore` as a process killed at the first step `stops` names;
/// return whether it stopped there (false: it ran to an end of its own).
fn interrupted_restore(
    backup: &Path,
    target: &Path,
    stops: impl FnMut(&Step<'_>) -> bool + 'static,
) -> bool {
    INTERRUPTION.with(|armed| *armed.borrow_mut() = Some(Box::new(stops)));
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_restore(backup, target)
    }));
    let unfired = INTERRUPTION.with(|armed| armed.borrow_mut().take()).is_some();
    outcome.is_err() && !unfired
}

fn seed_world(root: &Path) {
    std::fs::create_dir_all(root.join("native")).unwrap();
    let store = NativeStore::create(&root.join(NATIVE_DATABASE), [7; 32]).unwrap();
    // Every world this build creates has a checkpoint marker; restore requires it.
    mc_world::store_marker::write(
        root,
        store.store_uuid(),
        mc_world::store_marker::ROOT_BRANCH,
        0,
    )
    .unwrap();
    drop(store);
    let mut journal =
        mc_core::journal::JournalWriter::open(root, 1, mc_core::journal::test_wal_identity(root))
            .unwrap();
    journal.append(1, 1, &[]).unwrap();
    journal.flush().unwrap();
    std::fs::create_dir_all(root.join("operator")).unwrap();
    std::fs::write(root.join("operator/world.json"), b"world metadata").unwrap();
}

fn fixture_backup(root: &Path) -> (PathBuf, PathBuf) {
    let world = root.join("world");
    seed_world(&world);
    let backup = root.join("backup");
    run_backup(&world, &backup).unwrap();
    (world, backup)
}

fn change_manifest(backup: &Path, change: impl FnOnce(&mut serde_json::Value)) {
    let path = backup.join(BACKUP_MANIFEST);
    let mut manifest = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    change(&mut manifest);
    std::fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
}

#[test]
fn backup_roundtrip_preserves_real_core_store_wal_and_metadata() {
    let root = tempfile::tempdir().unwrap();
    let world = root.path().join("world");
    let mut config = mc_core::CoreConfig::new(world.clone(), 42);
    config.vanilla_data_dir =
        Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla"));
    config.view_distance = 1;
    config.simulation_distance = 0;
    let core = mc_core::Core::open(config).unwrap();
    core.shutdown();
    drop(core);

    let mut identity = [0; 32];
    identity[..16].copy_from_slice(b"KrasnoV2Native\0\0");
    identity[16..24].copy_from_slice(&42_i64.to_be_bytes());
    identity[24..].copy_from_slice(&(mc_worldgen::WORLDGEN_REVISION as u64).to_be_bytes());
    let store =
        NativeReadOnlyStore::open_with_cache(&world.join(NATIVE_DATABASE), identity, 1024 * 1024)
            .unwrap();
    let image = store
        .checkpoint_reader()
        .unwrap()
        .read_checkpoint()
        .unwrap();
    let (store_uuid, _) = store.marker_fields().unwrap();
    drop(store);
    // The tail this test appends must belong to the store that will replay it:
    // zero fields are a value, not a wildcard, so a fixture identity is refused.
    let registry = mc_core::registry::CoreRegistry::load(Some(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../data/vanilla")
            .as_path(),
    ))
    .unwrap();
    let identity = mc_core::journal::WalIdentity::new(
        store_uuid,
        mc_world::store_marker::ROOT_BRANCH,
        &registry.blocks,
    );
    let mut journal = mc_core::journal::JournalWriter::open(&world, 1, identity).unwrap();
    journal.append(image.tick + 1, image.tick + 1, &[]).unwrap();
    journal.flush().unwrap();
    drop(journal);
    std::fs::write(world.join("world-metadata.json"), b"operator metadata").unwrap();
    for name in [".krasnokamen-world.lock", "session.lock"] {
        std::fs::write(world.join(name), b"stale lease, not state").unwrap();
    }

    let backup = root.path().join("backup");
    run_backup(&world, &backup).unwrap();
    let manifest = read_manifest(&backup).unwrap();
    assert_eq!(manifest.format, BACKUP_FORMAT);
    assert!(manifest.files.contains_key(NATIVE_DATABASE));
    assert!(
        manifest
            .files
            .keys()
            .any(|name| name.starts_with("journal/batch-"))
    );
    assert!(manifest.files.contains_key("world-metadata.json"));
    assert!(!manifest.files.contains_key(".krasnokamen-world.lock"));
    assert!(!manifest.files.contains_key("session.lock"));
    for (name, hash) in &manifest.files {
        assert_eq!(&digest_file(&backup.join(name)).unwrap(), hash);
    }

    let restored = root.path().join("restored");
    run_restore(&backup, &restored).unwrap();
    // A restored copy is the same world on a branch of its own: the marker and
    // the replayable tail name the new branch, every other byte is the backup's.
    let source_marker = mc_world::store_marker::read(&world).unwrap().unwrap();
    let restored_marker = mc_world::store_marker::read(&restored).unwrap().unwrap();
    assert_eq!(restored_marker.store_uuid, source_marker.store_uuid);
    assert_eq!(
        restored_marker.checkpoint_tick,
        source_marker.checkpoint_tick
    );
    assert_ne!(
        restored_marker.branch, source_marker.branch,
        "the restore starts a branch"
    );
    assert_ne!(restored_marker.branch, mc_world::store_marker::ROOT_BRANCH);
    for name in manifest.files.keys() {
        if name == mc_world::store_marker::MARKER_FILE {
            continue;
        }
        let source = std::fs::read(world.join(name)).unwrap();
        let mut copy = std::fs::read(restored.join(name)).unwrap();
        let replayable = name
            .strip_prefix("journal/batch-")
            .and_then(|range| range.strip_suffix(".bin"))
            .and_then(|range| range.split_once('-'))
            .is_some_and(|(_, hi)| hi.parse::<u64>().unwrap() > source_marker.checkpoint_tick);
        if replayable {
            // Bytes 24..32 of the frame header are its branch.
            assert_eq!(
                &copy[24..32],
                &restored_marker.branch,
                "{name} keeps the old branch"
            );
            copy[24..32].copy_from_slice(&source[24..32]);
        }
        assert_eq!(source, copy, "{name} differs after restore");
    }
    assert!(!restored.join(BACKUP_MANIFEST).exists());
    #[cfg(not(windows))]
    assert!(!restored.join(".krasnokamen-world.lock").exists());
    let recovered = mc_core::Core::open({
        let mut config = mc_core::CoreConfig::new(restored, 42);
        config.vanilla_data_dir =
            Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla"));
        config
    })
    .unwrap();
    recovered.shutdown();
}

#[test]
fn live_core_lease_blocks_backup_and_restore() {
    let root = tempfile::tempdir().unwrap();
    let (_, backup) = fixture_backup(root.path());
    let live = root.path().join("live");
    let core = mc_core::Core::open({
        let mut config = mc_core::CoreConfig::new(live.clone(), 42);
        config.vanilla_data_dir =
            Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla"));
        config
    })
    .unwrap();
    let backup_error = run_backup(&live, &root.path().join("live-backup")).unwrap_err();
    let restore_error = run_restore(&backup, &live).unwrap_err();
    core.shutdown();
    assert!(
        backup_error.to_string().contains("leased"),
        "{backup_error:#}"
    );
    assert!(
        restore_error.to_string().contains("leased"),
        "{restore_error:#}"
    );
    assert!(!root.path().join("live-backup").exists());
}

#[test]
fn empty_target_directory_lease_blocks_restore() {
    let root = tempfile::tempdir().unwrap();
    let (_, backup) = fixture_backup(root.path());
    let target = root.path().join("target");
    std::fs::create_dir(&target).unwrap();
    // Exactly the directory-inode lease held by Core::open, no v1 lock file.
    let lease = mc_world::storage::open_for_lease(&target).unwrap();
    lease.try_lock().unwrap();
    let error = run_restore(&backup, &target).unwrap_err();
    assert!(error.to_string().contains("leased"), "{error:#}");
    assert!(
        std::fs::read_dir(&target)
            .unwrap()
            .all(|entry| is_lease_file(&entry.unwrap().path()))
    );
    drop(lease);
    run_restore(&backup, &target).unwrap();
}

#[test]
fn native_redb_writer_blocks_backup_without_a_directory_lease() {
    let root = tempfile::tempdir().unwrap();
    let world = root.path().join("world");
    seed_world(&world);
    let store = NativeStore::open(&world.join(NATIVE_DATABASE), [7; 32]).unwrap();
    let out = root.path().join("backup");
    let error = run_backup(&world, &out).unwrap_err();
    // The pair check opens the store through its own handle, so another writer's
    // database lock is the refusal that names the source.
    assert!(
        error.to_string().contains("backup source store"),
        "{error:#}"
    );
    assert!(!out.exists());
    drop(store);
    run_backup(&world, &out).unwrap();
}

#[test]
fn backup_holds_the_native_redb_lock_against_new_writers() {
    let root = tempfile::tempdir().unwrap();
    let world = root.path().join("world");
    seed_world(&world);
    let _lease = ExclusiveLease::acquire(&world.join(NATIVE_DATABASE)).unwrap();
    let error = NativeStore::open(&world.join(NATIVE_DATABASE), [7; 32])
        .err()
        .unwrap();
    assert!(error.to_string().contains("already open"), "{error}");
}

#[test]
fn restore_refuses_a_backup_opened_by_a_native_writer() {
    let root = tempfile::tempdir().unwrap();
    let (_, backup) = fixture_backup(root.path());
    let _store = NativeStore::open(&backup.join(NATIVE_DATABASE), [7; 32]).unwrap();
    let target = root.path().join("target");
    let error = run_restore(&backup, &target).unwrap_err();
    assert!(error.to_string().contains("leased"), "{error:#}");
    assert!(!target.exists());
}

#[test]
fn offline_backup_restore_copies_database_under_real_file_leases() {
    let root = tempfile::tempdir().unwrap();
    let world = root.path().join("world");
    seed_world(&world);
    let expected = std::fs::read(world.join(NATIVE_DATABASE)).unwrap();
    let backup = root.path().join("backup");
    run_backup(&world, &backup).unwrap();
    let restored = root.path().join("restored");
    run_restore(&backup, &restored).unwrap();
    assert_eq!(
        std::fs::read(restored.join(NATIVE_DATABASE)).unwrap(),
        expected
    );
    assert_eq!(
        std::fs::read(restored.join("operator/world.json")).unwrap(),
        b"world metadata"
    );
    NativeReadOnlyStore::open_with_cache(&restored.join(NATIVE_DATABASE), [7; 32], 1024 * 1024)
        .unwrap();
}

#[test]
fn restore_refuses_nonempty_target_without_changing_data() {
    let root = tempfile::tempdir().unwrap();
    let (_, backup) = fixture_backup(root.path());
    let target = root.path().join("target");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("existing"), b"live data").unwrap();
    let error = run_restore(&backup, &target).unwrap_err();
    assert!(error.to_string().contains("not empty"), "{error:#}");
    assert_eq!(
        std::fs::read(target.join("existing")).unwrap(),
        b"live data"
    );
    let entries: Vec<_> = std::fs::read_dir(&target)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name != mc_world::storage::WORLD_LEASE_FILE_NAME)
        .collect();
    assert_eq!(entries, ["existing"]);
}

#[test]
fn every_hash_is_verified_before_any_target_data_is_written() {
    let root = tempfile::tempdir().unwrap();
    let (_, backup) = fixture_backup(root.path());
    // This sorts after journal/ and native/: a one-pass verify-and-copy restore
    // would already have written the database before noticing the mismatch.
    std::fs::write(backup.join("operator/world.json"), b"tampered").unwrap();
    let absent = root.path().join("absent/target");
    let error = run_restore(&backup, &absent).unwrap_err();
    assert!(
        error.to_string().contains("failed verification"),
        "{error:#}"
    );
    assert!(!root.path().join("absent").exists());
    let empty = root.path().join("empty");
    std::fs::create_dir(&empty).unwrap();
    assert!(run_restore(&backup, &empty).is_err());
    assert!(
        std::fs::read_dir(&empty)
            .unwrap()
            .all(|entry| entry.unwrap().file_name() == mc_world::storage::WORLD_LEASE_FILE_NAME)
    );
}

#[test]
fn restore_rejects_unsafe_manifest_paths_before_creating_target() {
    for name in [
        "../escape",
        "/absolute",
        "native/../../escape",
        "",
        ".",
        "native//state.redb",
        "native/./state.redb",
        "native\\state.redb",
        "manifest.json",
        "session.lock",
        ".krasnokamen-world.lock",
        "nul\0file",
    ] {
        let root = tempfile::tempdir().unwrap();
        let (_, backup) = fixture_backup(root.path());
        change_manifest(&backup, |manifest| {
            manifest["files"][name] = serde_json::json!("00");
        });
        let target = root.path().join("target");
        let error = run_restore(&backup, &target).unwrap_err();
        assert!(
            error.to_string().contains("unsafe path"),
            "{name:?}: {error:#}"
        );
        assert!(!target.exists());
        assert!(!root.path().join("escape").exists());
    }
}

#[test]
fn restore_rejects_missing_or_unhashed_files() {
    let root = tempfile::tempdir().unwrap();
    let (_, backup) = fixture_backup(root.path());
    let target = root.path().join("target");
    std::fs::write(backup.join("extra"), b"unhashed").unwrap();
    assert!(
        run_restore(&backup, &target)
            .unwrap_err()
            .to_string()
            .contains("match manifest")
    );
    std::fs::remove_file(backup.join("extra")).unwrap();
    std::fs::remove_file(backup.join("operator/world.json")).unwrap();
    assert!(
        run_restore(&backup, &target)
            .unwrap_err()
            .to_string()
            .contains("match manifest")
    );
    assert!(!target.exists());
}

#[test]
fn backup_and_restore_refuse_overlapping_roots_before_writes() {
    let root = tempfile::tempdir().unwrap();
    let (world, backup) = fixture_backup(root.path());
    for target in [
        world.clone(),
        world.join("nested"),
        root.path().to_path_buf(),
    ] {
        assert!(
            run_backup(&world, &target)
                .unwrap_err()
                .to_string()
                .contains("overlaps")
        );
    }
    assert!(!world.join("nested").exists());
    for target in [
        backup.clone(),
        backup.join("nested"),
        root.path().to_path_buf(),
    ] {
        assert!(
            run_restore(&backup, &target)
                .unwrap_err()
                .to_string()
                .contains("overlaps")
        );
    }
    assert!(!backup.join("nested").exists());
}

#[test]
fn backup_refuses_nonempty_output_without_replacing_files() {
    let root = tempfile::tempdir().unwrap();
    let world = root.path().join("world");
    seed_world(&world);
    let out = root.path().join("backup");
    std::fs::create_dir(&out).unwrap();
    std::fs::write(out.join("existing"), b"keep").unwrap();
    assert!(
        run_backup(&world, &out)
            .unwrap_err()
            .to_string()
            .contains("not empty")
    );
    assert_eq!(std::fs::read(out.join("existing")).unwrap(), b"keep");
    assert_eq!(std::fs::read_dir(&out).unwrap().count(), 1);
}

#[test]
fn restore_rejects_v1_or_incompatible_schema_without_creating_target() {
    let root = tempfile::tempdir().unwrap();
    let (_, backup) = fixture_backup(root.path());
    let target = root.path().join("target");
    change_manifest(&backup, |manifest| {
        manifest["format"] = serde_json::json!(1)
    });
    assert!(
        run_restore(&backup, &target)
            .unwrap_err()
            .to_string()
            .contains("format")
    );
    change_manifest(&backup, |manifest| {
        manifest["format"] = serde_json::json!(BACKUP_FORMAT);
        manifest["native_prototype_schema"] = serde_json::json!(u32::MAX);
    });
    assert!(
        run_restore(&backup, &target)
            .unwrap_err()
            .to_string()
            .contains("schema")
    );
    assert!(!target.exists());
}

#[cfg(unix)]
#[test]
fn backup_and_restore_reject_file_and_directory_symlinks() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().unwrap();
    let (world, backup) = fixture_backup(root.path());
    let out = root.path().join("other-backup");
    symlink(world.join("operator/world.json"), world.join("linked-file")).unwrap();
    assert!(
        run_backup(&world, &out)
            .unwrap_err()
            .to_string()
            .contains("symlink")
    );
    std::fs::remove_file(world.join("linked-file")).unwrap();
    symlink(world.join("operator"), world.join("linked-dir")).unwrap();
    assert!(
        run_backup(&world, &out)
            .unwrap_err()
            .to_string()
            .contains("symlink")
    );
    assert!(!out.exists());

    let target = root.path().join("target");
    std::fs::remove_file(backup.join("operator/world.json")).unwrap();
    symlink(
        world.join("operator/world.json"),
        backup.join("operator/world.json"),
    )
    .unwrap();
    assert!(
        run_restore(&backup, &target)
            .unwrap_err()
            .to_string()
            .contains("symlink")
    );
    std::fs::remove_file(backup.join("operator/world.json")).unwrap();
    std::fs::remove_dir(backup.join("operator")).unwrap();
    symlink(world.join("operator"), backup.join("operator")).unwrap();
    assert!(
        run_restore(&backup, &target)
            .unwrap_err()
            .to_string()
            .contains("symlink")
    );
    assert!(!target.exists());
}

#[cfg(unix)]
#[test]
fn restore_rejects_symlink_manifest_and_resolves_alias_overlap() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().unwrap();
    let (world, backup) = fixture_backup(root.path());
    let alias = root.path().join("alias");
    symlink(&backup, &alias).unwrap();
    assert!(
        run_restore(&backup, &alias.join("nested"))
            .unwrap_err()
            .to_string()
            .contains("overlaps")
    );
    assert!(!backup.join("nested").exists());
    let manifest = backup.join(BACKUP_MANIFEST);
    let outside = world.join("manifest-copy.json");
    std::fs::copy(&manifest, &outside).unwrap();
    std::fs::remove_file(&manifest).unwrap();
    symlink(outside, manifest).unwrap();
    let target = root.path().join("target");
    assert!(
        run_restore(&backup, &target)
            .unwrap_err()
            .to_string()
            .contains("regular file")
    );
    assert!(!target.exists());
}

#[test]
fn utc_stamp_formats_known_instants() {
    assert_eq!(utc_stamp(0), "19700101T000000Z");
    assert_eq!(utc_stamp(1_000_000_000), "20010909T014640Z");
    assert_eq!(utc_stamp(1_709_164_799), "20240228T235959Z");
    assert_eq!(utc_stamp(1_709_164_800), "20240229T000000Z");
}

/// Write a serving configuration the way an operator runs `restore`: the file
/// names the content cache and the plugin deployment, and the command must read
/// it instead of falling back to defaults.
fn restore_config(root: &Path, world: &Path) -> PathBuf {
    let content = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla");
    let path = root.join("operator.toml");
    std::fs::write(
        &path,
        format!(
            "[data]\nworld_dir = \"{}\"\nseed = 42\nvanilla_data_dir = \"{}\"\n",
            world.display(),
            content.display()
        ),
    )
    .unwrap();
    path
}

fn live_config(world: PathBuf) -> mc_core::CoreConfig {
    let mut config = mc_core::CoreConfig::new(world, 42);
    config.vanilla_data_dir =
        Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla"));
    config.view_distance = 1;
    config.simulation_distance = 0;
    config
}

#[test]
fn hot_backup_of_a_running_core_is_a_verified_restorable_backup() {
    let root = tempfile::tempdir().unwrap();
    let core = mc_core::Core::open(live_config(root.path().join("live"))).unwrap();
    let backups = root.path().join("backups");
    let (path, copied) = run_hot_backup(&core, &backups).unwrap();
    let name = path.file_name().unwrap().to_str().unwrap();
    assert!(name.starts_with("backup-"), "{name}");
    assert!(
        name.ends_with(&format!("-tick{}", copied.wal_tick)),
        "{name}"
    );
    let names: Vec<_> = std::fs::read_dir(&backups)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(
        names,
        [path.file_name().unwrap()],
        "no partial directory remains"
    );

    let manifest = read_manifest(&path).unwrap();
    assert_eq!(manifest.files, copied.files);
    for (file, hash) in &manifest.files {
        assert_eq!(&digest_file(&path.join(file)).unwrap(), hash);
    }
    let restored = root.path().join("restored");
    run_restore(&path, &restored).unwrap();
    // The live world keeps running while its copy is restored and opened.
    let recovered = mc_core::Core::open(live_config(restored)).unwrap();
    recovered.shutdown();

    core.shutdown();
    let error = run_hot_backup(&core, &backups).unwrap_err();
    assert!(error.to_string().contains("core stopped"), "{error:#}");
    assert_eq!(std::fs::read_dir(&backups).unwrap().count(), 1);
}

fn stopped_core_world(root: &Path) -> PathBuf {
    let world = root.join("world");
    let core = mc_core::Core::open(live_config(world.clone())).unwrap();
    core.shutdown();
    drop(core);
    world
}

/// Books R4-P1: the checkpoint marker is part of the backed-up state, and a
/// restore proves the copied marker and the copied store still belong together.
#[test]
fn offline_backup_and_restore_keep_the_store_and_its_marker_paired() {
    let root = tempfile::tempdir().unwrap();
    let world = stopped_core_world(root.path());
    let source = mc_world::store_marker::read(&world)
        .unwrap()
        .expect("a Core world has a checkpoint marker");
    let backup = root.path().join("backup");
    run_backup(&world, &backup).unwrap();
    assert!(
        read_manifest(&backup)
            .unwrap()
            .files
            .contains_key(mc_world::store_marker::MARKER_FILE),
        "the backup manifest covers the marker"
    );

    let target = root.path().join("restored");
    run_restore(&backup, &target).unwrap();
    let restored = mc_world::store_marker::read(&target)
        .unwrap()
        .expect("the marker is restored with the store");
    assert_eq!(
        (restored.store_uuid, restored.checkpoint_tick),
        (source.store_uuid, source.checkpoint_tick),
        "the marker is restored with the store"
    );
    let (store_uuid, store_tick) =
        NativeStore::read_marker_fields(&target.join(NATIVE_DATABASE)).unwrap();
    assert_eq!(store_uuid, source.store_uuid);
    assert_eq!(store_tick, Some(source.checkpoint_tick));
    // The pair is admitted, not merely copied: the restored world serves.
    let recovered = mc_core::Core::open(live_config(target)).unwrap();
    recovered.shutdown();
}

/// DIVERGED-BACKUP-COPIES-SHARE-STORE-IDENTITY: restore one backup twice and let
/// one copy run on alone. Its journal next to the other copy's store is refused
/// by name instead of replayed: the store id is shared by design (a restored
/// backup is the same world), so only the branch each restore mints tells the
/// two timelines apart.
#[test]
fn a_restored_copy_refuses_the_journal_of_another_copy_of_its_backup() {
    let root = tempfile::tempdir().unwrap();
    let world = stopped_core_world(root.path());
    let backup = root.path().join("backup");
    run_backup(&world, &backup).unwrap();
    let left = root.path().join("left");
    let right = root.path().join("right");
    run_restore(&backup, &left).unwrap();
    run_restore(&backup, &right).unwrap();

    // The right copy runs on and publishes a tail of its own timeline, bound
    // exactly as its serve binds it: store id, marker branch, build palette.
    let marker = mc_world::store_marker::read(&right).unwrap().unwrap();
    let registry = mc_core::registry::CoreRegistry::load(Some(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../data/vanilla")
            .as_path(),
    ))
    .unwrap();
    let identity =
        mc_core::journal::WalIdentity::of_world(&right, marker.store_uuid, &registry.blocks)
            .unwrap();
    let tick = marker.checkpoint_tick + 1;
    let mut journal = mc_core::journal::JournalWriter::open(&right, 1, identity).unwrap();
    journal.append(tick, tick, &[]).unwrap();
    journal.flush().unwrap();
    drop(journal);

    // The operator mistake the row names: the right copy's journal ends up next
    // to the left copy's store, in exactly the tick range that continues it.
    if left.join("journal").exists() {
        std::fs::remove_dir_all(left.join("journal")).unwrap();
    }
    recopy_tree(&right.join("journal"), &left.join("journal"));

    let opened = mc_core::Core::open(live_config(left));
    let refusal = opened.as_ref().err().map(ToString::to_string);
    if let Ok(core) = opened {
        core.shutdown();
    }
    assert!(
        refusal
            .as_deref()
            .is_some_and(|text| text.contains("branch")),
        "the left copy replayed the right copy's journal: {refusal:?}"
    );
}

#[test]
fn restore_refuses_a_marker_from_another_store() {
    let root = tempfile::tempdir().unwrap();
    let world = stopped_core_world(root.path());
    let source = mc_world::store_marker::read(&world).unwrap().unwrap();
    let backup = root.path().join("backup");
    run_backup(&world, &backup).unwrap();

    // Replace the copied marker with a foreign store's marker and keep the
    // manifest honest, so only the pair check can refuse the restore.
    let foreign = [0x77; 16];
    mc_world::store_marker::write(
        &backup,
        foreign,
        mc_world::store_marker::ROOT_BRANCH,
        source.checkpoint_tick,
    )
    .unwrap();
    change_manifest(&backup, |manifest| {
        let digest = digest_file(&backup.join(mc_world::store_marker::MARKER_FILE)).unwrap();
        manifest["files"][mc_world::store_marker::MARKER_FILE] = serde_json::Value::String(digest);
    });

    let target = root.path().join("restored");
    let error = run_restore(&backup, &target).unwrap_err().to_string();
    assert!(error.contains("restored world is inconsistent"), "{error}");
    assert!(
        error.contains(&uuid::Uuid::from_bytes(foreign).to_string()),
        "{error}"
    );
    assert!(
        !target.join(NATIVE_DATABASE).exists(),
        "a refused restore keeps no store"
    );
    assert!(
        !root.path().join("restored.restore-staging").exists(),
        "a refused restore keeps no staging directory"
    );
}

/// A stopped Core world with `ticks` one-tick batches published above its
/// checkpoint, bound exactly as its serve binds them, backed up offline.
fn backup_with_tail(root: &Path, ticks: u64) -> (PathBuf, mc_world::store_marker::StoreMarker) {
    let world = stopped_core_world(root);
    let marker = mc_world::store_marker::read(&world).unwrap().unwrap();
    let registry = mc_core::registry::CoreRegistry::load(Some(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../data/vanilla")
            .as_path(),
    ))
    .unwrap();
    let identity =
        mc_core::journal::WalIdentity::of_world(&world, marker.store_uuid, &registry.blocks)
            .unwrap();
    let mut journal = mc_core::journal::JournalWriter::open(&world, 1, identity).unwrap();
    for tick in marker.checkpoint_tick + 1..=marker.checkpoint_tick + ticks {
        journal.append(tick, tick, &[]).unwrap();
    }
    journal.flush().unwrap();
    drop(journal);
    let backup = root.join("backup");
    run_backup(&world, &backup).unwrap();
    (backup, marker)
}

/// Branch bytes (frame offset 24..32) of every batch in `world` above `tick`.
fn tail_branches(world: &Path, tick: u64) -> Vec<[u8; 8]> {
    let mut branches = Vec::new();
    for entry in std::fs::read_dir(world.join("journal")).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        let replayable = name
            .strip_prefix("batch-")
            .and_then(|range| range.strip_suffix(".bin"))
            .and_then(|range| range.split_once('-'))
            .is_some_and(|(_, hi)| hi.parse::<u64>().unwrap() > tick);
        if replayable {
            let bytes = std::fs::read(&path).unwrap();
            branches.push(bytes[24..32].try_into().unwrap());
        }
    }
    branches
}

/// What an interrupted restore left at `target` must not be a world: at most
/// the empty directory the restore leased (and its lease file).
fn assert_no_world_at(target: &Path, backup: &mc_world::store_marker::StoreMarker) {
    let left: Vec<_> = std::fs::read_dir(target)
        .map(|entries| {
            entries
                .map(|entry| entry.unwrap().path())
                .filter(|path| !is_lease_file(path))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        left.is_empty(),
        "an interrupted restore left {left:?} at the target, marker {:?}; the backup's branch is {:?}",
        mc_world::store_marker::read(target).ok().flatten(),
        backup.branch,
    );
}

/// Opening the staging directory as a world is refused: its store has a
/// checkpoint and the copy's own marker is not written yet.
fn assert_staging_does_not_open(staging: &Path) {
    assert_eq!(mc_world::store_marker::read(staging).unwrap(), None);
    let opened = mc_core::Core::open(live_config(staging.to_path_buf()));
    let refusal = opened.as_ref().err().map(ToString::to_string);
    if let Ok(core) = opened {
        core.shutdown();
    }
    assert!(
        refusal
            .as_deref()
            .is_some_and(|text| text.contains("is missing")),
        "the staging directory opened as a world: {refusal:?}"
    );
}

/// Review blocker (DIVERGED-BACKUP-COPIES-SHARE-STORE-IDENTITY): a restore
/// killed after its last copy and before its first rebind must leave nothing
/// that opens. Copied in place, that copy is the backup's store, marker and
/// tail on the backup's own branch, which every other copy of the backup shares.
#[test]
fn a_restore_interrupted_before_its_first_rebind_leaves_nothing_that_opens() {
    let root = tempfile::tempdir().unwrap();
    let (backup, source) = backup_with_tail(root.path(), 2);
    let last = read_manifest(&backup)
        .unwrap()
        .files
        .into_keys()
        .rfind(|name| name != mc_world::store_marker::MARKER_FILE)
        .unwrap();
    let target = root.path().join("restored");
    let staging = root.path().join("restored.restore-staging");
    let interrupted = interrupted_restore(&backup, &target, move |step| {
        matches!(step, Step::Copied(path) if path.ends_with(&last))
    });
    assert!(interrupted, "the restore never copied its last file");
    assert_no_world_at(&target, &source);
    assert_staging_does_not_open(&staging);
    assert!(
        tail_branches(&staging, source.checkpoint_tick)
            .iter()
            .all(|branch| *branch == source.branch),
        "nothing was rebound yet"
    );

    // The next restore refuses the leftover instead of reusing or removing it.
    let error = run_restore(&backup, &target).unwrap_err().to_string();
    assert!(error.contains("interrupted restore"), "{error}");
    std::fs::remove_dir_all(&staging).unwrap();
    run_restore(&backup, &target).unwrap();
    let restored = mc_world::store_marker::read(&target).unwrap().unwrap();
    assert_ne!(restored.branch, source.branch);
}

/// A restore killed between two batches of its tail rebind leaves one batch on
/// the new branch and one on the backup's, with no marker naming either: the
/// staging directory does not open and the target holds nothing.
#[test]
fn a_restore_interrupted_inside_its_tail_rebind_leaves_nothing_that_opens() {
    let root = tempfile::tempdir().unwrap();
    let (backup, source) = backup_with_tail(root.path(), 2);
    let target = root.path().join("restored");
    let staging = root.path().join("restored.restore-staging");
    let mut rebinds = 0;
    let interrupted = interrupted_restore(&backup, &target, move |step| {
        rebinds += usize::from(matches!(step, Step::Rebinding));
        rebinds == 2
    });
    assert!(interrupted, "the restore never reached its second rebind");
    assert_no_world_at(&target, &source);
    let branches = tail_branches(&staging, source.checkpoint_tick);
    assert_eq!(branches.len(), 2, "{branches:?}");
    assert_eq!(
        branches
            .iter()
            .filter(|branch| **branch == source.branch)
            .count(),
        1,
        "exactly one batch moved: {branches:?}"
    );
    assert_staging_does_not_open(&staging);
}

/// A restore killed with its staged world complete and durable, just before
/// publication, leaves the target exactly as it was (here an existing empty
/// directory with a stale lock file) and the full copy beside it.
#[test]
fn a_restore_interrupted_before_publication_leaves_the_target_untouched() {
    let root = tempfile::tempdir().unwrap();
    let (backup, source) = backup_with_tail(root.path(), 2);
    let target = root.path().join("restored");
    let staging = root.path().join("restored.restore-staging");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("session.lock"), b"stale lease, not state").unwrap();
    let interrupted =
        interrupted_restore(&backup, &target, |step| matches!(step, Step::Publishing));
    assert!(interrupted, "the restore never reached publication");
    let left: Vec<_> = std::fs::read_dir(&target)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name != mc_world::storage::WORLD_LEASE_FILE_NAME)
        .collect();
    assert_eq!(left, ["session.lock"], "the target changed before publication");
    let staged = mc_world::store_marker::read(&staging).unwrap().unwrap();
    assert_eq!(
        (staged.store_uuid, staged.checkpoint_tick),
        (source.store_uuid, source.checkpoint_tick)
    );
    assert_ne!(staged.branch, source.branch);
    assert_ne!(staged.branch, mc_world::store_marker::ROOT_BRANCH);
    assert_eq!(
        tail_branches(&staging, source.checkpoint_tick),
        [staged.branch; 2]
    );

    std::fs::remove_dir_all(&staging).unwrap();
    run_restore(&backup, &target).unwrap();
    assert!(!target.join("session.lock").exists());
    let recovered = mc_core::Core::open(live_config(target)).unwrap();
    recovered.shutdown();
}

/// A restore killed right after the rename, before its final directory sync,
/// has published the complete world: its own branch, its rebound tail, no
/// staging directory left, and it opens.
#[test]
fn a_restore_interrupted_after_publication_leaves_the_complete_world() {
    let root = tempfile::tempdir().unwrap();
    let (backup, source) = backup_with_tail(root.path(), 2);
    let target = root.path().join("restored");
    let interrupted =
        interrupted_restore(&backup, &target, |step| matches!(step, Step::Published));
    assert!(interrupted, "the restore never published");
    assert!(!root.path().join("restored.restore-staging").exists());
    assert!(target.join("journal").is_dir());
    let restored = mc_world::store_marker::read(&target).unwrap().unwrap();
    assert_ne!(restored.branch, source.branch);
    assert_ne!(restored.branch, mc_world::store_marker::ROOT_BRANCH);
    assert_eq!(
        tail_branches(&target, source.checkpoint_tick),
        [restored.branch; 2]
    );
    let recovered = mc_core::Core::open(live_config(target)).unwrap();
    recovered.shutdown();
}

/// Replay refuses a restored copy without a journal directory (a lost tail),
/// so a restore always publishes one and a backup of a copy whose journal is
/// empty keeps the empty directory: that backup verifies.
#[test]
fn a_backup_of_a_restored_copy_with_an_empty_journal_verifies() {
    let root = tempfile::tempdir().unwrap();
    let world = stopped_core_world(root.path());
    let first = root.path().join("first");
    run_backup(&world, &first).unwrap();
    let restored = root.path().join("restored");
    run_restore(&first, &restored).unwrap();
    assert!(restored.join("journal").is_dir());
    let second = root.path().join("second");
    run_backup(&restored, &second).unwrap();
    assert!(second.join("journal").is_dir());
    verify_backup(&second, &live_config(restored)).unwrap();
}

/// Hot backup → the real `restore` command with installed content: the copy of
/// a running store needs redb repair, so verification must read the identity,
/// store id and checkpoint from the one verification instance (a repaired
/// scratch copy) instead of opening the archive read-only first and aborting.
/// The CLI path is what an operator runs; `run_restore` alone is not evidence.
#[tokio::test]
async fn the_restore_command_accepts_a_hot_backup_verified_with_content() {
    use clap::Parser;

    let root = tempfile::tempdir().unwrap();
    let core = mc_core::Core::open(live_config(root.path().join("live"))).unwrap();
    let backups = root.path().join("backups");
    let (path, copied) = run_hot_backup(&core, &backups).unwrap();
    core.shutdown();
    drop(core);

    let restored = root.path().join("restored");
    let cfg = restore_config(root.path(), &restored);
    let cli = crate::operator_cli::Cli::try_parse_from([
        "krasnokamen",
        "--config",
        cfg.to_str().unwrap(),
        "restore",
        "--from",
        path.to_str().unwrap(),
        "--world-dir",
        restored.to_str().unwrap(),
    ])
    .unwrap();
    let mut cfg = crate::operator_cli::load_config(&cli).expect("read the operator configuration");
    assert!(
        crate::operator_cli::run_offline(&cli, &mut cfg)
            .await
            .expect("the restore command verifies and restores a hot backup"),
        "restore is an offline command"
    );

    let marker = mc_world::store_marker::read(&restored)
        .unwrap()
        .expect("the restore keeps the checkpoint marker");
    assert_eq!(
        marker.checkpoint_tick, copied.checkpoint_tick,
        "the restored marker is the copied checkpoint"
    );
    if copied.wal_tick > copied.checkpoint_tick {
        let tail: Vec<_> = std::fs::read_dir(restored.join("journal"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        let last = format!("-{:020}.bin", copied.wal_tick);
        assert!(
            tail.iter().any(|name| name.ends_with(&last)),
            "the restored journal carries the copied tail up to tick {}: {tail:?}",
            copied.wal_tick
        );
    }
    let recovered = mc_core::Core::open(live_config(restored)).unwrap();
    recovered.shutdown();
}

/// A world a serve produced records its plugin startup identity (the registry a
/// serve runs on, plugin contribution included). The `restore` command must
/// verify such a copy with that same registry and keep the record, so a
/// restored world is one a serve still accepts.
#[tokio::test]
async fn the_restore_command_verifies_a_world_that_records_its_plugin_startup_identity() {
    use clap::Parser;

    let root = tempfile::tempdir().unwrap();
    let core = mc_core::Core::open(live_config(root.path().join("live"))).unwrap();
    let backups = root.path().join("backups");
    let (path, _) = run_hot_backup(&core, &backups).unwrap();
    core.shutdown();
    drop(core);

    let restored = root.path().join("restored");
    let cli = crate::operator_cli::Cli::try_parse_from([
        "krasnokamen",
        "restore",
        "--from",
        path.to_str().unwrap(),
        "--world-dir",
        restored.to_str().unwrap(),
    ])
    .unwrap();
    let mut cfg = crate::config::EdgeConfig::default();
    cfg.data.vanilla_data_dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla");
    assert!(
        crate::operator_cli::run_offline(&cli, &mut cfg)
            .await
            .expect("the restore command verifies a copy with the plugins a serve resolves"),
        "restore is an offline command"
    );

    let recovered = mc_core::native_checkpoint::recover_offline(&live_config(restored.clone()))
        .expect("the restored world recovers offline");
    assert!(
        recovered.checkpoint.globals.contains_key("plugin_startup"),
        "the restored world keeps the plugin startup identity a serve enforces"
    );
}

/// A real first-party guest whose startup identity contributes native types
/// (`chisel` declares items, recipes and tools; `basic-economy` contributes an
/// empty identity and cannot distinguish a deployment from none).
fn real_plugin_package() -> PathBuf {
    let package = crate::plugin_tests::default_plugins_root().join("chisel");
    for file in ["plugin.toml", "plugin.wasm"] {
        assert!(
            package.join(file).is_file(),
            "missing integration prerequisite: {}; deploy the real first-party chisel component or set KRASNOKAMEN_DEFAULT_PLUGINS_ROOT; no substitute guest is allowed",
            package.join(file).display()
        );
    }
    package
        .canonicalize()
        .expect("resolve deployed sibling package")
}

fn deploy_plugin(source: &Path, destination: &Path) {
    let package = destination.join("chisel");
    std::fs::create_dir_all(&package).expect("create isolated deployment");
    for file in ["plugin.toml", "plugin.wasm"] {
        std::fs::copy(source.join(file), package.join(file))
            .expect("copy actual deployed first-party package bytes");
    }
    recopy_tree(&source.join("client"), &package.join("client"));
}

/// Optional client bundles stay optional, but a deployment that carries them
/// must look exactly like the sibling package it came from.
fn recopy_tree(source: &Path, destination: &Path) {
    if !source.is_dir() {
        return;
    }
    std::fs::create_dir_all(destination).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.path().is_dir() {
            recopy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// The operator's `server.toml`: content, world and (optionally) the plugin
/// deployment the CLI must read for `backup` and `restore`.
fn deployment_config(root: &Path, name: &str, world: &Path, plugins: Option<&Path>) -> PathBuf {
    let content = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla");
    let path = root.join(format!("{name}.toml"));
    let plugin_section = plugins.map_or_else(String::new, |directory| {
        format!("[plugins]\ndirectory = \"{}\"\n", directory.display())
    });
    std::fs::write(
        &path,
        format!(
            "[data]\nworld_dir = \"{}\"\nseed = 42\nvanilla_data_dir = \"{}\"\n{plugin_section}",
            world.display(),
            content.display()
        ),
    )
    .unwrap();
    path
}

/// Served with the real first-party guest, a world records that plugin startup
/// identity; `backup` and `restore` through the CLI must read `--config` and
/// verify the copy under the same deployment, and refuse it when the
/// configuration names no plugins.
#[tokio::test]
async fn the_cli_backup_and_restore_verify_under_the_configured_plugin_deployment() {
    use clap::Parser;

    let root = tempfile::tempdir().unwrap();
    let deployment = root.path().join("plugins");
    deploy_plugin(&real_plugin_package(), &deployment);
    let live = root.path().join("live");
    let mut config = live_config(live.clone());
    config.plugin_directory = Some(deployment.clone());
    let core = mc_core::Core::open(config).unwrap();
    core.shutdown();
    drop(core);

    let backups = root.path().join("backups");
    let with_plugins = deployment_config(root.path(), "with-plugins", &live, Some(&deployment));
    let backup_cli = crate::operator_cli::Cli::try_parse_from([
        "krasnokamen",
        "--config",
        with_plugins.to_str().unwrap(),
        "backup",
        "--world-dir",
        live.to_str().unwrap(),
        "--out",
        backups.to_str().unwrap(),
    ])
    .unwrap();
    let mut cfg =
        crate::operator_cli::load_config(&backup_cli).expect("read the operator configuration");
    assert_eq!(
        cfg.plugins.directory.as_deref(),
        Some(deployment.as_path()),
        "the CLI reads the deployment the operator configured"
    );
    assert!(
        crate::operator_cli::run_offline(&backup_cli, &mut cfg)
            .await
            .expect("the backup command verifies a world served with its deployment"),
        "backup is an offline command"
    );

    let restored = root.path().join("restored");
    let restore_cli = crate::operator_cli::Cli::try_parse_from([
        "krasnokamen",
        "--config",
        with_plugins.to_str().unwrap(),
        "restore",
        "--from",
        backups.to_str().unwrap(),
        "--world-dir",
        restored.to_str().unwrap(),
    ])
    .unwrap();
    let mut cfg =
        crate::operator_cli::load_config(&restore_cli).expect("read the operator configuration");
    assert!(
        crate::operator_cli::run_offline(&restore_cli, &mut cfg)
            .await
            .expect("the restore command verifies the copy under the same deployment"),
        "restore is an offline command"
    );
    let mut served = live_config(restored.clone());
    served.plugin_directory = Some(deployment.clone());
    mc_core::Core::open(served)
        .expect("the restored world serves with the deployment")
        .shutdown();

    // The same backup restored with a configuration that names no deployment is
    // refused, and the refusal names the plugin startup identity a serve checks.
    let elsewhere = root.path().join("restored-without-plugins");
    let without_plugins = deployment_config(root.path(), "without-plugins", &elsewhere, None);
    let mismatch_cli = crate::operator_cli::Cli::try_parse_from([
        "krasnokamen",
        "--config",
        without_plugins.to_str().unwrap(),
        "restore",
        "--from",
        backups.to_str().unwrap(),
        "--world-dir",
        elsewhere.to_str().unwrap(),
    ])
    .unwrap();
    let mut cfg =
        crate::operator_cli::load_config(&mismatch_cli).expect("read the operator configuration");
    let error = format!(
        "{:#}",
        crate::operator_cli::run_offline(&mismatch_cli, &mut cfg)
            .await
            .expect_err("a copy served with plugins is refused without them")
    );
    assert!(error.contains("plugin startup"), "{error}");
}
