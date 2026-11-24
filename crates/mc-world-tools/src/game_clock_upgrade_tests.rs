use super::*;
use mc_core::journal::{CheckpointManifest, JournalEntry, JournalWriter};
use mc_world::native::NativeCheckpoint;

#[test]
fn copy_command_checkpoints_legacy_tail_then_archives_identical_wal() {
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
    let mut operator_config = config.clone();
    operator_config.world_dir = copy.join("original");
    std::fs::create_dir(&operator_config.world_dir).unwrap();
    let report = upgrade_game_clock(&operator_config, &copy).unwrap();
    assert_eq!(report.tick, checkpoint + 1);
    assert_eq!(report.checkpoint_tick, checkpoint);
    assert_eq!(report.tail_held_ticks, 1);
    assert_eq!(report.held_since, Some(checkpoint + 1));
    assert_eq!(
        std::fs::read(
            copy.join(format!("journal-before-game-clock-{checkpoint}"))
                .join(batch)
        )
        .unwrap(),
        before
    );
    assert!(!copy.join("journal").exists());
    let recovered = mc_core::native_checkpoint::recover_offline(&config).unwrap();
    assert_eq!(recovered.checkpoint.tick, checkpoint + 1);
    drop(recovered);
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
