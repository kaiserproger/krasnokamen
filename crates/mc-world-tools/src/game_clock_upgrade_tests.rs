use super::*;
use mc_core::journal::{CheckpointManifest, JournalEntry, JournalWriter};
use mc_world::native::NativeCheckpoint;
use mc_world::store_marker::{self, MARKER_FILE};
use std::path::PathBuf;

type Fault = Box<dyn FnOnce(&Path)>;

thread_local! {
    static BEFORE_MARKER: std::cell::RefCell<Option<Fault>> =
        const { std::cell::RefCell::new(None) };
}

/// Run the armed fault (once) between the converted commit and the marker write.
pub(super) fn before_marker(copy: &Path) {
    if let Some(fault) = BEFORE_MARKER.with(|armed| armed.borrow_mut().take()) {
        fault(copy);
    }
}

/// A stopped root-branch copy whose checkpoint and one-batch tail look like a
/// legacy frozen clock written by an older binary.
struct LegacyCopy {
    copy: PathBuf,
    config: CoreConfig,
    checkpoint: u64,
    store_uuid: [u8; 16],
    batch: String,
    before: Vec<u8>,
}

impl LegacyCopy {
    /// The configuration of an operator whose original world is elsewhere.
    fn operator_config(&self) -> CoreConfig {
        let mut operator = self.config.clone();
        operator.world_dir = self.copy.join("original");
        std::fs::create_dir(&operator.world_dir).unwrap();
        operator
    }
}

fn legacy_copy() -> LegacyCopy {
    let copy = std::env::temp_dir().join(format!("game-clock-conversion-{}", uuid::Uuid::new_v4()));
    let mut config = CoreConfig::new(copy.clone(), 42);
    config.vanilla_data_dir =
        Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla"));
    config.start_frozen = true;
    config.view_distance = 0;
    config.simulation_distance = 0;
    let core = mc_core::Core::open(config.clone()).unwrap();
    core.subscribe().blocking_recv().unwrap();
    core.shutdown();
    drop(core);
    let store = NativeStore::open_checked(&copy, native_identity(&config)).unwrap();
    let mut image = store.read_checkpoint().unwrap();
    let checkpoint = image.tick;
    let mut manifest: CheckpointManifest =
        serde_json::from_slice(&image.globals["checkpoint"].payload).unwrap();
    let climate = manifest.climate.as_mut().unwrap();
    climate.sim_debt_anchored_since = None;
    if let mc_core::weather::GameClockImage::Held { held_since, .. } = &mut climate.game_clock {
        *held_since = None;
    } else {
        panic!("frozen checkpoint required")
    }
    let tail_climate = *climate;
    for debt in &mut manifest.debt {
        debt.held_anchor = None;
    }
    for image in manifest.dimensions.values_mut() {
        for debt in &mut image.debt {
            debt.held_anchor = None;
        }
    }
    image.globals.get_mut("checkpoint").unwrap().payload = serde_json::to_vec(&manifest).unwrap();
    let registry =
        mc_core::registry::CoreRegistry::load(config.vanilla_data_dir.as_deref()).unwrap();
    store
        .commit_checkpoint(
            NativeCheckpoint {
                tick: checkpoint,
                previous_tick: checkpoint,
                chunks: &[],
                entities: &[],
                players: &[],
                globals: &image.globals,
            },
            &registry.blocks,
            &registry.items,
        )
        .unwrap();
    let identity = mc_core::journal::WalIdentity::new(
        store.store_uuid(),
        mc_world::store_marker::ROOT_BRANCH,
        &registry.blocks,
    );
    let store_uuid = store.store_uuid();
    drop(store);
    let mut writer = JournalWriter::open(&copy, 1, identity).unwrap();
    writer
        .append(
            checkpoint + 1,
            checkpoint + 1,
            &[JournalEntry::Climate {
                value: tail_climate,
            }],
        )
        .unwrap();
    writer.flush().unwrap();
    drop(writer);
    let batch = format!("batch-{:020}-{:020}.bin", checkpoint + 1, checkpoint + 1);
    let before = std::fs::read(copy.join("journal").join(&batch)).unwrap();
    LegacyCopy {
        copy,
        config,
        checkpoint,
        store_uuid,
        batch,
        before,
    }
}

