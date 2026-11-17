use super::*;
use std::io::Write;

#[derive(Clone, Default)]
struct Log(std::sync::Arc<parking_lot::Mutex<Vec<u8>>>);

impl Write for Log {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn capture(action: impl FnOnce()) -> String {
    let log = Log::default();
    let writer = log.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, action);
    String::from_utf8(log.0.lock().clone()).unwrap()
}

fn writer_store(root: &std::path::Path) -> Arc<mc_world::native::NativeStore> {
    Arc::new(mc_world::native::NativeStore::create(&root.join("state.redb"), [0; 32]).unwrap())
}

#[test]
fn native_writer_channel_close_logs_stop_cause() {
    let root = tempfile::tempdir().unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    drop(sender);
    let log = capture(|| {
        native_writer_loop(
            root.path(),
            writer_store(root.path()),
            receiver,
            &AtomicU64::new(0),
            &std::sync::atomic::AtomicBool::new(false),
            CheckpointManifest::default(),
        );
    });
    assert!(
        log.contains("ERROR"),
        "missing error-level stop record: {log}"
    );
    assert!(log.contains("native checkpoint writer stopped"), "{log}");
    assert!(log.contains("journal work channel closed"), "{log}");
}

#[test]
fn native_writer_explicit_shutdown_logs_info_without_waiting_for_channel_close() {
    let root = tempfile::tempdir().unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    sender.send(NativeWork::Shutdown).unwrap();
    let log = capture(|| {
        native_writer_loop(
            root.path(),
            writer_store(root.path()),
            receiver,
            &AtomicU64::new(0),
            &std::sync::atomic::AtomicBool::new(false),
            CheckpointManifest::default(),
        );
    });
    assert!(log.contains("INFO"), "{log}");
    assert!(log.contains("native checkpoint writer stopped"), "{log}");
    assert!(log.contains("shutdown requested"), "{log}");
    assert!(!log.contains("ERROR"), "{log}");
    drop(sender);
}

#[test]
fn journal_io_failure_still_logs_error_and_never_acknowledges() {
    let root = tempfile::tempdir().unwrap();
    let writer = JournalWriter::open(root.path(), 1).unwrap();
    let journal = root.path().join("journal");
    std::fs::remove_dir(&journal).unwrap();
    std::fs::write(&journal, b"injected non-directory").unwrap();
    let (sender, receiver) = journal_work_channel();
    let (ack, acknowledged) = std::sync::mpsc::channel();
    sender
        .send(JournalWork::Append {
            tick_lo: 1,
            tick_hi: 1,
            entries: Vec::new(),
            plugin_ack: Some(ack),
            reserved: 0,
        })
        .unwrap();
    drop(sender);
    let log = capture(|| {
        journal_loop(
            root.path(),
            writer_store(root.path()),
            receiver,
            writer,
            &JournalBudget::new(crate::journal_budget::JOURNAL_QUEUE_BYTES),
            CheckpointManifest::default(),
            crate::plugin::PluginState::default(),
        )
    });
    assert!(
        log.contains("ERROR") && log.contains("journal append failed"),
        "{log}"
    );
    assert!(log.contains("world halting (fail closed)"), "{log}");
    assert!(acknowledged.recv().is_err());
    assert_eq!(std::fs::read(journal).unwrap(), b"injected non-directory");
}

#[test]
fn graceful_shutdown_state_distinguishes_expected_broadcast_close() {
    let root = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(root.path().to_owned(), 42);
    let core = Core::open(config.clone()).unwrap();
    let mut output = core.subscribe();
    assert!(!core.shutdown_requested());
    core.shutdown();
    assert!(core.shutdown_requested());
    while let Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) =
        output.blocking_recv()
    {}
    let restored = Core::open(config).expect("orderly shutdown retains a readable checkpoint");
    assert!(!restored.shutdown_requested());
    restored.shutdown();
}

