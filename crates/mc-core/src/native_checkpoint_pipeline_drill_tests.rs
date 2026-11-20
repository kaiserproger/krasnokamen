//! Item-bearing checkpoints through the production journal queue/native worker.
//! These complement the mc-world transaction-stage tests, not replace them.
use super::*;
use std::io::Write;
use std::path::PathBuf;

struct Observe {
    path: PathBuf,
    events: Arc<parking_lot::Mutex<Vec<&'static str>>>,
}
impl Observe {
    fn new(path: &std::path::Path) -> Self {
        let events = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let observed = Arc::clone(&events);
        CHECKPOINT_STAGE_OBSERVERS.lock().push((
            path.to_path_buf(),
            Box::new(move |stage| observed.lock().push(stage)),
        ));
        Self {
            path: path.to_path_buf(),
            events,
        }
    }
}
impl Drop for Observe {
    fn drop(&mut self) {
        CHECKPOINT_STAGE_OBSERVERS
            .lock()
            .retain(|(path, _)| path != &self.path);
    }
}
fn batch_path(config: &CoreConfig, tick: u64) -> PathBuf {
    config
        .world_dir
        .join("journal")
        .join(format!("batch-{tick:020}-{tick:020}.bin"))
}
fn assert_pruned(config: &CoreConfig, through: u64) {
    let journal = config.world_dir.join("journal");
    for entry in std::fs::read_dir(&journal).unwrap() {
        let name = entry.unwrap().file_name();
        if let Some((_, hi)) = crate::journal::parse_batch_name(&name.to_string_lossy()) {
            assert!(hi > through, "covered WAL batch {name:?} was not pruned");
        }
    }
    crate::journal::read_complete_batches_after_test(&journal, through).unwrap();
}
fn assert_new_checkpoint_then_recovery(mut recovered: Drill, expected: &Layout) {
    let previous = recovered.manifest.tick;
    recovered.run(&[]).unwrap();
    recovered.checkpoint();
    assert!(
        recovered.manifest.tick > previous,
        "a NEW checkpoint, not another database open"
    );
    assert_eq!(&recovered.native_layout(), expected);
    assert_pruned(&recovered.config, recovered.manifest.tick);
    recovered = recovered.crash();
    assert_eq!(
        &recovered.layout(),
        expected,
        "recovery after the new checkpoint pruned its WAL"
    );
}

fn inverse_transfer(drill: &mut Drill, before: &Layout) -> (Batch, Layout) {
    let id = drill.state.connections.entity_of(ConnectionId(1)).unwrap();
    let index = drill.state.players[&id]
        .inventory
        .slots
        .iter()
        .position(|stack| stack.custom_name.as_deref() == Some("durable-0"))
        .unwrap();
    let menu = drill.state.players[&id].open.as_ref().unwrap();
    let packet = PlayerInput::ContainerClick {
        connection: ConnectionId(1),
        window: menu.window,
        state_id: menu.state_id,
        slot: (if index < 9 {
            54 + index
        } else {
            27 + index - 9
        }) as i16,
        button: 0,
        mode: ClickMode::QuickMove,
    };
    let batch = drill.capture_tick(&[packet]);
    let after = drill.layout();
    assert_operation(1, Some("durable-0"), before, &after);
    (batch, after)
}

#[test]
fn transfer_checkpoint_failure_sends_negative_ack_and_retains_replay_custody() {
    let directory = tempfile::tempdir().unwrap();
    let (mut drill, batch, before, after) = prepared_transfer(directory.path());
    let target_tick = batch.tick;
    let previous_checkpoint = drill.manifest.tick;
    let observe = Observe::new(directory.path());
    let (job, ack, reply) = drill.capture_checkpoint();
    FAIL_CHECKPOINT_AFTER_REGIONS
        .lock()
        .unwrap()
        .push(directory.path().to_path_buf());
    drill.dispatch_checkpoint(job, Some(batch), None);
    assert!(
        ack.recv().unwrap().is_none(),
        "failed checkpoint cannot acknowledge captured revisions"
    );
    assert!(!reply.recv().unwrap());
    assert!(
        !FAIL_CHECKPOINT_AFTER_REGIONS
            .lock()
            .unwrap()
            .contains(&directory.path().to_path_buf()),
        "the requested failure was actually consumed"
    );
    assert_eq!(
        *observe.events.lock(),
        [
            "wal_gated",
            "before_native_commit",
            "before_ack",
            "ack_none_sent"
        ]
    );
    assert_eq!(drill.manifest.tick, previous_checkpoint);
    assert_eq!(
        drill.native_layout(),
        before,
        "failed checkpoint retained the complete old native placement"
    );
    assert!(
        batch_path(&drill.config, target_tick).exists(),
        "failed checkpoint must not prune the item transfer"
    );
    drop(observe);
    let recovered = drill.crash();
    assert_eq!(recovered.state.tick, target_tick);
    assert_eq!(
        recovered.layout(),
        after,
        "retained WAL recovers the COMPLETE transferred placement"
    );
    assert_new_checkpoint_then_recovery(recovered, &after);
}

