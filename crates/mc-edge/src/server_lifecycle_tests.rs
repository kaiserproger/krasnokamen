use super::*;

fn bulk_score_changes() -> Vec<mc_core::boundary::PlayerChange> {
    vec![mc_core::boundary::PlayerChange::Scoreboard {
        events: (0..601)
            .map(|i| mc_core::scoreboard::ScoreboardEvent::Score {
                owner: format!("holder{i:04}"),
                objective: "bulk".into(),
                score: 7,
            })
            .collect::<Vec<_>>()
            .into(),
    }]
}

fn assert_bulk_frames(bytes: bytes::Bytes) {
    use mc_protocol::packets::{Packet, play::ClientboundSetScore};
    let mut bytes = bytes::BytesMut::from(bytes.as_ref());
    for i in 0..601 {
        let frame = mc_protocol::frame::try_decode_frame(
            &mut bytes,
            mc_protocol::frame::Compression::Threshold(256),
        )
        .unwrap()
        .expect("score frame");
        assert_eq!(frame.id, ClientboundSetScore::ID);
        let score = ClientboundSetScore::decode(&mut frame.body.clone()).unwrap();
        assert_eq!(score.owner, format!("holder{i:04}"));
        assert_eq!(score.objective, "bulk");
        assert_eq!(score.score, 7);
    }
    assert!(bytes.is_empty());
}

#[test]
fn scoreboard_bulk_join_replay_fits_reliable_queue() {
    let registry = ConnectionRegistry::new();
    let id = registry.joining_id();
    let entity = mc_core::boundary::EntityId(1);
    let mut changes = vec![mc_core::boundary::PlayerChange::Joined {
        entity,
        dimension: Default::default(),
        position: [0.0; 3],
        yaw: 0.0,
        pitch: 0.0,
        game_mode: 0,
    }];
    changes.extend(bulk_score_changes());
    assert!(registry.handles_or_stash(id, &mut changes).is_none());
    let (reliable_tx, mut reliable_rx) = mpsc::channel(crate::fanout::OUTBOUND_QUEUE_BOUND);
    let (world_tx, _world_rx) = mpsc::channel(crate::fanout::OUTBOUND_QUEUE_BOUND);
    let stashed = registry
        .register(
            id,
            ConnectionHandles {
                reliable_tx,
                world_tx,
            },
        )
        .unwrap();
    let mut viewer = crate::plan::ViewerState::new(id, entity, [0.0; 3]);
    replay_join_tail(&registry, id, stashed, &mut viewer).expect("bulk join replay");
    assert_bulk_frames(reliable_rx.try_recv().expect("queued join batch"));
    assert!(reliable_rx.try_recv().is_err());
}

#[test]
fn scoreboard_bulk_live_updates_fit_reliable_queue() {
    let id = ConnectionId(1);
    let mut viewer = crate::plan::ViewerState::new(id, mc_core::boundary::EntityId(1), [0.0; 3]);
    let packets = crate::plan::plan_player_changes(&bulk_score_changes(), &mut viewer);
    let (tx, mut rx) = mpsc::channel(crate::fanout::OUTBOUND_QUEUE_BOUND);
    for packet in packets {
        tx.try_send(packet.data).expect("bulk live reliable queue");
    }
    assert_bulk_frames(rx.try_recv().expect("queued live batch"));
    assert!(rx.try_recv().is_err());
}

#[test]
fn text_burst_live_updates_fit_reliable_queue() {
    use mc_protocol::packets::{Packet, play::ClientboundSystemChat};
    let id = ConnectionId(1);
    let mut viewer = crate::plan::ViewerState::new(id, mc_core::boundary::EntityId(1), [0.0; 3]);
    let lines = (0..601)
        .map(|i| mc_core::boundary::PlayerChange::Chat {
            message: format!("line {i:04}"),
        })
        .collect();
    let burst = [mc_core::boundary::PlayerChange::TextBurst { lines }];
    let packets = crate::plan::plan_player_changes(&burst, &mut viewer);
    let (tx, mut rx) = mpsc::channel(crate::fanout::OUTBOUND_QUEUE_BOUND);
    for packet in packets {
        tx.try_send(packet.data).expect("text burst reliable queue");
    }
    let mut bytes = bytes::BytesMut::from(rx.try_recv().expect("queued text burst").as_ref());
    for _ in 0..601 {
        let frame = mc_protocol::frame::try_decode_frame(
            &mut bytes,
            mc_protocol::frame::Compression::Threshold(256),
        )
        .unwrap()
        .expect("chat frame");
        assert_eq!(frame.id, ClientboundSystemChat::ID);
    }
    assert!(bytes.is_empty());
    assert!(rx.try_recv().is_err());
}

