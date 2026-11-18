//! Books R4-P1: a durable checkpoint marker outside redb, on a real Core and
//! real files. The marker records `(store id, checkpoint tick)` after every
//! committed checkpoint and before its WAL is pruned, so a redb repair that
//! rolls the store back behind an acknowledged checkpoint is refused at open
//! instead of silently serving the older world.
use super::*;
use mc_world::store_marker::{self, StoreMarker};

fn config(path: &std::path::Path, checkpoint_every_ticks: u64) -> CoreConfig {
    let mut config = CoreConfig::new(path.to_path_buf(), 42);
    config.view_distance = 1;
    config.simulation_distance = 0;
    config.checkpoint_every_ticks = checkpoint_every_ticks;
    config.journal_fsync_every_batches = 1;
    config
}

fn marker(config: &CoreConfig) -> StoreMarker {
    store_marker::read(&config.world_dir)
        .unwrap()
        .expect("a world with a checkpoint has a marker")
}

fn wait_for_tick(rx: &mut tokio::sync::broadcast::Receiver<TickOutput>, at_least: u64) {
    loop {
        match rx.blocking_recv() {
            Ok(output) if output.tick >= at_least => return,
            Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                panic!("tick thread stopped before tick {at_least}")
            }
        }
    }
}

fn batch_ticks(config: &CoreConfig) -> Vec<u64> {
    let mut ticks = Vec::new();
    for entry in std::fs::read_dir(config.world_dir.join("journal")).unwrap() {
        let name = entry.unwrap().file_name();
        if let Some((lo, hi)) = crate::journal::parse_batch_name(&name.to_string_lossy()) {
            ticks.push((lo, hi));
        }
    }
    ticks.sort_unstable();
    ticks.into_iter().map(|(_, hi)| hi).collect()
}

/// One observed checkpoint stage and the marker tick durable at that moment.
type StageEvent = (&'static str, Option<u64>);

/// One observed stage, the marker tick durable at that moment, and the two
/// rendezvous the tests wait on instead of sleeping.
struct Observe {
    path: PathBuf,
    events: Arc<parking_lot::Mutex<Vec<StageEvent>>>,
    pruned: std::sync::mpsc::Receiver<u64>,
    failed: std::sync::mpsc::Receiver<()>,
}

impl Observe {
    fn new(path: &std::path::Path) -> Self {
        let events = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let observed = Arc::clone(&events);
        let watched = path.to_path_buf();
        let (pruned_tx, pruned) = std::sync::mpsc::channel();
        let (failed_tx, failed) = std::sync::mpsc::channel();
        CHECKPOINT_STAGE_OBSERVERS.lock().push((
            path.to_path_buf(),
            Box::new(move |stage| {
                let tick = store_marker::read(&watched)
                    .ok()
                    .flatten()
                    .map(|marker| marker.checkpoint_tick);
                observed.lock().push((stage, tick));
                match stage {
                    "pruned" => {
                        let _ = pruned_tx.send(tick.unwrap_or(0));
                    }
                    "ack_none_sent" => {
                        let _ = failed_tx.send(());
                    }
                    _ => {}
                }
            }),
        ));
        Self {
            path: path.to_path_buf(),
            events,
            pruned,
            failed,
        }
    }

    /// Block until a checkpoint at or above `at_least` finished pruning, i.e. its
    /// marker write and ACK already happened.
    fn wait_pruned(&mut self, at_least: u64) -> u64 {
        loop {
            let tick = self
                .pruned
                .recv_timeout(std::time::Duration::from_secs(120))
                .expect("no checkpoint reached its prune");
            if tick >= at_least {
                return tick;
            }
        }
    }

    /// Block until a checkpoint answered with a negative ACK.
    fn wait_negative_ack(&mut self) {
        self.failed
            .recv_timeout(std::time::Duration::from_secs(120))
            .expect("no checkpoint answered with a negative ACK");
    }

    fn events(&self) -> Vec<(&'static str, Option<u64>)> {
        self.events.lock().clone()
    }
}

impl Drop for Observe {
    fn drop(&mut self) {
        CHECKPOINT_STAGE_OBSERVERS
            .lock()
            .retain(|(path, _)| path != &self.path);
    }
}

#[test]
fn a_checkpoint_writes_the_marker_after_the_commit_and_before_the_ack_and_prune() {
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path(), 1);
    let core = Core::open(config.clone()).unwrap();
    let mut rx = core.subscribe();
    assert_eq!(marker(&config).checkpoint_tick, 0, "the initial checkpoint");

    let mut observe = Observe::new(&config.world_dir);
    wait_for_tick(&mut rx, 2);
    observe.wait_pruned(2);
    core.shutdown();
    drop(core);

    let events = observe.events();
    let failures: Vec<_> = events
        .iter()
        .map(|(stage, _)| *stage)
        .filter(|stage| *stage == "ack_none_sent")
        .collect();
    assert!(failures.is_empty(), "no checkpoint failed here");
    let first_commit = events
        .iter()
        .position(|(stage, _)| *stage == "native_committed")
        .expect("a native commit stage");
    let first_ack = events
        .iter()
        .position(|(stage, _)| *stage == "before_ack")
        .expect("an acknowledgement stage");
    assert!(first_commit < first_ack);
    let at_commit = events[first_commit]
        .1
        .expect("a marker exists before a commit");
    let at_ack = events[first_ack]
        .1
        .expect("the marker is durable at the acknowledgement");
    assert!(
        at_ack > at_commit,
        "the marker must move after the native commit and before the ACK: {at_commit} -> {at_ack}"
    );
    // Prune follows the marker: nothing at or below the final marker tick survives.
    let final_marker = marker(&config).checkpoint_tick;
    for hi in batch_ticks(&config) {
        assert!(
            hi > final_marker,
            "a batch covered by marker tick {final_marker} was not pruned"
        );
    }
}