#[test]
fn transfer_checkpoint_without_wal_coverage_refuses_ack_and_keeps_previous_transfer() {
    let directory = tempfile::tempdir().unwrap();
    let (mut drill, take, native_before, after_take) = prepared_transfer(directory.path());
    let take_tick = take.tick;
    drill.append(&take).unwrap();
    // Now request a real inverse transfer, but do not publish its tick. The
    // checkpoint must refuse the gap, retaining the earlier item-bearing WAL.
    let (missing, _) = inverse_transfer(&mut drill, &after_take);
    assert_eq!(missing.tick, take_tick + 1);
    let observe = Observe::new(directory.path());
    let (job, ack, reply) = drill.capture_checkpoint();
    let previous_checkpoint = drill.manifest.tick;
    drill.dispatch_checkpoint(job, None, None);
    assert!(ack.recv().unwrap().is_none());
    assert!(!reply.recv().unwrap());
    assert_eq!(
        *observe.events.lock(),
        ["gate_refused", "before_ack", "ack_none_sent"]
    );
    assert_eq!(drill.manifest.tick, previous_checkpoint);
    assert_eq!(drill.native_layout(), native_before);
    assert!(
        batch_path(&drill.config, take_tick).exists(),
        "the gate refusal cannot prune the PREVIOUS durable transfer"
    );
    assert!(!batch_path(&drill.config, missing.tick).exists());
    drop(observe);
    let recovered = drill.crash();
    assert_eq!(recovered.state.tick, take_tick);
    assert_eq!(
        recovered.layout(),
        after_take,
        "refused inverse transfer must not be smuggled into a checkpoint"
    );
    assert_new_checkpoint_then_recovery(recovered, &after_take);
}

