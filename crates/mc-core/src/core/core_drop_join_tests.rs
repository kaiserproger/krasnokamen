//! CORE-DROP-RELEASES-LEASE-BEFORE-OWNERS-JOIN.
//!
//! A plain `drop(core)` must run the same stop order as `Core::shutdown`: the
//! tick and journal owners are joined before the handle returns and before the
//! world lease is released. The test measures that boundary with real events:
//! a held native checkpoint publication, the tick owner's broadcast closing,
//! the world lease flock, and finally the reopened world.

use super::*;
use crate::boundary::{BlockPos, ChangeSet, ChunkPos, ConnectionId, PlayerInput, WorldSnapshot};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct TickObservation {
    snapshot: Arc<WorldSnapshot>,
    changes: Arc<ChangeSet>,
}

enum Event {
    Tick(TickObservation),
    /// The tick owner exited and dropped the broadcast sender: exact terminal.
    TickStreamClosed,
    /// The thread that owned the `Core` finished `drop(core)`.
    DropReturned,
}

fn config(directory: &std::path::Path) -> CoreConfig {
    let mut config = CoreConfig::new(directory.to_owned(), 42);
    config.view_distance = 1;
    config.simulation_distance = 1;
    // A real checkpoint every tick, so a native publication is easy to hold.
    config.checkpoint_every_ticks = 1;
    config.journal_fsync_every_batches = 1;
    config
}

/// Tick outputs plus the exact stream-closed terminal event over an `mpsc`
/// channel. The sender is returned so the drop thread reports its own terminal
/// event on the same channel.
fn subscribe(core: &Core) -> (mpsc::Receiver<Event>, mpsc::Sender<Event>) {
    let mut broadcast = core.subscribe();
    let (tx, rx) = mpsc::channel();
    let forwarder = tx.clone();
    std::thread::spawn(move || {
        loop {
            match broadcast.blocking_recv() {
                Ok(output) => {
                    let observation = TickObservation {
                        snapshot: output.snapshot,
                        changes: output.changes,
                    };
                    if forwarder.send(Event::Tick(observation)).is_err() {
                        return;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    let _ = forwarder.send(Event::TickStreamClosed);
                    return;
                }
            }
        }
    });
    (rx, tx)
}

fn wait_for<T>(
    events: &mpsc::Receiver<Event>,
    what: &str,
    mut found: impl FnMut(&TickObservation) -> Option<T>,
) -> T {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "timed out waiting for {what}");
        match events.recv_timeout(left) {
            Ok(Event::Tick(output)) => {
                if let Some(value) = found(&output) {
                    return value;
                }
            }
            Ok(Event::TickStreamClosed) => panic!("tick stream closed while waiting for {what}"),
            Ok(Event::DropReturned) => panic!("drop returned while waiting for {what}"),
            Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for {what}"),
            Err(RecvTimeoutError::Disconnected) => {
                panic!("event channel ended while waiting for {what}")
            }
        }
    }
}

/// Join a permission-4 player and write a known air→stone block at the world
/// spawn +5, gated on the published outputs (the pattern of the existing
/// shutdown durability test). Returns the block position and the stone state.
fn write_a_block(core: &Core, events: &mpsc::Receiver<Event>) -> (BlockPos, u32) {
    let uuid = uuid::Uuid::from_u128(0xD0_09_5E_11);
    let connection = ConnectionId(51);
    assert!(core.submit(PlayerInput::Join {
        connection,
        uuid,
        name: "DropProbe".into(),
        permission_level: 4,
    }));
    wait_for(events, "join", |output| {
        output
            .snapshot
            .entities
            .iter()
            .any(|entity| entity.uuid == uuid)
            .then_some(())
    });
    let stone = core.registry().states.stone;
    let [x, y, z] = core.world_spawn();
    let position = BlockPos { x, y: y + 5, z };
    assert!(core.submit(PlayerInput::Command {
        connection,
        command: format!(
            "setblock {} {} {} minecraft:stone",
            position.x, position.y, position.z
        ),
        permission_level: 4,
    }));
    wait_for(events, "the block write", |output| {
        output
            .changes
            .blocks
            .iter()
            .any(|change| change.position == position && change.state == stone)
            .then_some(())
    });
    (position, stone)
}

/// Keeps the path-scoped observer out of the global registry on every exit path.
struct ObserverGuard(std::path::PathBuf);

impl Drop for ObserverGuard {
    fn drop(&mut self) {
        super::journal::CHECKPOINT_STAGE_OBSERVERS
            .lock()
            .retain(|(path, _)| path != &self.0);
    }
}

