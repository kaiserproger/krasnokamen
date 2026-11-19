//! Checkpoint and published/unpublished tick-WAL cuts on real native terrain.
use super::*;
use crate::settlements::persistence;

fn disk_fixture(config: &CoreConfig) -> Fixture {
    let mut world = crate::core::recover_dimension_state(config, Dimension::Overworld).unwrap();
    startup(&mut world);
    let _ = crate::tick::run_tick(&mut world, &[]);
    fixture(world)
}

fn startup(world: &mut TickState) {
    crate::plugin_startup::apply(
        Arc::make_mut(&mut world.registry),
        &mc_plugin_host::DeploymentContribution::default(),
        None,
        &[],
    )
    .unwrap();
}

fn checkpoint(f: &mut Fixture, config: &CoreConfig, legacy: bool) {
    use crate::native_checkpoint::{encode_entities, encode_players, record, refresh_metadata};
    use mc_world::native::NativeCheckpoint;
    let store = crate::native_checkpoint::tests::fixture_store(config).unwrap();
    let tick = f.world.tick;
    let (players, captured, block_entities) = crate::core::checkpoint_data_for(&f.world, tick);
    let entities = encode_entities(
        Dimension::Overworld,
        &captured,
        &f.world.registry.items,
        tick,
    )
    .unwrap();
    let players = encode_players(players.iter(), &f.world.registry.items, tick).unwrap();
    let capture = f.world.world.capture_dirty(&block_entities);
    let written = capture.written();
    let mut image = store.read_checkpoint().unwrap();
    let mut manifest: crate::journal::CheckpointManifest =
        serde_json::from_slice(&image.globals["checkpoint"].payload).unwrap();
    manifest.tick = tick;
    manifest.climate = Some(crate::weather::ClimateImage::capture(&f.world));
    for child in manifest.dimensions.values_mut() {
        child.tick = tick;
    }
    image
        .globals
        .extend(persistence::checkpoint_rows(&f.world, tick).unwrap());
    if legacy {
        let row = image
            .globals
            .get_mut(&persistence::row_key(f.town))
            .unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(&row.payload).unwrap();
        let production = value["value"]["economy"]["production"]
            .as_object_mut()
            .unwrap();
        production.remove("clock");
        production.remove("consumption");
        for job in production["jobs"].as_object_mut().unwrap().values_mut() {
            job.as_object_mut().unwrap().remove("deferred");
        }
        row.payload = serde_json::to_vec(&value).unwrap();
    }
    image.globals.insert(
        "plugins".into(),
        record(tick, serde_json::to_vec(&f.world.plugins).unwrap()),
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
                        write(snapshot).map_err(|e| crate::CoreError::Storage(e.to_string()))
                    })
                    .map_err(|e| mc_world::native::NativeError::Source(e.to_string()))
            },
            entities.iter().map(Ok),
            &f.world.registry.blocks,
            &f.world.registry.items,
        )
        .unwrap();
    f.world.world.mark_flushed(&written);
}

fn step(f: &mut Fixture, writer: Option<&mut JournalWriter>) {
    let (output, _, _) = crate::tick::run_tick(&mut f.world, &[]);
    let entries = crate::core::journal_entries_for_tick(&mut f.world, &output).unwrap();
    if let Some(writer) = writer {
        writer.append(output.tick, output.tick, &entries).unwrap();
    }
}

fn step_to(f: &mut Fixture, writer: &mut JournalWriter, now: u64) {
    while f.world.tick <= now {
        step(f, Some(writer));
    }
}

fn reopen(f: Fixture, config: &CoreConfig, writer: Option<JournalWriter>, active: bool) -> Fixture {
    if let Some(mut writer) = writer {
        writer.flush().unwrap();
    }
    let Fixture {
        world,
        town,
        worker,
        chest,
    } = f;
    drop(world);
    let mut world = crate::core::recover_dimension_state(config, Dimension::Overworld).unwrap();
    startup(&mut world);
    ensure(&mut world);
    crate::tick::sync_chunk_block_entities(&mut world);
    if !active {
        world.tickets.clear();
        world.refresh_active_chunks(world.tick);
    }
    assert!(world.entities.slot_of_uuid(worker).is_some());
    Fixture {
        world,
        town,
        worker,
        chest,
    }
}

