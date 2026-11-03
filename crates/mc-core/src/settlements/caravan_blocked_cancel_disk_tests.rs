//! Actual native checkpoint and published WAL recovery of blocked cancellation.
//! Uses the existing caravan fixture in a real native-store-backed world.
use super::*;
use crate::settlements::persistence;
use crate::{CoreConfig, dimensions::Dimension, journal::JournalWriter};

fn startup(state: &mut TickState) {
    crate::plugin_startup::apply(
        std::sync::Arc::make_mut(&mut state.registry),
        &mc_plugin_host::DeploymentContribution::default(),
        None,
        &[],
    )
    .unwrap();
}

fn disk_fixture(config: &CoreConfig) -> Fixture {
    let mut state = crate::core::recover_dimension_state(config, Dimension::Overworld).unwrap();
    startup(&mut state);
    let _ = crate::tick::run_tick(&mut state, &[]);
    let mut f = fixture_in(state);
    let slot = f.state.entities.slot_of(f.carrier).unwrap();
    f.state.entities.row_mut(slot).controls.no_ai = true;
    f
}

fn checkpoint(f: &mut Fixture, config: &CoreConfig) {
    use crate::native_checkpoint::{encode_entities, encode_players, record, refresh_metadata};
    use mc_world::native::NativeCheckpoint;
    let store = crate::native_checkpoint::tests::fixture_store(config).unwrap();
    let tick = f.state.tick;
    let (players, captured, block_entities) = crate::core::checkpoint_data_for(&f.state, tick);
    let entities = encode_entities(
        Dimension::Overworld,
        &captured,
        &f.state.registry.items,
        tick,
    )
    .unwrap();
    let players = encode_players(players.iter(), &f.state.registry.items, tick).unwrap();
    let capture = f.state.world.capture_dirty(&block_entities);
    let written = capture.written();
    let mut image = store.read_checkpoint().unwrap();
    let mut manifest: crate::journal::CheckpointManifest =
        serde_json::from_slice(&image.globals["checkpoint"].payload).unwrap();
    manifest.tick = tick;
    for child in manifest.dimensions.values_mut() {
        child.tick = tick;
    }
    image
        .globals
        .extend(persistence::checkpoint_rows(&f.state, tick).unwrap());
    image.globals.insert(
        "plugins".into(),
        record(tick, serde_json::to_vec(&f.state.plugins).unwrap()),
    );
    image.globals.insert(
        "checkpoint".into(),
        record(tick, serde_json::to_vec(&manifest).unwrap()),
    );
    refresh_metadata(&mut image.globals, &manifest).unwrap();
    for global in image.globals.values_mut() {
        global.captured_tick = tick;
    }
    store
        .commit_checkpoint_stream(
            NativeCheckpoint {
                tick,
                previous_tick: image.tick,
                chunks: &[],
                entities: &[],
                players: &players,
                globals: &image.globals,
            },
            |write| {
                capture
                    .visit_native(|snapshot| {
                        write(snapshot)
                            .map_err(|error| crate::config::CoreError::Storage(error.to_string()))
                    })
                    .map_err(|error| mc_world::native::NativeError::Source(error.to_string()))
            },
            entities.iter().map(Ok),
            &f.state.registry.blocks,
            &f.state.registry.items,
        )
        .unwrap();
    f.state.world.mark_flushed(&written);
}

fn step(f: &mut Fixture, writer: &mut JournalWriter) {
    let (output, _, _) = crate::tick::run_tick(&mut f.state, &[]);
    let entries = crate::core::journal_entries_for_tick(&mut f.state, &output);
    writer.append(output.tick, output.tick, &entries).unwrap();
}

fn reopen(f: Fixture, config: &CoreConfig, mut writer: JournalWriter) -> (Fixture, JournalWriter) {
    writer.flush().unwrap();
    drop(writer);
    let Fixture {
        state,
        settlement,
        source,
        destination,
        carrier,
        carrier_uuid,
        bread,
    } = f;
    drop(state);
    let mut state = crate::core::recover_dimension_state(config, Dimension::Overworld).unwrap();
    startup(&mut state);
    ensure(&mut state);
    assert_eq!(
        state.entities.slot_of_uuid(carrier_uuid),
        state.entities.slot_of(carrier)
    );
    let mut f = Fixture {
        state,
        settlement,
        source,
        destination,
        carrier,
        carrier_uuid,
        bread,
    };
    let mut writer = JournalWriter::open(&config.world_dir, 1).unwrap();
    // Blocked/cancelled shipments are not runnable. A newly admitted production
    // portion has its ordinary 200-tick deadline, so this one hydration tick
    // cannot manufacture output or move the cancelled cargo.
    step(&mut f, &mut writer);
    (f, writer)
}