/// CORE-DROP-RELEASES-LEASE-BEFORE-OWNERS-JOIN.
///
/// The real native checkpoint writer is held at `before_native_commit`. With
/// the fix the plain drop joins the tick first: its broadcast closes (an exact
/// terminal event) while the publication is still held. At that boundary the
/// world lease must still be `WouldBlock`. Only the RAII release publishes;
/// then the drop returns and the checkpointed block replays and reopens.
/// Without the fix the drop returns first, the lease is already free, and the
/// held publication lands after the handle was dropped.
#[test]
fn plain_drop_holds_the_world_lease_until_the_publication_commits() {
    // The writer publication is held through a process-global observer; keep it
    // out of the parent test process so unrelated journals never block on it.
    const CHILD: &str = "KRASNOKAMEN_CORE_DROP_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let full_name = concat!(
            module_path!(),
            "::plain_drop_holds_the_world_lease_until_the_publication_commits"
        );
        let test_name = full_name.split_once("::").unwrap().1;
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test_name, "--nocapture"])
            .env(CHILD, "1")
            .output()
            .expect("run the isolated plain-drop test");
        assert!(
            child.status.success(),
            "plain-drop child failed: stdout={} stderr={}",
            String::from_utf8_lossy(&child.stdout),
            String::from_utf8_lossy(&child.stderr)
        );
        eprint!("{}", String::from_utf8_lossy(&child.stderr));
        return;
    }

    let directory = tempfile::tempdir().unwrap();
    let config = config(directory.path());
    let core = Core::open(config.clone()).expect("open the world");
    let owners = core.owners_probe();
    let stop = core.shutdown_sender_probe().expect("shutdown sender");
    let (events, event_tx) = subscribe(&core);
    let (position, stone) = write_a_block(&core, &events);

    let _observers = ObserverGuard(directory.path().to_path_buf());
    let armed = Arc::new(AtomicBool::new(true));
    let (entered_tx, entered_rx) = mpsc::channel();
    let (committed_tx, committed_rx) = mpsc::channel();
    let entered = Arc::new(Mutex::new(entered_tx));
    let committed = Arc::new(Mutex::new(committed_tx));
    let (release, release_rx) = mpsc::channel::<()>();
    {
        let armed = Arc::clone(&armed);
        let entered = Arc::clone(&entered);
        let committed = Arc::clone(&committed);
        let release_rx = Mutex::new(Some(release_rx));
        super::journal::CHECKPOINT_STAGE_OBSERVERS.lock().push((
            directory.path().to_path_buf(),
            Box::new(move |stage| match stage {
                "before_native_commit" if armed.swap(false, Ordering::SeqCst) => {
                    let _ = entered.lock().unwrap().send(());
                    // RAII one-shot: dropping the test's sender publishes; a
                    // panic in the test drops it too instead of deadlocking.
                    let receiver = release_rx.lock().unwrap().take();
                    if let Some(receiver) = receiver {
                        let _ = receiver.recv();
                    }
                }
                "native_committed" => {
                    let _ = committed.lock().unwrap().send(());
                }
                _ => {}
            }),
        ));
    }
    entered_rx
        .recv_timeout(Duration::from_secs(90))
        .expect("no checkpoint reached the native commit");
    // Only the held publication may commit next.
    while committed_rx.try_recv().is_ok() {}

    let drop_events = event_tx.clone();
    let dropping = std::thread::spawn(move || {
        drop(core);
        let _ = drop_events.send(Event::DropReturned);
    });

    // Exact terminal multiplex, no timing prediction.
    let first = loop {
        match events.recv_timeout(Duration::from_secs(90)) {
            Ok(Event::Tick(_)) => {}
            Ok(Event::TickStreamClosed) => break Event::TickStreamClosed,
            Ok(Event::DropReturned) => break Event::DropReturned,
            Err(RecvTimeoutError::Timeout) => panic!("neither boundary event arrived"),
            Err(RecvTimeoutError::Disconnected) => panic!("event channel ended"),
        }
    };

    let probe = mc_world::storage::open_for_lease(directory.path()).expect("probe the lease");
    let lease = probe.try_lock();
    drop(probe);

    if let Event::TickStreamClosed = first {
        // Fixed path: the tick broadcast closed while the publication and
        // lease are still held; closure alone does not prove the join completed.
        assert!(
            matches!(lease, Err(std::fs::TryLockError::WouldBlock)),
            "the plain drop released the world lease while a checkpoint publication was held"
        );
        drop(release);
        committed_rx
            .recv_timeout(Duration::from_secs(90))
            .expect("the held checkpoint never committed");
        loop {
            match events.recv_timeout(Duration::from_secs(90)) {
                Ok(Event::Tick(_)) => {}
                Ok(Event::DropReturned) => break,
                Ok(Event::TickStreamClosed) => panic!("tick stream closed twice"),
                Err(_) => panic!("the drop did not return after the publication committed"),
            }
        }
        dropping.join().expect("drop thread");
        assert!(
            owners.wait_zero(Duration::from_secs(60)),
            "an owner outlived the joined drop"
        );

        // Durability and reopen, after the owners have ended. The chunk must be
        // loaded before reading it; the state is dropped before the reopen
        // (store handle).
        let mut state = recover_dimension_state(&config, crate::dimensions::Dimension::Overworld)
            .expect("recover the world written before the plain drop");
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
            Some(stone),
            "the block written before the plain drop did not survive it"
        );
        drop(state);
        Core::open(config)
            .expect("reopen after the plain drop")
            .shutdown();
        return;
    }

    // Control: the drop returned while the publication was held and the lease
    // was already free. Release the writer, witness the publication landing
    // after the drop, stop the detached owners and wait for their end.
    assert!(
        lease.is_ok(),
        "a plain drop returned while the publication was held but kept the lease"
    );
    drop(release);
    committed_rx
        .recv_timeout(Duration::from_secs(90))
        .expect("the held checkpoint never committed after the drop");
    let _ = stop.send(());
    assert!(
        owners.wait_zero(Duration::from_secs(60)),
        "the detached owners did not end after cleanup"
    );
    panic!(
        "a plain drop returned while a real checkpoint publication was held: the lease was \
         already free and the publication landed after the handle was dropped"
    );
}