#[test]
fn removed_registration_fails_join_replay_without_panicking() {
    let registry = ConnectionRegistry::new();
    let id = registry.joining_id();
    let entity = mc_core::boundary::EntityId(1);
    let mut changes = vec![
        mc_core::boundary::PlayerChange::Joined {
            entity,
            dimension: Default::default(),
            position: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            game_mode: 0,
        },
        mc_core::boundary::PlayerChange::Chat {
            message: "join tail".into(),
        },
    ];
    assert!(registry.handles_or_stash(id, &mut changes).is_none());
    let (reliable_tx, mut reliable_rx) = mpsc::channel(4);
    let (world_tx, _world_rx) = mpsc::channel(4);
    let stashed = registry
        .register(
            id,
            ConnectionHandles {
                reliable_tx,
                world_tx,
            },
        )
        .unwrap();
    assert_eq!(stashed.len(), 2);
    // The pump can close delivery after register succeeds, before replay.
    registry.remove(id);
    let mut viewer = crate::plan::ViewerState::new(id, entity, [0.0; 3]);
    let error = replay_join_tail(&registry, id, stashed, &mut viewer).unwrap_err();
    assert!(error.to_string().contains("closed"));
    assert!(matches!(
        reliable_rx.try_recv(),
        Err(mpsc::error::TryRecvError::Disconnected)
    ));
}

#[test]
fn late_reliable_results_do_not_revive_removed_connection() {
    let registry = ConnectionRegistry::new();
    let id = registry.joining_id();
    let (reliable_tx, _reliable_rx) = mpsc::channel(4);
    let (world_tx, _world_rx) = mpsc::channel(4);
    registry
        .register(
            id,
            ConnectionHandles {
                reliable_tx,
                world_tx,
            },
        )
        .unwrap();
    registry.remove(id);

    let mut late = vec![mc_core::boundary::PlayerChange::Chat {
        message: "already closed".to_owned(),
    }];
    assert!(registry.handles_or_stash(id, &mut late).is_none());
    let inner = registry.inner.lock();
    assert!(
        !inner.pending_reliable.contains_key(&id),
        "closed connection must not regain a pending queue"
    );
    assert!(!inner.pending_joins.contains_key(&id));
}

#[test]
fn closing_slot_is_released_only_after_lifecycle_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let admin = crate::admin::Admin::open(dir.path()).unwrap();
    let registry = ConnectionRegistry::new();
    let id = registry.joining_id();
    let lifecycle = ConnectionLifecycle::new(&registry, &admin, id);
    registry.remove(id);
    let next = registry.alloc_id();
    assert!(
        registry
            .admit(next, 1, |full| full.then(|| serde_json::json!("full")))
            .is_some(),
        "closing must retain admission until native/admin cleanup runs"
    );
    drop(lifecycle);
    assert!(
        registry
            .admit(next, 1, |full| full.then(|| serde_json::json!("full")))
            .is_none()
    );
}

#[test]
fn concurrent_admission_reserves_the_last_slot_before_registration() {
    let registry = ConnectionRegistry::new();
    let barrier = std::sync::Barrier::new(2);
    let accepted = std::thread::scope(|scope| {
        let tasks: Vec<_> = (0..2)
            .map(|_| {
                scope.spawn(|| {
                    let id = registry.alloc_id();
                    barrier.wait();
                    registry
                        .admit(id, 1, |full| full.then(|| serde_json::json!("full")))
                        .is_none()
                })
            })
            .collect();
        tasks
            .into_iter()
            .map(|task| task.join().unwrap())
            .filter(|accepted| *accepted)
            .count()
    });
    assert_eq!(accepted, 1);
    assert_eq!(registry.inner.lock().joining.len(), 1);
    assert_eq!(
        registry.len(),
        0,
        "the winner has not registered Play handles yet"
    );
}

#[test]
fn admission_retains_the_real_operator_player_limit_bypass() {
    let dir = tempfile::tempdir().unwrap();
    let uuid = uuid::Uuid::from_u128(42);
    std::fs::write(
        dir.path().join("ops.json"),
        serde_json::to_vec(&serde_json::json!([{
            "uuid": uuid, "name": "Operator", "level": 4,
            "bypassesPlayerLimit": true
        }]))
        .unwrap(),
    )
    .unwrap();
    let admin = crate::admin::Admin::open(dir.path()).unwrap();
    let registry = ConnectionRegistry::new();
    let id = registry.alloc_id();
    assert!(
        registry
            .admit(id, 0, |full| admin.admission(uuid, None, full))
            .is_none()
    );
    assert!(registry.inner.lock().joining.contains(&id));
}