#[test]
fn shutdown_releases_database_while_backup_admission_is_paused() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(root.path().to_owned(), 42);
    let core = Core::open(config.clone()).unwrap();
    std::thread::scope(|scope| {
        let (entered, admission) = std::sync::mpsc::sync_channel(0);
        let (resume, paused) = std::sync::mpsc::sync_channel(0);
        *core.inner.backup_admission_pause.lock().unwrap() = Some((entered, paused));
        let backup = scope.spawn(|| core.backup(output.path()));
        admission
            .recv_timeout(std::time::Duration::from_secs(30))
            .unwrap();
        // The caller has constructed its job, but has not sent it. Shutdown
        // must still finish every database owner's Drop before releasing the
        // world lease: reopening must work while that caller remains paused.
        core.shutdown();
        let reopened = Core::open(config);
        resume.send(()).unwrap();
        assert!(
            backup.join().unwrap().is_err(),
            "late admission must be refused"
        );
        let reopened = reopened.expect("shutdown released both world and database leases");
        assert_eq!(std::fs::read_dir(output.path()).unwrap().count(), 0);
        reopened.shutdown();
    });
}

#[test]
fn shutdown_drains_queued_ticks_and_restores_two_players_and_block() {
    use crate::boundary::{BlockPos, ChunkPos, ConnectionId, PlayerChange, PlayerInput};
    use std::sync::{Arc, mpsc};

    // The write rendezvous holds a process-global test hook. Keep it out of
    // the parent test process so unrelated parallel journals cannot block on it.
    const CHILD: &str = "KRASNOKAMEN_SHUTDOWN_QUEUE_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let full_name = concat!(
            module_path!(),
            "::shutdown_drains_queued_ticks_and_restores_two_players_and_block"
        );
        let test_name = full_name.split_once("::").unwrap().1;
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test_name, "--nocapture"])
            .env(CHILD, "1")
            .output()
            .expect("run isolated shutdown lifecycle");
        assert!(
            child.status.success(),
            "shutdown child failed: stdout={} stderr={}",
            String::from_utf8_lossy(&child.stdout),
            String::from_utf8_lossy(&child.stderr)
        );
        eprint!("{}", String::from_utf8_lossy(&child.stderr));
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let mut config = CoreConfig::new(root.path().to_owned(), 42);
    config.view_distance = 1;
    config.simulation_distance = 1;
    config.checkpoint_every_ticks = 10_000;
    let core = Arc::new(Core::open(config.clone()).unwrap());
    let mut output = core.subscribe();
    let dirt = core
        .registry()
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:dirt").unwrap())
        .unwrap();
    let stone = core.registry().states.stone;
    let [x, y, z] = core.world_spawn();
    let position = BlockPos { x, y: y + 5, z };
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let first = std::sync::atomic::AtomicBool::new(true);
    *crate::journal::WAL_WRITE_OBSERVER.lock() = Some((
        root.path().join("journal"),
        Box::new(move |_, _| {
            if first.swap(false, std::sync::atomic::Ordering::Relaxed) {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            }
        }),
    ));
    let players = [uuid::Uuid::new_v4(), uuid::Uuid::new_v4()];
    for (index, uuid) in players.into_iter().enumerate() {
        core.submit(PlayerInput::Join {
            connection: ConnectionId(index as u64 + 1),
            uuid,
            name: format!("shutdown-{index}"),
            permission_level: 0,
        });
    }
    entered_rx
        .recv_timeout(std::time::Duration::from_secs(30))
        .unwrap();
    let mut last_tick = 0;
    let mut joined = false;
    while last_tick < 20 || !joined {
        let tick = output.blocking_recv().unwrap();
        joined = players.iter().all(|uuid| {
            tick.snapshot
                .entities
                .iter()
                .any(|entity| entity.uuid == *uuid)
        });
        last_tick = tick.tick;
    }
    for connection in 1..=2 {
        core.submit(PlayerInput::Command {
            connection: ConnectionId(connection),
            command: format!("debug give minecraft:dirt {} 0", connection + 2),
            permission_level: 4,
        });
    }
    core.submit(PlayerInput::Command {
        connection: ConnectionId(1),
        command: format!(
            "setblock {} {} {} minecraft:stone",
            position.x, position.y, position.z
        ),
        permission_level: 4,
    });
    let mut inventories = [false; 2];
    let mut block_changed = false;
    for _ in 0..80 {
        let tick = output.blocking_recv().unwrap();
        block_changed |= tick
            .changes
            .blocks
            .iter()
            .any(|change| change.position == position && change.state == stone);
        for (connection, changes) in core.try_drain_outbox() {
            for change in changes {
                let has_stack =
                    |item: &mc_data::ItemStack, count| item.item_id == dirt && item.count == count;
                let count = match connection.0 {
                    1 => 3,
                    2 => 4,
                    _ => continue,
                };
                if match change {
                    PlayerChange::MenuSlot {
                        item: Some(item), ..
                    } => has_stack(&item, count),
                    PlayerChange::MenuContents { slots, .. } => {
                        slots.iter().flatten().any(|item| has_stack(item, count))
                    }
                    _ => false,
                } {
                    inventories[connection.0 as usize - 1] = true;
                }
            }
        }
        if block_changed && inventories.iter().all(|found| *found) {
            break;
        }
    }
    assert!(
        block_changed && inventories.iter().all(|found| *found),
        "native mutations were not observed"
    );
    let stopped_core = Arc::clone(&core);
    let shutdown = std::thread::spawn(move || stopped_core.shutdown());
    while let Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) =
        output.blocking_recv()
    {}
    let started = std::time::Instant::now();
    release_tx.send(()).unwrap();
    shutdown.join().unwrap();
    eprintln!(
        "queued WAL and checkpoint shutdown after release: {:?}",
        started.elapsed()
    );
    *crate::journal::WAL_WRITE_OBSERVER.lock() = None;
    drop(core);

    let mut state = recover_dimension_state(&config, crate::dimensions::Dimension::Overworld)
        .expect("recover checkpoint after shutdown");
    for (index, uuid) in players.into_iter().enumerate() {
        let player = state.offline_players.get(&uuid).expect("durable player");
        assert!(
            player
                .inventory
                .iter()
                .any(|item| item.item_id == dirt && item.count == index as i32 + 3)
        );
    }
    state.world.ensure_blocking(
        &[(
            ChunkPos {
                x: position.x.div_euclid(16),
                z: position.z.div_euclid(16),
            },
            0,
        )]
        .into_iter()
        .collect(),
    );
    assert_eq!(
        state.world.block_at(position.x, position.y, position.z),
        Some(stone)
    );
    drop(state);
    let reopened = Core::open(config).expect("reopen after shutdown");
    reopened.shutdown();
}

