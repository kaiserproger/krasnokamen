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

/// Wait until the marker on disk witnesses `at_least`, i.e. that checkpoint's
/// marker write finished.
fn wait_for_marker(config: &CoreConfig, at_least: u64) -> StoreMarker {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        if let Some(marker) = store_marker::read(&config.world_dir).unwrap()
            && marker.checkpoint_tick >= at_least
        {
            return marker;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "checkpoint marker never reached tick {at_least}"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn last_batch_tick(config: &CoreConfig) -> u64 {
    let mut last = 0;
    for entry in std::fs::read_dir(config.world_dir.join("journal")).unwrap() {
        let name = entry.unwrap().file_name();
        if let Some((_, hi)) = crate::journal::parse_batch_name(&name.to_string_lossy()) {
            last = last.max(hi);
        }
    }
    last
}

/// One observed stage and the marker tick that was durable at that moment.
type StageEvent = (&'static str, Option<u64>);

/// Records every native checkpoint stage together with the marker tick visible
/// at that moment, so the write order can be checked rather than inferred.
struct Observe {
    path: PathBuf,
    events: Arc<parking_lot::Mutex<Vec<StageEvent>>>,
}

impl Observe {
    fn new(path: &std::path::Path) -> Self {
        let events = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let observed = Arc::clone(&events);
        let watched = path.to_path_buf();
        CHECKPOINT_STAGE_OBSERVERS.lock().push((
            path.to_path_buf(),
            Box::new(move |stage| {
                let tick = store_marker::read(&watched)
                    .ok()
                    .flatten()
                    .map(|marker| marker.checkpoint_tick);
                observed.lock().push((stage, tick));
            }),
        ));
        Self {
            path: path.to_path_buf(),
            events,
        }
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

    let observe = Observe::new(&config.world_dir);
    wait_for_tick(&mut rx, 2);
    let committed = wait_for_marker(&config, 2);
    core.shutdown();
    drop(core);

    let events = observe.events();
    let first_commit = events
        .iter()
        .position(|(stage, _)| *stage == "native_committed")
        .expect("a native commit stage");
    let first_ack = events
        .iter()
        .position(|(stage, _)| *stage == "before_ack")
        .expect("an acknowledgement stage");
    assert!(first_commit < first_ack);
    assert_eq!(
        events[first_commit].1,
        Some(0),
        "the marker must not move before the native commit"
    );
    assert!(
        events[first_ack].1.is_some_and(|tick| tick >= 1),
        "the marker must be durable before the checkpoint is acknowledged: {:?}",
        events[first_ack]
    );
    // Prune follows the marker: nothing at or below the marker tick survives.
    for entry in std::fs::read_dir(config.world_dir.join("journal")).unwrap() {
        let name = entry.unwrap().file_name();
        if let Some((_, hi)) = crate::journal::parse_batch_name(&name.to_string_lossy()) {
            assert!(
                hi > committed.checkpoint_tick,
                "batch {name:?} covered by marker tick {} was not pruned",
                committed.checkpoint_tick
            );
        }
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
    wait_for_tick(&mut rx, 2);
    let marker = wait_for_marker(&config, 2);
    core.shutdown();
    drop(core);
    (config, earlier, marker)
}

#[test]
fn a_store_rolled_back_behind_the_marker_is_refused_before_replay() {
    let root = tempfile::tempdir().unwrap();
    let (config, earlier, marker) = world_with_saved_earlier_store(root.path());
    assert!(marker.checkpoint_tick >= 2);
    // redb repair swapped to the older commit: the store is behind the marker
    // whose WAL was already pruned.
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

#[test]
fn a_marker_write_failure_prunes_nothing_and_halts_the_world() {
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path(), 1);
    let core = Core::open(config.clone()).unwrap();
    let mut rx = core.subscribe();
    assert_eq!(marker(&config).checkpoint_tick, 0);
    FAIL_CHECKPOINT_MARKER
        .lock()
        .unwrap()
        .push(config.world_dir.clone());

    // The writer stops; the tick thread halts and closes the broadcast.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        match rx.try_recv() {
            Ok(_)
            | Err(tokio::sync::broadcast::error::TryRecvError::Empty)
            | Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {}
            Err(tokio::sync::broadcast::error::TryRecvError::Closed) => break,
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the world kept serving after a checkpoint marker write failure"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    core.shutdown();

    assert_eq!(
        marker(&config).checkpoint_tick,
        0,
        "a failed marker write must not publish the checkpoint"
    );
    assert!(
        last_batch_tick(&config) >= 1,
        "the failed checkpoint's WAL must not be pruned"
    );
    FAIL_CHECKPOINT_MARKER
        .lock()
        .unwrap()
        .retain(|dir| dir != &config.world_dir);
}