/// Open a world, save its tick-0 store, let it checkpoint further and stop.
/// Returns the saved tick-0 store path and the final marker.
fn world_with_saved_earlier_store(root: &std::path::Path) -> (CoreConfig, PathBuf, StoreMarker) {
    let config = config(&root.join("world"), 1);
    let core = Core::open(config.clone()).unwrap();
    let mut rx = core.subscribe();
    assert_eq!(marker(&config).checkpoint_tick, 0);
    let earlier = root.join("state-tick0.redb");
    std::fs::copy(config.world_dir.join("native/state.redb"), &earlier).unwrap();
    let mut observe = Observe::new(&config.world_dir);
    wait_for_tick(&mut rx, 2);
    observe.wait_pruned(1);
    // Shutdown checkpoints again, so the final marker is read after it.
    core.shutdown();
    drop(core);
    let marker = marker(&config);
    (config, earlier, marker)
}

#[test]
fn a_store_rolled_back_behind_the_marker_is_refused_before_replay() {
    let root = tempfile::tempdir().unwrap();
    let (config, earlier, marker) = world_with_saved_earlier_store(root.path());
    assert!(marker.checkpoint_tick >= 1);
    // A real older store file stands in for redb's own slot switch: the pair
    // now records a checkpoint the store no longer has.
    std::fs::copy(&earlier, config.world_dir.join("native/state.redb")).unwrap();

    let error = Core::open(config.clone())
        .expect_err("a store behind its marker must not open")
        .to_string();
    assert!(error.contains("behind checkpoint marker"), "{error}");
    assert!(error.contains("checkpoint 0"), "{error}");
    assert!(
        error.contains(&format!("marker {}", marker.checkpoint_tick)),
        "{error}"
    );
    assert!(error.contains("refusing to open"), "{error}");
}

#[test]
fn a_marker_from_another_store_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root.path().join("world"), u64::MAX);
    let core = Core::open(config.clone()).unwrap();
    core.shutdown();
    drop(core);

    let ours = marker(&config);
    let foreign = [0x5a; 16];
    store_marker::write(&config.world_dir, foreign, ours.checkpoint_tick).unwrap();
    let error = Core::open(config.clone())
        .expect_err("foreign marker")
        .to_string();
    assert!(error.contains("belong to different worlds"), "{error}");
    assert!(
        error.contains(&uuid::Uuid::from_bytes(foreign).to_string()),
        "{error}"
    );
    assert!(
        error.contains(&uuid::Uuid::from_bytes(ours.store_uuid).to_string()),
        "{error}"
    );
}

#[test]
fn a_missing_marker_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root.path().join("world"), u64::MAX);
    let core = Core::open(config.clone()).unwrap();
    core.shutdown();
    drop(core);

    std::fs::remove_file(store_marker::path(&config.world_dir)).unwrap();
    let error = Core::open(config.clone())
        .expect_err("missing marker")
        .to_string();
    assert!(
        error.contains("is missing but this store has checkpoint"),
        "{error}"
    );
    assert!(error.contains("no converter"), "{error}");
}