#[test]
fn panic_hook_logs_message_and_source_location() {
    stop::install_panic_hook();
    let log = capture(|| {
        assert!(std::panic::catch_unwind(|| panic!("tick owner test panic")).is_err());
    });
    assert!(log.contains("ERROR"), "{log}");
    assert!(log.contains("core process panic"), "{log}");
    assert!(log.contains("tick owner test panic"), "{log}");
    assert!(log.contains("core_stop_tests.rs:"), "{log}");
}

#[test]
fn failed_owner_join_logs_payload() {
    let owner = std::thread::Builder::new()
        .name("mc-core-test-owner".into())
        .spawn(|| panic!("owner join test panic"))
        .unwrap();
    let log = capture(|| stop::join(owner));
    assert!(log.contains("ERROR"), "{log}");
    assert!(log.contains("mc-core-test-owner"), "{log}");
    assert!(log.contains("owner join test panic"), "{log}");
    assert!(log.contains("core owner join failed"), "{log}");
}

#[test]
fn stop_error_display_includes_source_chain() {
    #[derive(Debug, thiserror::Error)]
    #[error("journal write failed")]
    struct Failure(#[source] std::io::Error);

    let error = Failure(std::io::Error::other("disk full"));
    assert_eq!(
        stop::Chain(&error).to_string(),
        "journal write failed: disk full"
    );
}

/// A startup failure after the journal owner started must join it before
/// returning Err, with the world lease held until it has ended. Otherwise the
/// flock is released while the journal owner is still a writer, so a second
/// start on the same world becomes a second writer.
#[test]
fn startup_failure_after_the_journal_owner_started_joins_it_before_returning() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = CoreConfig::new(directory.path().to_path_buf(), 42);
    config.checkpoint_every_ticks = u64::MAX;
    Core::open(config.clone())
        .expect("create the world")
        .shutdown();
    // `InputLog::open` is the fallible step after the journal owner spawned: a
    // regular file in the log directory's place fails exactly that step.
    std::fs::write(
        directory.path().join(crate::input_log::DIR),
        b"not a directory",
    )
    .unwrap();
    // Hold the owner's exit, so "joined before returning" is measured instead of
    // racing the owner's own quick exit on the closed work queue.
    let (entered_tx, entered) = std::sync::mpsc::channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    *super::journal::JOURNAL_EXIT_HOLD.lock() =
        Some((directory.path().to_path_buf(), entered_tx, release_rx));

    let mut failing = config.clone();
    failing.input_log = true;
    let (returned_tx, returned) = std::sync::mpsc::channel();
    let opening = std::thread::spawn(move || {
        let result = Core::open(failing).map(|_| ());
        let _ = returned_tx.send(());
        result
    });
    entered
        .recv()
        .expect("the journal owner never reached its exit path");
    // A start that joins its journal owner cannot return while that owner is
    // held here; the deadline only bounds a start that does not join at all.
    let early = returned.recv_timeout(std::time::Duration::from_secs(2));
    release.send(()).expect("release the journal owner");
    let error = opening
        .join()
        .expect("failed start thread")
        .expect_err("start must fail")
        .to_string();
    assert!(
        early.is_err(),
        "the failed start returned while its journal owner was still running"
    );
    assert!(error.contains("input log"), "{error}");
    // Joined and lease-free: the same process reopens the world instead of
    // meeting a live writer behind a released lease.
    Core::open(config)
        .expect("reopen after the failed start")
        .shutdown();
}