#[test]
fn blocked_loaded_cancel_releases_ownership_across_actual_wal_checkpoint_and_new_job_admission() {
    for cause in [Cause::StaleDestination, Cause::RouteBudget] {
        for save_checkpoint in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let config = CoreConfig::new(directory.path().to_path_buf(), 42);
            let mut f = disk_fixture(&config);
            begin(&mut f, cause);
            checkpoint(&mut f, &config);
            let mut writer = JournalWriter::open(directory.path(), 1).unwrap();
            step(&mut f, &mut writer); // Actual source pickup; no synthetic loaded flag.
            after_pickup(&mut f, cause);
            for _ in 0..8 {
                step(&mut f, &mut writer);
                if f.state
                    .settlements
                    .get(f.settlement)
                    .unwrap()
                    .economy
                    .caravan
                    .shipments["blocked-custody"]
                    .status
                    == Status::Blocked
                {
                    break;
                }
            }
            let blocked = f
                .state
                .settlements
                .get(f.settlement)
                .unwrap()
                .economy
                .caravan
                .shipments["blocked-custody"]
                .clone();
            assert_eq!(blocked.status, Status::Blocked);
            assert!(blocked.loaded && blocked.load.is_some());
            assert!(blocked.delivery.is_none() && blocked.unload.is_none());
            assert!(economy::caravan_reserved(
                &f.state,
                Some(f.settlement),
                f.carrier_uuid
            ));
            let held = physical(&f);
            writer.flush().unwrap();
            if save_checkpoint {
                checkpoint(&mut f, &config);
            }
            let (mut f, mut writer) = reopen(f, &config, writer);
            assert_eq!(physical(&f), held);
            assert_eq!(
                f.state
                    .settlements
                    .get(f.settlement)
                    .unwrap()
                    .economy
                    .caravan
                    .shipments["blocked-custody"],
                blocked
            );
            assert!(
                economy::caravan_reserved(&f.state, Some(f.settlement), f.carrier_uuid),
                "blocked custody must survive recovery before cancellation"
            );
            native(
                &mut f,
                Request::Cancel {
                    id: "blocked-custody".into(),
                },
            );
            assert_eq!(physical(&f), held, "cancellation changes policy only");
            let mut cancelled = blocked;
            cancelled.status = Status::Cancelled;
            step(&mut f, &mut writer); // Publish cancellation alongside unchanged physical images.
            writer.flush().unwrap();
            if save_checkpoint {
                checkpoint(&mut f, &config);
            }
            let (mut f, mut writer) = reopen(f, &config, writer);
            assert_eq!(physical(&f), held);
            assert_eq!(
                f.state
                    .settlements
                    .get(f.settlement)
                    .unwrap()
                    .economy
                    .caravan
                    .shipments["blocked-custody"],
                cancelled
            );
            assert!(!economy::caravan_reserved(
                &f.state,
                Some(f.settlement),
                f.carrier_uuid
            ));
            assert!(!delivered(
                &f.state
                    .settlements
                    .get(f.settlement)
                    .unwrap()
                    .economy
                    .caravan,
                "blocked-custody"
            ));
            validate(
                &f.state
                    .settlements
                    .get(f.settlement)
                    .unwrap()
                    .economy
                    .caravan,
            )
            .unwrap();
            native(
                &mut f,
                Request::Cancel {
                    id: "blocked-custody".into(),
                },
            );
            production_after_cancel(&mut f); // Actual production admission after recovery, same UUID.
            assert_eq!(physical(&f), held);
            assert!(economy::production_reserved(
                &f.state,
                Some(f.settlement),
                f.carrier_uuid
            ));
            assert!(!economy::caravan_reserved(
                &f.state,
                Some(f.settlement),
                f.carrier_uuid
            ));
            step(&mut f, &mut writer);
            let (f, writer) = reopen(f, &config, writer);
            assert_eq!(physical(&f), held);
            assert_eq!(
                f.state
                    .settlements
                    .get(f.settlement)
                    .unwrap()
                    .economy
                    .caravan
                    .shipments["blocked-custody"],
                cancelled
            );
            assert_eq!(
                f.state
                    .settlements
                    .get(f.settlement)
                    .unwrap()
                    .economy
                    .production
                    .jobs["after-blocked-cancel"]
                    .done,
                0
            );
            assert!(economy::production_reserved(
                &f.state,
                Some(f.settlement),
                f.carrier_uuid
            ));
            assert!(!economy::caravan_reserved(
                &f.state,
                Some(f.settlement),
                f.carrier_uuid
            ));
            drop(writer);
        }
    }
}