#[test]
fn a_store_ahead_of_the_marker_is_the_crash_window_and_rewrites_the_marker() {
    let root = tempfile::tempdir().unwrap();
    let (config, _earlier, marker) = world_with_saved_earlier_store(root.path());
    // The process died between the redb commit and the marker write.
    store_marker::write(
        &config.world_dir,
        marker.store_uuid,
        marker.checkpoint_tick - 1,
    )
    .unwrap();

    let core = Core::open(config.clone()).expect("a store ahead of the marker opens");
    assert_eq!(
        store_marker::read(&config.world_dir).unwrap(),
        Some(marker),
        "the marker is rewritten to the committed checkpoint"
    );
    core.shutdown();
    drop(core);
}

/// SM2-1: a first start interrupted between its tick-0 commit and the first
/// marker write must not brick the new world. The first-start file is the
/// bootstrap proof; without it the same state is refused.
#[test]
fn an_interrupted_first_start_publishes_the_marker_once_and_then_refuses() {
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path(), u64::MAX);
    FAIL_CHECKPOINT_MARKER
        .lock()
        .unwrap()
        .push(config.world_dir.clone());
    let error = Core::open(config.clone())
        .expect_err("the initial marker write is injected to fail")
        .to_string();
    assert!(error.contains("checkpoint marker"), "{error}");
    assert!(
        config.world_dir.join(FIRST_START_MARKER).exists(),
        "the interrupted first start keeps its proof"
    );
    assert!(store_marker::read(&config.world_dir).unwrap().is_none());
    FAIL_CHECKPOINT_MARKER
        .lock()
        .unwrap()
        .retain(|dir| dir != &config.world_dir);

    // Retry: the store has checkpoint 0 and the first-start file proves it.
    let core = Core::open(config.clone()).expect("interrupted first start retries");
    assert_eq!(
        marker(&config).checkpoint_tick,
        0,
        "the tick-0 pair is published"
    );
    core.shutdown();
    drop(core);
    assert!(!config.world_dir.join(FIRST_START_MARKER).exists());

    // The same state without the proof is a rolled-back world: refused.
    let marker = marker(&config);
    std::fs::remove_file(store_marker::path(&config.world_dir)).unwrap();
    let error = Core::open(config.clone())
        .expect_err("a served world without its marker is refused")
        .to_string();
    assert!(
        error.contains("is missing but this store has checkpoint"),
        "{error}"
    );
    // Restore the pair so the temporary world is left valid.
    store_marker::write(&config.world_dir, marker.store_uuid, marker.checkpoint_tick).unwrap();
}

/// SM2-3: the marker write fails at real file I/O (its temporary path is a
/// directory). The checkpoint is committed but not acknowledged, its WAL range
/// stays intact, and the writer stops.
#[test]
fn a_marker_write_failure_prunes_nothing_and_halts_the_world() {
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path(), 1);
    let core = Core::open(config.clone()).unwrap();
    let mut observe = Observe::new(&config.world_dir);
    assert_eq!(marker(&config).checkpoint_tick, 0);
    std::fs::create_dir(config.world_dir.join("checkpoint.marker.tmp"))
        .expect("block the marker's temporary path with a directory");

    observe.wait_negative_ack();
    let at_failure = marker(&config).checkpoint_tick;
    let batches_at_failure = batch_ticks(&config);
    core.shutdown();
    drop(core);

    assert_eq!(
        marker(&config).checkpoint_tick,
        at_failure,
        "a failed marker write must not publish the checkpoint"
    );
    assert_eq!(
        batch_ticks(&config),
        batches_at_failure,
        "the failed checkpoint's WAL must not be pruned"
    );
    assert!(
        batches_at_failure.contains(&(at_failure + 1)),
        "the failed checkpoint's own WAL batch must stay: {batches_at_failure:?}"
    );
    let events = observe.events();
    let failed = events
        .iter()
        .position(|(stage, _)| *stage == "ack_none_sent")
        .expect("the checkpoint answered with a negative ACK");
    let committed = events[..failed]
        .iter()
        .rposition(|(stage, _)| *stage == "native_committed")
        .expect("the checkpoint did commit before its marker failed");
    assert_eq!(
        events[committed].1,
        Some(at_failure),
        "the commit happened and the marker stayed at the last good checkpoint"
    );
    assert!(
        events[failed..].iter().all(|(stage, _)| *stage != "pruned"),
        "a checkpoint whose marker failed must not prune: {:?}",
        &events[failed..]
    );
}