/// The startup owner guard stops and joins its owners before the lease is
/// released, for both windows `Core::open` has: the journal owner alone (the
/// tick never started) and the journal plus tick owners.
#[test]
fn startup_owner_guard_stops_and_joins_before_the_lease_is_released() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let directory = tempfile::tempdir().unwrap();
    let _lease = lock_world_lease(directory.path()).expect("lease the world");

    // Journal only: nothing else owns the work queue, so closing it ends the
    // journal owner and the guard's drop joins it.
    let journal_done = Arc::new(AtomicBool::new(false));
    let (work_tx, work_rx) = std::sync::mpsc::sync_channel::<JournalWork>(1);
    let (shutdown_tx, _) = std::sync::mpsc::channel::<()>();
    let flag = Arc::clone(&journal_done);
    let journal = std::thread::spawn(move || {
        let _ = work_rx.recv();
        flag.store(true, Ordering::SeqCst);
    });
    drop(stop::StartupOwners::new(shutdown_tx, work_tx, journal));
    assert!(
        journal_done.load(Ordering::SeqCst),
        "journal owner outlived the guard"
    );

    // Journal and tick: the guard signals the tick owner, joins it, and only
    // then releases anything; a tick that is still running must see the world
    // still leased.
    let tick_done = Arc::new(AtomicBool::new(false));
    let lease_held = Arc::new(AtomicBool::new(false));
    let second_journal_done = Arc::new(AtomicBool::new(false));
    let (work_tx, work_rx) = std::sync::mpsc::sync_channel::<JournalWork>(1);
    let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel::<()>();
    let flag = Arc::clone(&second_journal_done);
    let journal = std::thread::spawn(move || {
        let _ = work_rx.recv();
        flag.store(true, Ordering::SeqCst);
    });
    let mut guard = stop::StartupOwners::new(shutdown_tx, work_tx, journal);
    let work = guard.take_work();
    let probe_dir = directory.path().to_path_buf();
    let running = Arc::clone(&tick_done);
    let held = Arc::clone(&lease_held);
    let tick = std::thread::spawn(move || {
        let probe = mc_world::storage::open_for_lease(&probe_dir).expect("probe the lease");
        held.store(
            matches!(probe.try_lock(), Err(std::fs::TryLockError::WouldBlock)),
            Ordering::SeqCst,
        );
        let _ = shutdown_rx.recv();
        let _ = work.send(super::JournalWork::Shutdown);
        running.store(true, Ordering::SeqCst);
    });
    guard.set_tick(tick);
    drop(guard);
    assert!(
        tick_done.load(Ordering::SeqCst),
        "tick owner outlived the guard"
    );
    assert!(
        second_journal_done.load(Ordering::SeqCst),
        "journal owner outlived the tick owner's Shutdown item"
    );
    assert!(
        lease_held.load(Ordering::SeqCst),
        "the lease was free while an owner thread was still running"
    );
}
