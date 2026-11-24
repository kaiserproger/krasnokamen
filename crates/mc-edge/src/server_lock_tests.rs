use super::*;

#[test]
fn routing_snapshot_only_rebuilds_changed_rosters() {
    let registry = ConnectionRegistry::new();
    let id = registry.joining_id();
    let (reliable_tx, _reliable_rx) = mpsc::channel(2);
    let (world_tx, _world_rx) = world_queue(2, WORLD_QUEUE_BYTES);
    registry
        .register(
            id,
            ConnectionHandles::new(reliable_tx, world_tx, crate::plan::CHUNKS_PER_TICK),
            || 0,
        )
        .unwrap();
    assert!(registry.routing_view().1.is_none());
    registry.set_name(id, "Zoe".into());
    registry.set_name(ConnectionId(99), "Alex".into());
    let (handles, names) = registry.routing_view();
    assert!(handles.contains_key(&id));
    assert_eq!(names.unwrap(), ["Alex", "Zoe"]);
    assert!(registry.routing_view().1.is_none());
    registry.set_name(id, "Zoe".into());
    assert!(registry.routing_view().1.is_none());
    registry.set_name(id, "Sam".into());
    assert_eq!(registry.routing_view().1.unwrap(), ["Alex", "Sam"]);
    registry.remove(id);
    let (handles, names) = registry.routing_view();
    assert!(handles.is_empty());
    assert_eq!(names.unwrap(), ["Alex"]);
    assert!(registry.routing_view().1.is_none());
}

#[tokio::test]
async fn registry_removal_clears_all_connection_state() {
    let registry = ConnectionRegistry::new();
    let id = registry.joining_id();
    let (reliable_tx, mut reliable_rx) = mpsc::channel(2);
    let (world_tx, mut world_rx) = world_queue(2, WORLD_QUEUE_BYTES);
    registry
        .register(
            id,
            ConnectionHandles::new(reliable_tx, world_tx, crate::plan::CHUNKS_PER_TICK),
            || 0,
        )
        .unwrap();
    registry.set_name(id, "Alex".into());
    registry.stash_disconnect_reason(id, "test".into());
    let waiter = registry.wait_join(id);
    registry.remove(id);
    assert!(waiter.await.is_err());
    assert!(registry.get(id).is_none());
    assert!(registry.take_join(id).is_none());
    assert!(registry.take_disconnect_reason(id).is_none());
    assert!(registry.roster().is_empty());
    assert!(reliable_rx.recv().await.is_none());
    assert!(world_rx.recv().await.is_none());
}

#[tokio::test]
async fn join_waiter_observes_join_stashed_between_check_and_registration() {
    let registry = ConnectionRegistry::new();
    let id = registry.joining_id();
    assert!(registry.take_join(id).is_none());
    let joined = JoinedSpawn {
        entity: mc_core::boundary::EntityId(7),
        position: [1.0, 2.0, 3.0],
        rotation: [0.0, 0.0],
        game_mode: 0,
        dimension: Default::default(),
    };
    registry.stash_join(id, joined);
    assert_eq!(registry.wait_join(id).await.unwrap(), joined);
    registry.remove(id);
    assert!(registry.take_join(id).is_none());
}

#[test]
// The base update is what keeps this literal identical in a tree whose `PumpTick` still carries
// the close-cause map and one that reads it on request; in the second it names no extra field.
#[allow(clippy::needless_update)]
fn batched_dashboard_record_updates_counters_and_retains_unchanged_roster() {
    let state = dashboard_state(&EdgeConfig::default());
    state.record_pump_tick(crate::dashboard::PumpTick {
        tick: 4,
        core_us: 10,
        publish_us: 20,
        bytes_written: 123,
        reliable_drops: 2,
        chunks: DashboardChunks { loaded_total: 3 },
        entities: DashboardEntities { total: 5 },
        players: Some(crate::dashboard::DashboardPlayers {
            count: 1,
            max: 20,
            names: vec!["Alex".into()],
        }),
        ..Default::default()
    });
    let sample = state.snapshot();
    assert_eq!(sample.simulation_tick, 4);
    assert_eq!(sample.network.bytes_written, 123);
    assert_eq!(sample.network.reliable_drops, 2);
    assert_eq!(sample.chunks.loaded_total, 3);
    assert_eq!(sample.entities.total, 5);
    assert_eq!(sample.players.names, ["Alex"]);
    state.record_pump_tick(crate::dashboard::PumpTick {
        tick: 5,
        ..Default::default()
    });
    assert_eq!(state.snapshot().players.names, ["Alex"]);
}