#[test]
fn offscreen_double_count_checkpoint_reopen_retry_and_downtime() {
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_path_buf(), 42);
    let mut f = disk_fixture(&config);
    let first = job(&f).next_due;
    let period = job(&f).duration;
    let body = f
        .world
        .entities
        .body(f.world.entities.slot_of_uuid(f.worker).unwrap())
        .id;
    give(
        vec![item(&f.world, "minecraft:wheat", 3)],
        &mut f.world,
        body,
    );
    checkpoint(&mut f, &config, false);
    let mut writer =
        JournalWriter::open(directory.path(), 1, crate::journal::WalIdentity::fixture()).unwrap();
    step_to(&mut f, &mut writer, first);
    assert_eq!(job(&f).done, 1);
    // Delivery retry must not change the period watermark or manufacture work.
    at(&mut f.world, body, [20.5, 201.0, 8.5]);
    step_to(&mut f, &mut writer, first + 20);
    assert!(!job(&f).delivery_pending());
    assert_eq!(job(&f).next_due, first + period);
    f.world.tickets.clear();
    step_to(&mut f, &mut writer, first + 2 * period);
    assert_eq!((job(&f).done, job(&f).deferred), (1, 2));
    checkpoint(&mut f, &config, false);
    let saved_tick = f.world.tick;
    let saved_due = job(&f).next_due;
    let mut f = reopen(f, &config, Some(writer), true);
    assert_eq!(f.world.tick, saved_tick, "downtime adds no game time");
    assert_eq!(job(&f).next_due, saved_due);
    let mut writer =
        JournalWriter::open(directory.path(), 1, crate::journal::WalIdentity::fixture()).unwrap();
    step(&mut f, Some(&mut writer));
    assert_eq!((job(&f).done, job(&f).deferred), (3, 0));
    assert_eq!(total(&f, "minecraft:bread"), 3);
    let mut f = reopen(f, &config, Some(writer), true);
    let mut writer =
        JournalWriter::open(directory.path(), 1, crate::journal::WalIdentity::fixture()).unwrap();
    step(&mut f, Some(&mut writer));
    assert_eq!(job(&f).done, 3, "restart retry cannot add a portion");
    assert_eq!(total(&f, "minecraft:bread"), 3);
}

#[test]
fn offscreen_wal_commit_progress_apply_cuts() {
    for save_checkpoint in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let config = CoreConfig::new(directory.path().to_path_buf(), 42);
        let mut f = disk_fixture(&config);
        let due = job(&f).next_due;
        let period = job(&f).duration;
        checkpoint(&mut f, &config, false);
        f.world.tickets.clear();
        f.world.tick = due;
        step(&mut f, None); // in-progress accrual, no published WAL
        assert_eq!(job(&f).deferred, 1);
        let mut f = reopen(f, &config, None, false);
        assert_eq!(job(&f).deferred, 0);
        let mut writer =
            JournalWriter::open(directory.path(), 1, crate::journal::WalIdentity::fixture())
                .unwrap();
        step_to(&mut f, &mut writer, due + period);
        assert_eq!(job(&f).deferred, 2);
        if save_checkpoint {
            checkpoint(&mut f, &config, false);
        }
        let mut f = reopen(f, &config, Some(writer), true);
        assert_eq!((job(&f).done, job(&f).deferred), (0, 2));
        step(&mut f, None); // physical apply happened, but not committed
        assert_eq!(total(&f, "minecraft:bread"), 2);
        let mut f = reopen(f, &config, None, true);
        assert_eq!(total(&f, "minecraft:bread"), 0);
        assert_eq!((job(&f).done, job(&f).deferred), (0, 2));
        let mut writer =
            JournalWriter::open(directory.path(), 1, crate::journal::WalIdentity::fixture())
                .unwrap();
        step(&mut f, Some(&mut writer));
        if save_checkpoint {
            checkpoint(&mut f, &config, false);
        }
        let mut f = reopen(f, &config, Some(writer), true);
        assert_eq!((job(&f).done, job(&f).deferred), (2, 0));
        assert_eq!(total(&f, "minecraft:bread"), 2);
        let mut writer =
            JournalWriter::open(directory.path(), 1, crate::journal::WalIdentity::fixture())
                .unwrap();
        step(&mut f, Some(&mut writer));
        assert_eq!(total(&f, "minecraft:bread"), 2);
    }
}