#[tokio::test]
async fn refused_join_wakes_the_waiter_and_keeps_the_reason_for_the_play_disconnect() {
    let registry = ConnectionRegistry::new();
    let id = registry.joining_id();
    let waiter = registry.wait_join(id);
    registry.stash_disconnect_reason(id, "You logged in from another location".into());
    // The reason alone does not end the wait: a Disconnect that arrives with a Joined must
    // still deliver the join first (the pump calls `refuse_join` only without a Joined).
    registry.refuse_join(id);
    assert!(
        waiter.await.is_err(),
        "no Joined will come: the join wait fails at once"
    );
    assert_eq!(
        registry.take_disconnect_reason(id).as_deref(),
        Some("You logged in from another location")
    );
}

#[tokio::test]
async fn removed_join_rejects_late_join_reason_waiter_and_registration() {
    let registry = ConnectionRegistry::new();
    let id = registry.joining_id();
    registry.remove(id);
    registry.stash_join(
        id,
        (
            mc_core::boundary::EntityId(1),
            [0.0; 3],
            0,
            Default::default(),
        ),
    );
    registry.stash_disconnect_reason(id, "late".into());
    assert!(registry.take_join(id).is_none());
    assert!(registry.take_disconnect_reason(id).is_none());
    assert!(registry.wait_join(id).await.is_err());
    let (reliable_tx, _) = mpsc::channel(1);
    let (world_tx, _) = mpsc::channel(1);
    assert!(
        registry
            .register(
                id,
                ConnectionHandles {
                    reliable_tx,
                    world_tx
                }
            )
            .is_err()
    );
    assert!(registry.is_empty());
}

#[tokio::test]
async fn delivery_close_wakes_owner_and_aborts_pending_writer() {
    let dir = tempfile::tempdir().unwrap();
    let registry = Arc::new(ConnectionRegistry::new());
    let admin = Arc::new(crate::admin::Admin::open(dir.path()).unwrap());
    let id = registry.joining_id();
    admin.register(
        id,
        uuid::Uuid::nil(),
        "SlowWriter".into(),
        "127.0.0.1".parse().unwrap(),
    );
    let writer = tokio::spawn(std::future::pending::<(&'static str, Option<String>)>());
    let abort = writer.abort_handle();
    let (ready, waiting) = tokio::sync::oneshot::channel();
    let task_registry = Arc::clone(&registry);
    let task_admin = Arc::clone(&admin);
    let owner = tokio::spawn(async move {
        let mut lifecycle = ConnectionLifecycle::new(&task_registry, &task_admin, id);
        lifecycle.writer = Some(writer);
        let cancelled = lifecycle.cancelled.take().unwrap();
        ready.send(()).unwrap();
        cancelled.await.unwrap();
        lifecycle.stop_writer().await;
        drop(lifecycle);
    });
    waiting.await.unwrap();
    registry.remove(id);
    owner.await.unwrap();
    assert!(
        abort.is_finished(),
        "owner completion must wait for the writer"
    );
    assert!(admin.roster(false).is_empty());
    assert!(registry.inner.lock().admitted.is_empty());
    assert!(registry.inner.lock().closers.is_empty());
}

#[tokio::test]
async fn task_cancellation_releases_admission_admin_and_writer() {
    let dir = tempfile::tempdir().unwrap();
    let registry = Arc::new(ConnectionRegistry::new());
    let admin = Arc::new(crate::admin::Admin::open(dir.path()).unwrap());
    let id = registry.joining_id();
    admin.register(
        id,
        uuid::Uuid::nil(),
        "Cancelled".into(),
        "127.0.0.1".parse().unwrap(),
    );
    let (writer_closed, writer_dropped) = tokio::sync::oneshot::channel::<()>();
    let writer = tokio::spawn(async move {
        let _closed = writer_closed;
        std::future::pending::<(&'static str, Option<String>)>().await
    });
    let (ready, waiting) = tokio::sync::oneshot::channel();
    let task_registry = Arc::clone(&registry);
    let task_admin = Arc::clone(&admin);
    let task = tokio::spawn(async move {
        let mut lifecycle = ConnectionLifecycle::new(&task_registry, &task_admin, id);
        lifecycle.writer = Some(writer);
        ready.send(()).unwrap();
        std::future::pending::<()>().await;
        drop(lifecycle);
    });
    waiting.await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(writer_dropped.await.is_err());
    assert!(admin.roster(false).is_empty());
    assert!(registry.inner.lock().joining.is_empty());
    let next = registry.alloc_id();
    assert!(
        registry
            .admit(next, 1, |full| full.then(|| serde_json::json!("full")))
            .is_none()
    );
}
