use super::*;
use crate::boundary::{ConnectionId, PlayerInput};
use crate::commands::GameMode;
use crate::native_checkpoint::tests::read_manifest;

fn join(connection: ConnectionId, name: &str) -> PlayerInput {
    PlayerInput::Join {
        connection,
        uuid: uuid::Uuid::new_v4(),
        name: name.into(),
        permission_level: 0,
    }
}

fn game_mode_of(state: &crate::tick::TickState, connection: ConnectionId) -> GameMode {
    let entity = state.connections.entity_of(connection).expect("joined");
    state.players[&entity].game_mode
}

/// Vanilla `WorldData.GameType`: `/defaultgamemode` survives a crash (journal) and a checkpoint (manifest).
#[test]
fn default_game_mode_survives_wal_restart_and_checkpoint_and_joins_new_players() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = CoreConfig::new(directory.path().to_owned(), 42);
    config.checkpoint_every_ticks = u64::MAX;
    config.journal_fsync_every_batches = 1;
    config.view_distance = 1;
    config.simulation_distance = 0;
    let core = Core::open(config.clone()).unwrap();
    core.shutdown();
    let mut manifest = read_manifest(&config).unwrap();
    assert_eq!(manifest.default_game_mode, GameMode::Survival);
    let load =
        || recover_dimension_state(&config, crate::dimensions::Dimension::Overworld).unwrap();
    let mut state = load();
    let operator = ConnectionId(3);
    let (output, _, _) = crate::tick::run_tick(
        &mut state,
        &[
            join(operator, "Operator"),
            PlayerInput::Command {
                connection: operator,
                command: "defaultgamemode creative".into(),
                permission_level: 4,
            },
        ],
    );
    assert_eq!(state.default_game_mode, GameMode::Creative);
    let entries = journal_entries_for_tick(&mut state, &output);
    assert!(entries.iter().any(|entry| matches!(
        entry,
        JournalEntry::DefaultGameMode {
            value: GameMode::Creative
        }
    )));
    let mut writer = JournalWriter::open(directory.path(), 1).unwrap();
    writer.append(output.tick, output.tick, &entries).unwrap();
    writer.flush().unwrap();

    // Crash after the command, before any checkpoint: the WAL alone restores it.
    let mut restarted = load();
    assert_eq!(restarted.default_game_mode, GameMode::Creative);
    let newcomer = ConnectionId(4);
    crate::tick::run_tick(&mut restarted, &[join(newcomer, "Newcomer")]);
    assert_eq!(game_mode_of(&restarted, newcomer), GameMode::Creative);

    // A checkpoint freezes the value in the manifest; the later in-memory change is not captured.
    let tick = output.tick;
    let (work_tx, work_rx) = journal_work_channel();
    let (done_tx, _done_rx) = std::sync::mpsc::channel();
    assert!(start_checkpoint(&mut state, &work_tx, &done_tx, tick));
    state.default_game_mode = GameMode::Adventure;
    let JournalWork::Checkpoint(job) = work_rx.recv().unwrap() else {
        panic!("expected frozen checkpoint");
    };
    write_checkpoint(
        directory.path(),
        &job,
        &state.plugins,
        &mut writer,
        &mut manifest,
    )
    .unwrap();
    assert_eq!(manifest.default_game_mode, GameMode::Creative);
    prune_batches_through(&directory.path().join("journal"), tick);
    assert_eq!(load().default_game_mode, GameMode::Creative);
}

#[test]
fn a_bad_default_game_mode_is_refused_not_defaulted() {
    let error = serde_json::from_str::<JournalEntry>(r#"{"DefaultGameMode":{"value":"flying"}}"#)
        .unwrap_err();
    assert!(error.to_string().contains("flying"), "{error}");
    let error = serde_json::from_str::<CheckpointManifest>(r#"{"default_game_mode":"flying"}"#)
        .unwrap_err();
    assert!(error.to_string().contains("flying"), "{error}");
}
