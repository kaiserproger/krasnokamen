use super::*;
use crate::boundary::{ConnectionId, PlayerChange, PlayerInput};
use crate::journal::read_complete_batches_after;
use crate::native_checkpoint::tests::read_manifest;
use crate::survival::Difficulty;

#[test]
fn difficulty_checkpoint_is_frozen_and_wal_restart_preserves_join_and_query() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = CoreConfig::new(directory.path().to_owned(), 42);
    config.checkpoint_every_ticks = u64::MAX;
    config.journal_fsync_every_batches = 1;
    config.view_distance = 1;
    config.simulation_distance = 0;
    let core = Core::open(config.clone()).unwrap();
    core.shutdown();
    let mut manifest = read_manifest(&config).unwrap();
    let load = |_: &CheckpointManifest| {
        recover_dimension_state(&config, crate::dimensions::Dimension::Overworld).unwrap()
    };
    let mut state = load(&manifest);
    for dimension in [
        crate::dimensions::Dimension::Nether,
        crate::dimensions::Dimension::End,
    ] {
        state.other_dimensions.insert(
            dimension,
            Box::new(recover_dimension_state(&config, dimension).unwrap()),
        );
    }
    state.difficulty = Difficulty::Hard;
    let (output, _, _) = crate::tick::run_tick(&mut state, &[]);
    let entries = journal_entries_for_tick(&mut state, &output);
    assert!(entries.iter().any(|entry| matches!(
        entry,
        JournalEntry::Difficulty {
            value: Difficulty::Hard
        }
    )));
    let tick = output.tick;
    let (work_tx, work_rx) = journal_work_channel();
    let (done_tx, _done_rx) = std::sync::mpsc::channel();
    assert!(start_checkpoint(&mut state, &work_tx, &done_tx, tick));
    state.difficulty = Difficulty::Easy;
    let JournalWork::Checkpoint(job) = work_rx.recv().unwrap() else {
        panic!("expected frozen checkpoint");
    };
    let mut writer = JournalWriter::open(directory.path(), 1).unwrap();
    writer.append(tick, tick, &entries).unwrap();
    write_checkpoint(
        directory.path(),
        &job,
        &state.plugins,
        &mut writer,
        &mut manifest,
    )
    .unwrap();
    assert_eq!(manifest.difficulty, Difficulty::Hard);
    prune_batches_through(&directory.path().join("journal"), tick);
    assert!(
        read_complete_batches_after(&directory.path().join("journal"), 0)
            .unwrap()
            .is_empty()
    );
    let mut restored = load(&manifest);
    assert_eq!(restored.difficulty, Difficulty::Hard);
    let connection = ConnectionId(17);
    let (output, drains, _) = crate::tick::run_tick(
        &mut restored,
        &[
            PlayerInput::Join {
                connection,
                uuid: uuid::Uuid::new_v4(),
                name: "DifficultyTester".into(),
                permission_level: 0,
            },
            PlayerInput::Command {
                connection,
                command: "difficulty normal".into(),
                permission_level: 4,
            },
        ],
    );
    assert!(
        drains
            .iter()
            .any(|(_, changes)| changes.iter().any(|change| matches!(
                change,
                PlayerChange::DifficultyChanged { difficulty: 3, .. }
            )))
    );
    assert_eq!(restored.difficulty, Difficulty::Normal);
    let entries = journal_entries_for_tick(&mut restored, &output);
    assert!(entries.iter().any(|entry| matches!(
        entry,
        JournalEntry::Difficulty {
            value: Difficulty::Normal
        }
    )));
    writer.append(output.tick, output.tick, &entries).unwrap();
    writer.flush().unwrap();
    let mut restarted = load(&manifest);
    assert_eq!(restarted.difficulty, Difficulty::Normal);
    let (_, drains, _) = crate::tick::run_tick(
        &mut restarted,
        &[PlayerInput::Join {
            connection,
            uuid: uuid::Uuid::new_v4(),
            name: "QueryTester".into(),
            permission_level: 0,
        }],
    );
    assert!(
        drains
            .iter()
            .any(|(_, changes)| changes.iter().any(|change| matches!(
                change,
                PlayerChange::DifficultyChanged { difficulty: 2, .. }
            )))
    );
    let (_, drains, _) = crate::tick::run_tick(
        &mut restarted,
        &[PlayerInput::Command {
            connection,
            command: "difficulty".into(),
            permission_level: 4,
        }],
    );
    assert!(
        drains
            .iter()
            .any(|(_, changes)| changes.iter().any(|change| {
                matches!(change, PlayerChange::StyledChat { content, .. }
            if format!("{content:?}").contains("options.difficulty.normal"))
            }))
    );
    let error = serde_json::from_str::<CheckpointManifest>(r#"{"tick":9}"#).unwrap_err();
    assert!(error.to_string().contains("domain_versions"), "{error}");
}