/// Clears the journal exit hold when a test leaves before the owner fires it.
struct ExitHoldGuard(std::path::PathBuf);

impl Drop for ExitHoldGuard {
    fn drop(&mut self) {
        let mut hold = super::journal::JOURNAL_EXIT_HOLD.lock();
        if hold.as_ref().is_some_and(|(dir, _, _)| dir == &self.0) {
            hold.take();
        }
    }
}

/// CORE-DROP-LEAVES-WORLD-DIR-WRITER.
///
/// The recorded symptom is `remove_dir_all` of the world directory failing with
/// `DirectoryNotEmpty` right after `drop(core)`: a writer still created a file
/// there. This test holds a real writer — the journal owner, which owns the
/// world directory's WAL — on its exit path and drops the core in another
/// thread. A plain drop must join that owner before it returns, so no writer is
/// left to create a file afterwards and the directory deletes.
#[test]
fn plain_drop_joins_a_journal_writer_holding_the_world_directory() {
    // The journal exit hold is process-global; isolate it like the lease test so
    // no unrelated journal in the parent process can ever park on it.
    const CHILD: &str = "KRASNOKAMEN_CORE_DROP_WORLD_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let full_name = concat!(
            module_path!(),
            "::plain_drop_joins_a_journal_writer_holding_the_world_directory"
        );
        let test_name = full_name.split_once("::").unwrap().1;
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test_name, "--nocapture"])
            .env(CHILD, "1")
            .output()
            .expect("run the isolated world-directory drop test");
        assert!(
            child.status.success(),
            "world-directory drop child failed: stdout={} stderr={}",
            String::from_utf8_lossy(&child.stdout),
            String::from_utf8_lossy(&child.stderr)
        );
        eprint!("{}", String::from_utf8_lossy(&child.stderr));
        return;
    }

    let directory =
        std::env::temp_dir().join(format!("mc-core-drop-world-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release, release_rx) = mpsc::channel::<()>();
    *super::journal::JOURNAL_EXIT_HOLD.lock() = Some((directory.clone(), entered_tx, release_rx));
    let guard = ExitHoldGuard(directory.clone());
    let core = Core::open(config(&directory)).expect("open the world");

    let (dropped_tx, dropped_rx) = mpsc::channel();
    let dropping = std::thread::spawn(move || {
        drop(core);
        let _ = dropped_tx.send(());
    });
    // The row's loss, detected first: a plain drop that detaches its owners
    // returns while the journal owner is still a live writer of the world
    // directory. A joined drop cannot return before `release` (the owner is
    // parked on the exit hold), so this window can only fire on the defect;
    // it is a detection bound, not a timing prediction of the fixed path.
    if dropped_rx
        .recv_timeout(Duration::from_secs(2))
        .is_ok()
    {
        panic!(
            "the plain drop returned while the journal owner was still alive: a writer was \
             left in the world directory"
        );
    }
    // `entered` is sent from inside the journal owner's exit path: the writer is
    // alive and joined, and the drop must still be waiting for it.
    entered_rx
        .recv_timeout(Duration::from_secs(90))
        .expect("the journal owner never reached its exit path");
    assert!(
        matches!(dropped_rx.try_recv(), Err(mpsc::TryRecvError::Empty)),
        "the plain drop returned while the journal owner still held the world directory"
    );
    drop(release);
    dropped_rx
        .recv_timeout(Duration::from_secs(90))
        .expect("the plain drop never returned after the journal owner exited");
    dropping.join().expect("drop thread");
    drop(guard);
    std::fs::remove_dir_all(&directory)
        .expect("a writer left in the world directory recreated a file after the drop returned");
}
