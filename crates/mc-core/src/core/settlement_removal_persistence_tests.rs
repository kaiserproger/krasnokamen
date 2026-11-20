//! A settlement removed after it reached a checkpoint: the next checkpoint
//! through the journal thread's real writer must delete its `settlement/<id>`
//! row, or the following startup refuses the stale row.
use super::*;
use crate::boundary::{ConnectionId, PlayerInput};
use crate::dimensions::Dimension;
use crate::native_checkpoint::tests::{fixture_store, read_manifest};
use crate::tick::TickState;

/// One tick with `inputs`, its WAL batch, then a checkpoint of that tick
/// frozen and written exactly as the journal thread does.
fn tick_and_checkpoint(
    state: &mut TickState,
    writer: &mut JournalWriter,
    manifest: &mut CheckpointManifest,
    directory: &std::path::Path,
    inputs: &[PlayerInput],
) {
    let (output, _, _) = crate::tick::run_tick(state, inputs);
    let entries = journal_entries_for_tick(state, &output).unwrap();
    writer.append(output.tick, output.tick, &entries).unwrap();
    let (work_tx, work_rx) = journal_work_channel();
    let (done_tx, _done_rx) = std::sync::mpsc::channel();
    assert!(start_checkpoint(state, &work_tx, &done_tx, output.tick));
    let JournalWork::Checkpoint(job) = work_rx.recv().unwrap() else {
        panic!("expected a frozen checkpoint");
    };
    write_checkpoint(directory, &job, &state.plugins, writer, manifest).unwrap();
}

#[test]
fn removed_settlement_row_is_deleted_by_the_next_checkpoint_and_the_world_reopens() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = CoreConfig::new(directory.path().to_owned(), 42);
    config.checkpoint_every_ticks = u64::MAX;
    config.journal_fsync_every_batches = 1;
    config.view_distance = 1;
    config.simulation_distance = 0;
    Core::open(config.clone()).unwrap().shutdown();
    let mut manifest = read_manifest(&config).unwrap();
    let load = || {
        let mut state = recover_dimension_state(&config, Dimension::Overworld)?;
        for dimension in [Dimension::Nether, Dimension::End] {
            let world = recover_dimension_state(&config, dimension)?;
            state.other_dimensions.insert(dimension, Box::new(world));
        }
        Ok::<_, crate::config::CoreError>(state)
    };
    let mut state = load().unwrap();
    let mut writer = JournalWriter::open_test(directory.path(), 1).unwrap();
    let id = state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "town".into(),
            "founder".into(),
            [0, 64, 0],
            32,
        )
        .unwrap();
    let row = format!("settlement/{}", id.0);
    tick_and_checkpoint(
        &mut state,
        &mut writer,
        &mut manifest,
        directory.path(),
        &[],
    );
    let stored = fixture_store(&config).unwrap().read_checkpoint().unwrap();
    assert!(
        stored.globals.contains_key(&row),
        "the settlement reached the store"
    );

    // An operator removes it through the real command path; the next
    // checkpoint no longer carries the row.
    let connection = ConnectionId(7);
    let inputs = [
        PlayerInput::Join {
            connection,
            uuid: uuid::Uuid::new_v4(),
            name: "Operator".into(),
            permission_level: 4,
        },
        PlayerInput::Command {
            connection,
            command: "settlement remove town".into(),
            permission_level: 4,
        },
    ];
    tick_and_checkpoint(
        &mut state,
        &mut writer,
        &mut manifest,
        directory.path(),
        &inputs,
    );
    assert!(
        state.settlements.get(id).is_none(),
        "the command removed it"
    );
    writer.flush().unwrap();
    drop(writer);
    drop(state);

    // The world reopens without the settlement and without its row.
    let reopened = load().expect("the world reopens after the removal checkpoint");
    assert!(reopened.settlements.get(id).is_none());
    let stored = fixture_store(&config).unwrap().read_checkpoint().unwrap();
    assert!(
        !stored.globals.contains_key(&row),
        "the stale row was deleted"
    );
}