#[test]
fn worldborder_command_mid_lerp_checkpoint_restores_and_finishes() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = CoreConfig::new(directory.path().to_owned(), 42);
    config.checkpoint_every_ticks = u64::MAX;
    config.journal_fsync_every_batches = 1;
    config.view_distance = 1;
    config.simulation_distance = 0;
    let core = Core::open(config.clone()).unwrap();
    core.shutdown();
    let mut manifest = read_manifest(&config).unwrap();
    let load =
        || recover_dimension_state(&config, crate::dimensions::Dimension::Overworld).unwrap();
    let mut state = load();
    for dimension in [
        crate::dimensions::Dimension::Nether,
        crate::dimensions::Dimension::End,
    ] {
        state.other_dimensions.insert(
            dimension,
            Box::new(recover_dimension_state(&config, dimension).unwrap()),
        );
    }
    let connection = ConnectionId(17);
    crate::tick::run_tick(
        &mut state,
        &[PlayerInput::Join {
            connection,
            uuid: uuid::Uuid::new_v4(),
            name: "BorderTester".into(),
            permission_level: 4,
        }],
    );
    for command in ["worldborder set 100", "worldborder set 50 200t"] {
        crate::tick::run_tick(
            &mut state,
            &[PlayerInput::Command {
                connection,
                command: command.into(),
                permission_level: 4,
            }],
        );
    }
    for _ in 0..99 {
        crate::tick::run_tick(&mut state, &[]);
    }
    assert!(state.border.lerp_remaining > 0 && state.border.lerp_remaining < 200);
    assert!(state.border.size > 50.0 && state.border.size < 100.0);
    let (output, _, _) = crate::tick::run_tick(&mut state, &[]);
    let mid = state.border;
    let entries = journal_entries_for_tick(&mut state, &output);
    assert!(
        entries
            .iter()
            .any(|entry| matches!(entry, JournalEntry::Border { value } if *value == mid))
    );
    let (work_tx, work_rx) = journal_work_channel();
    let (done_tx, _done_rx) = std::sync::mpsc::channel();
    assert!(start_checkpoint(
        &mut state,
        &work_tx,
        &done_tx,
        output.tick
    ));
    let JournalWork::Checkpoint(job) = work_rx.recv().unwrap() else {
        panic!("expected checkpoint")
    };
    let mut writer = JournalWriter::open(directory.path(), 1).unwrap();
    writer
        .append(manifest.tick + 1, output.tick, &entries)
        .unwrap();
    write_checkpoint(
        directory.path(),
        &job,
        &state.plugins,
        &mut writer,
        &mut manifest,
    )
    .unwrap();
    let mut restored = load();
    assert_eq!(restored.border, mid);
    for _ in 0..mid.lerp_remaining {
        crate::tick::run_tick(&mut restored, &[]);
    }
    assert_eq!(
        (
            restored.border.size,
            restored.border.lerp_target,
            restored.border.lerp_remaining
        ),
        (50.0, 50.0, 0)
    );
}