#[test]
fn successful_transfer_checkpoint_acknowledges_and_prunes_only_covered_wal() {
    let directory = tempfile::tempdir().unwrap();
    let (mut drill, batch, _, after) = prepared_transfer(directory.path());
    let target_tick = batch.tick;
    let observe = Observe::new(directory.path());
    let (job, ack, reply) = drill.capture_checkpoint();
    let (tail, after_tail) = inverse_transfer(&mut drill, &after);
    let tail_tick = tail.tick;
    assert_eq!(tail_tick, target_tick + 1);
    let config = drill.config.clone();
    let previous_checkpoint = drill.manifest.tick;
    drill = std::thread::scope(|scope| {
        let (tail_ack, tail_received) = mpsc::channel();
        let tail_received = parking_lot::Mutex::new(tail_received);
        let (entered, before_commit) = mpsc::channel();
        // Own the release sender INSIDE this scope. An assertion failure drops
        // it before scope joins, waking the worker instead of stranding it.
        let (release, resume) = mpsc::channel();
        let resume = parking_lot::Mutex::new(resume);
        CHECKPOINT_STAGE_OBSERVERS.lock().push((
            config.world_dir.clone(),
            Box::new(move |stage| {
                if stage == "before_native_commit" {
                    assert_eq!(tail_received.lock().recv().unwrap(), tail_tick);
                    entered.send(tail_tick).unwrap();
                    resume.lock().recv().expect("pre-commit probe released");
                }
            }),
        ));
        let worker = scope.spawn(move || {
            drill.dispatch_checkpoint(job, Some(batch), Some((tail, tail_ack)));
            drill
        });
        assert_eq!(before_commit.recv().unwrap(), tail_tick);
        assert!(
            matches!(ack.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "no ACK before the native commit"
        );
        assert!(
            matches!(reply.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "no positive save reply before the native commit"
        );
        assert_eq!(read_manifest(&config).unwrap().tick, previous_checkpoint);
        assert!(
            batch_path(&config, target_tick).exists(),
            "captured transfer cannot be pruned before commit"
        );
        assert!(
            batch_path(&config, tail_tick).exists(),
            "newer transfer was actually append-ACKed"
        );
        release.send(()).unwrap();
        worker.join().unwrap()
    });
    let ack = ack.recv().unwrap();
    assert_eq!(ack.as_ref().map(|ack| ack.0), Some(target_tick));
    assert!(reply.recv().unwrap());
    assert_eq!(
        *observe.events.lock(),
        [
            "wal_gated",
            "before_native_commit",
            "native_committed",
            "before_ack",
            "ack_some_sent",
            "pruned"
        ]
    );
    assert_eq!(drill.manifest.tick, target_tick);
    assert_eq!(drill.native_layout(), after);
    assert!(!batch_path(&drill.config, target_tick).exists());
    assert!(
        batch_path(&drill.config, tail_tick).exists(),
        "newer inverse-transfer WAL is not covered by this checkpoint"
    );
    acknowledge_checkpoint(&mut drill.state, ack);
    drop(observe);
    // Apply the older capture's ACK to the LIVE newer state, not a recovered
    // state with newly seeded dirty marks. The next checkpoint must preserve
    // the inverse transfer even after that stale ACK, before any restart.
    drill.writer = Some(
        JournalWriter::open_recovered_test(&drill.config.world_dir, 1, target_tick, tail_tick)
            .unwrap(),
    );
    drill.checkpoint();
    assert_eq!(drill.manifest.tick, tail_tick);
    assert_eq!(drill.native_layout(), after_tail);
    assert_pruned(&drill.config, tail_tick);
    let recovered = drill.crash();
    assert_eq!(recovered.state.tick, tail_tick);
    assert_eq!(
        recovered.layout(),
        after_tail,
        "new native placement plus the complete newer inverse-transfer tail"
    );
    assert_new_checkpoint_then_recovery(recovered, &after_tail);
}

#[test]
fn abrupt_stop_at_core_checkpoint_boundaries_keeps_whole_item_placement() {
    const CHILD: &str = "KRASNOKAMEN_ITEM_CHECKPOINT_DRILL_PATH";
    const STAGES: [&str; 6] = [
        "wal_gated",
        "before_native_commit",
        "native_committed",
        "before_ack",
        "ack_some_sent",
        "pruned",
    ];
    if let Some(path) = std::env::var_os(CHILD) {
        let path = PathBuf::from(path);
        let stage = std::env::var("KRASNOKAMEN_ITEM_CHECKPOINT_DRILL_STAGE").unwrap();
        let stop = STAGES
            .into_iter()
            .find(|candidate| *candidate == stage)
            .unwrap();
        let mut config = CoreConfig::new(path.clone(), 42);
        config.view_distance = 0;
        config.simulation_distance = 0;
        let mut drill = Drill::open(config);
        let (id, _, _) = drill.open_chest().unwrap();
        drill.checkpoint();
        let before = drill.layout();
        let menu = drill.state.players[&id].open.as_ref().unwrap();
        let click = PlayerInput::ContainerClick {
            connection: ConnectionId(1),
            window: menu.window,
            state_id: menu.state_id,
            slot: 0,
            button: 0,
            mode: ClickMode::QuickMove,
        };
        let batch = drill.capture_tick(&[click]);
        assert_operation(0, Some("durable-0"), &before, &drill.layout());
        let (job, _ack, _reply) = drill.capture_checkpoint();
        let log_path = path.join("checkpoint-stage-receipt");
        CHECKPOINT_STAGE_OBSERVERS.lock().push((
            path.clone(),
            Box::new(move |stage| {
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&log_path)
                    .unwrap();
                writeln!(file, "{stage}").unwrap();
                file.sync_all().unwrap();
                if stage == stop {
                    std::process::exit(39);
                }
            }),
        ));
        drill.dispatch_checkpoint(job, Some(batch), None);
        panic!("checkpoint stop hook not reached");
    }
    let mut actual_stops = 0;
    for (stop_index, stage) in STAGES.into_iter().enumerate() {
        let directory = tempfile::tempdir().unwrap();
        let (drill, _, before, after) = prepared_transfer(directory.path());
        let config = drill.config.clone();
        drop(drill);
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "core::native_checkpoint_crash_tests::persistence_drills::checkpoint_pipeline::abrupt_stop_at_core_checkpoint_boundaries_keeps_whole_item_placement", "--nocapture"])
            .env(CHILD, directory.path()).env("KRASNOKAMEN_ITEM_CHECKPOINT_DRILL_STAGE", stage).status().unwrap();
        assert_eq!(status.code(), Some(39), "{stage}");
        actual_stops += 1;
        let receipt =
            std::fs::read_to_string(directory.path().join("checkpoint-stage-receipt")).unwrap();
        assert_eq!(
            receipt.lines().collect::<Vec<_>>(),
            STAGES[..=stop_index],
            "only ACTUALLY reached gate/commit/ACK/prune stages count"
        );
        let mut recovered = Drill::open(config);
        recovered.hydrate();
        let committed = stop_index >= 2;
        assert_eq!(
            &recovered.native_layout(),
            if committed { &after } else { &before },
            "{stage}: native checkpoint is wholly old or new"
        );
        assert_eq!(
            recovered.layout(),
            after,
            "{stage}: native plus retained WAL is the whole real transfer"
        );
        if stage == "pruned" {
            assert_pruned(&recovered.config, recovered.manifest.tick);
            assert_eq!(recovered.state.tick, recovered.manifest.tick);
        } else {
            assert!(
                batch_path(&recovered.config, recovered.state.tick).exists(),
                "{stage}: WAL cannot disappear before pruning"
            );
        }
        assert_new_checkpoint_then_recovery(recovered, &after);
    }
    assert_eq!(actual_stops, STAGES.len());
}