#[test]
fn offscreen_legacy_overdue_conversion_preserves_one_opportunity_only() {
    for save_checkpoint in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let config = CoreConfig::new(directory.path().to_path_buf(), 42);
        let mut f = disk_fixture(&config);
        let body = f
            .world
            .entities
            .body(f.world.entities.slot_of_uuid(f.worker).unwrap())
            .id;
        give(
            vec![item(&f.world, "minecraft:wheat", 30)],
            &mut f.world,
            body,
        );
        f.world.tick = job(&f).next_due + 10 * job(&f).duration;
        checkpoint(&mut f, &config, true);
        let mut f = reopen(f, &config, None, false);
        assert_eq!(
            job(&f).next_due,
            f.world.tick_control.game_ticks(f.world.tick)
        );
        assert_eq!(job(&f).deferred, 0);
        // Losing the first conversion batch must deterministically reconvert.
        step(&mut f, None);
        let mut f = reopen(f, &config, None, false);
        assert_eq!(job(&f).deferred, 0);
        let mut writer =
            JournalWriter::open(directory.path(), 1, crate::journal::WalIdentity::fixture())
                .unwrap();
        step(&mut f, Some(&mut writer));
        assert_eq!(
            job(&f).deferred,
            1,
            "old code never caught up ten missed periods"
        );
        if save_checkpoint {
            checkpoint(&mut f, &config, false);
        }
        let mut f = reopen(f, &config, Some(writer), true);
        let mut writer =
            JournalWriter::open(directory.path(), 1, crate::journal::WalIdentity::fixture())
                .unwrap();
        step(&mut f, Some(&mut writer));
        assert_eq!((job(&f).done, job(&f).deferred), (1, 0));
        assert_eq!(total(&f, "minecraft:bread"), 1);
    }
}

#[test]
fn offscreen_consumption_debit_credit_and_inventory_share_wal_boundary() {
    for save_checkpoint in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let config = CoreConfig::new(directory.path().to_path_buf(), 42);
        let mut f = disk_fixture(&config);
        f.world
            .settlements
            .get_mut(f.town)
            .unwrap()
            .economy
            .production
            .jobs
            .clear();
        let mut slots = vec![ItemStack::EMPTY; 27];
        slots[0] = item(&f.world, "minecraft:bread", 10);
        fill(&mut f, slots);
        step(&mut f, None);
        // Begin immediately before a daily boundary; the daily-period arithmetic
        // itself is covered by the four-day online/offline native-state test.
        let due = f.world.tick + 1;
        f.world
            .settlements
            .get_mut(f.town)
            .unwrap()
            .economy
            .production
            .consumption
            .next_due = Some(due);
        checkpoint(&mut f, &config, false);
        f.world.tickets.clear();
        f.world.tick = due;
        step(&mut f, None);
        assert_eq!(consumption(&f).debit, 1);
        let mut f = reopen(f, &config, None, false);
        assert_eq!(consumption(&f).debit, 0);
        let mut writer =
            JournalWriter::open(directory.path(), 1, crate::journal::WalIdentity::fixture())
                .unwrap();
        step_to(&mut f, &mut writer, due);
        if save_checkpoint {
            checkpoint(&mut f, &config, false);
        }
        let mut f = reopen(f, &config, Some(writer), true);
        assert_eq!(consumption(&f).debit, 1);
        step(&mut f, None);
        assert_eq!((consumption(&f).debit, consumption(&f).credit), (0, 3));
        assert_eq!(total(&f, "minecraft:bread"), 9);
        let mut f = reopen(f, &config, None, true);
        assert_eq!((consumption(&f).debit, consumption(&f).credit), (1, 0));
        assert_eq!(total(&f, "minecraft:bread"), 10);
        let mut writer =
            JournalWriter::open(directory.path(), 1, crate::journal::WalIdentity::fixture())
                .unwrap();
        step(&mut f, Some(&mut writer));
        if save_checkpoint {
            checkpoint(&mut f, &config, false);
        }
        let mut f = reopen(f, &config, Some(writer), true);
        assert_eq!((consumption(&f).debit, consumption(&f).credit), (0, 3));
        assert_eq!(total(&f, "minecraft:bread"), 9);
        let mut writer =
            JournalWriter::open(directory.path(), 1, crate::journal::WalIdentity::fixture())
                .unwrap();
        step(&mut f, Some(&mut writer));
        assert_eq!(
            consumption(&f).credit,
            3,
            "replay cannot spend credit twice"
        );
        assert_eq!(total(&f, "minecraft:bread"), 9);
    }
}