#[test]
fn copy_command_checkpoints_legacy_tail_then_archives_identical_wal() {
    let legacy = legacy_copy();
    let (copy, checkpoint) = (&legacy.copy, legacy.checkpoint);
    let report = upgrade_game_clock(&legacy.operator_config(), copy).unwrap();
    assert_eq!(report.tick, checkpoint + 1);
    assert_eq!(report.checkpoint_tick, checkpoint);
    assert_eq!(report.tail_held_ticks, 1);
    assert_eq!(report.held_since, Some(checkpoint + 1));
    // The converted checkpoint is pinned in the marker, same store and branch,
    // before the WAL left `journal/`. Read before any open: `Core::open` would
    // itself advance a marker that lags the store.
    assert_eq!(
        store_marker::read(copy).unwrap(),
        Some(store_marker::StoreMarker::new(
            legacy.store_uuid,
            store_marker::ROOT_BRANCH,
            report.tick
        )),
        "the WAL was archived with the marker still at the old checkpoint"
    );
    assert_eq!(
        std::fs::read(
            copy.join(format!("journal-before-game-clock-{checkpoint}"))
                .join(&legacy.batch)
        )
        .unwrap(),
        legacy.before
    );
    // The archive takes the batches, not the directory: `journal/` stays, empty.
    assert_eq!(
        std::fs::read_dir(copy.join("journal")).unwrap().count(),
        0,
        "journal/ still holds files after the archive"
    );
    let recovered = mc_core::native_checkpoint::recover_offline(&legacy.config).unwrap();
    assert_eq!(recovered.checkpoint.tick, checkpoint + 1);
    drop(recovered);
    std::fs::remove_dir_all(copy).unwrap();
}

/// The marker write that pins the converted checkpoint fails (its temporary
/// file cannot be created): the upgrade reports the error and the WAL stays in
/// `journal/`, unarchived, so the tail still covers a store rolled back to the
/// old checkpoint.
#[test]
fn a_failed_marker_write_leaves_the_wal_in_the_journal() {
    let legacy = legacy_copy();
    let (copy, checkpoint) = (&legacy.copy, legacy.checkpoint);
    BEFORE_MARKER.with(|armed| {
        *armed.borrow_mut() = Some(Box::new(|copy: &Path| {
            // `store_marker::write` stages the marker in `<marker>.tmp`.
            std::fs::create_dir(copy.join(format!("{MARKER_FILE}.tmp"))).unwrap();
        }));
    });
    let error = upgrade_game_clock(&legacy.operator_config(), copy).unwrap_err();
    assert!(error.to_string().contains(MARKER_FILE), "{error}");
    assert_eq!(
        std::fs::read(copy.join("journal").join(&legacy.batch)).ok(),
        Some(legacy.before.clone()),
        "the WAL left journal/ although the marker was not written"
    );
    assert!(
        !copy
            .join(format!("journal-before-game-clock-{checkpoint}"))
            .exists()
    );
    assert_eq!(
        store_marker::read(copy)
            .unwrap()
            .map(|marker| marker.checkpoint_tick),
        Some(checkpoint)
    );
    std::fs::remove_dir_all(copy).unwrap();
}

#[test]
fn copy_command_refuses_the_original_and_its_canonical_aliases_without_writes() {
    let directory =
        std::env::temp_dir().join(format!("game-clock-original-{}", uuid::Uuid::new_v4()));
    let original = directory.join("original");
    std::fs::create_dir_all(&original).unwrap();
    let sentinel = original.join("keep");
    std::fs::write(&sentinel, b"immutable original").unwrap();
    let config = CoreConfig::new(original.clone(), 42);
    let aliases = vec![original.clone(), original.join(".")];
    #[cfg(unix)]
    let aliases = {
        let mut aliases = aliases;
        let alias = directory.join("symlink");
        std::os::unix::fs::symlink(&original, &alias).unwrap();
        aliases.push(alias);
        aliases
    };
    for alias in aliases {
        let error = upgrade_game_clock(&config, &alias).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("not the configured original world"),
            "{error}"
        );
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"immutable original");
        assert_eq!(std::fs::read_dir(&original).unwrap().count(), 1);
    }
    std::fs::remove_dir_all(directory).unwrap();
}