fn consumption(f: &Fixture) -> &offscreen::Consumption {
    &f.world
        .settlements
        .get(f.town)
        .unwrap()
        .economy
        .production
        .consumption
}

#[test]
fn fresh_consumption_near_two_day_bound_survives_native_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_path_buf(), 42);
    let mut f = disk_fixture(&config);
    let id = town(&mut f.world, "late-phase", [8, 201, 8], 32);
    assert_eq!(id.0, 2);
    f.world.tick = 3;
    crate::settlements::economy::tick(&mut f.world);
    let due = f
        .world
        .settlements
        .get(id)
        .unwrap()
        .economy
        .production
        .consumption
        .next_due;
    assert_eq!(due, Some(48_002));
    checkpoint(&mut f, &config, false);
    let f = reopen(f, &config, None, true);
    assert_eq!(
        f.world
            .settlements
            .get(id)
            .unwrap()
            .economy
            .production
            .consumption
            .next_due,
        due
    );
}

#[test]
fn paused_retry_zero_after_reopen_cannot_authorize_a_portion() {
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_path_buf(), 42);
    let mut f = disk_fixture(&config);
    let body = f
        .world
        .entities
        .body(f.world.entities.slot_of_uuid(f.worker).unwrap())
        .id;
    give(
        vec![item(&f.world, "minecraft:wheat", 3)],
        &mut f.world,
        body,
    );
    let due = job(&f).next_due;
    park(
        f.world
            .settlements
            .get_mut(f.town)
            .unwrap()
            .economy
            .production
            .jobs
            .get_mut("supply")
            .unwrap(),
        "missing input",
        due,
    );
    checkpoint(&mut f, &config, false);
    let mut f = reopen(f, &config, None, true);
    assert_eq!(job(&f).retry_at, 0);
    let mut writer =
        JournalWriter::open(directory.path(), 1, crate::journal::WalIdentity::fixture()).unwrap();
    step(&mut f, Some(&mut writer));
    assert_eq!(job(&f).done, 0);
    assert_eq!(job(&f).next_due, due);
    assert_eq!(total(&f, "minecraft:bread"), 0);
}

#[test]
fn simulated_wait_then_reopen_unload_keeps_only_one_waited_opportunity() {
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_path_buf(), 42);
    let mut f = disk_fixture(&config);
    fill(&mut f, vec![ItemStack::EMPTY; 27]);
    let due = job(&f).next_due;
    let period = job(&f).duration;
    checkpoint(&mut f, &config, false);
    let mut writer =
        JournalWriter::open(directory.path(), 1, crate::journal::WalIdentity::fixture()).unwrap();
    step_to(&mut f, &mut writer, due + 4 * period);
    assert_eq!(job(&f).next_due, due);
    assert!(job(&f).waiting_simulated);
    checkpoint(&mut f, &config, false);
    let mut f = reopen(f, &config, Some(writer), false);
    assert!(
        job(&f).waiting_simulated,
        "the boundary guard survives restart"
    );
    assert_eq!(job(&f).retry_at, 0, "retry time is not persisted authority");
    let mut writer =
        JournalWriter::open(directory.path(), 1, crate::journal::WalIdentity::fixture()).unwrap();
    step(&mut f, Some(&mut writer));
    assert_eq!(
        job(&f).deferred,
        1,
        "simulated waiting never becomes five offscreen portions"
    );
    let next = job(&f).next_due;
    step_to(&mut f, &mut writer, next + RETRY_TICKS - 1);
    assert_eq!(job(&f).deferred, 2, "normal offscreen accrual resumes");
    ensure(&mut f.world);
    let mut slots = vec![ItemStack::EMPTY; 27];
    slots[0] = item(&f.world, "minecraft:wheat", 60);
    fill(&mut f, slots);
    let retry = job(&f).retry_at;
    step_to(&mut f, &mut writer, retry);
    assert_eq!((job(&f).done, job(&f).deferred), (2, 0));
    assert_eq!(total(&f, "minecraft:bread"), 2);
}
