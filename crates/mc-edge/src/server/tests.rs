use super::*;

#[test]
fn load_queues_aggregate_real_slots_and_expose_unavailable_metrics() {
    let registry = ConnectionRegistry::new();
    assert!(registry.admit(ConnectionId(1), 2, |_| None).is_none());
    assert!(registry.admit(ConnectionId(2), 2, |_| None).is_none());
    let (reliable_tx, mut reliable_rx) = mpsc::channel(4);
    let (world_tx, mut world_rx) = world_queue(4, WORLD_QUEUE_BYTES);
    reliable_tx
        .try_send(bytes::Bytes::from_static(b"abc"))
        .unwrap();
    world_tx
        .try_send(bytes::Bytes::from_static(b"queued"))
        .unwrap();
    registry
        .register(
            ConnectionId(1),
            ConnectionHandles::new(reliable_tx.clone(), world_tx.clone()),
            || 0,
        )
        .unwrap();
    let (reliable_tx2, _reliable_rx2) = mpsc::channel(2);
    let (world_tx2, _world_rx2) = world_queue(2, WORLD_QUEUE_BYTES);
    world_tx2
        .try_send(bytes::Bytes::from_static(b"longer payload"))
        .unwrap();
    registry
        .register(
            ConnectionId(2),
            ConnectionHandles::new(reliable_tx2, world_tx2),
            || 0,
        )
        .unwrap();
    let snapshot = registry.load_queues().unwrap();
    assert_eq!(snapshot.viewers, 2);
    assert_eq!(snapshot.reliable.capacity_slots, 6);
    assert_eq!(snapshot.world.capacity_slots, 6);
    assert_eq!(snapshot.reliable.queued_or_reserved_slots, 1);
    assert_eq!(snapshot.world.queued_or_reserved_slots, 2);
    assert_eq!(snapshot.world.max_viewer_queued_or_reserved_slots, 1);
    assert!(snapshot.capabilities.queued_or_reserved_slots);
    assert!(!snapshot.capabilities.queued_slots);
    assert!(!snapshot.capabilities.reserved_slots);
    assert!(!snapshot.capabilities.queued_bytes);
    assert!(!snapshot.capabilities.reserved_bytes);
    assert!(snapshot.world.queued_bytes.is_none());
    assert!(snapshot.world.reserved_bytes.is_none());
    assert!(snapshot.world.queued_slots.is_none());
    assert!(snapshot.world.reserved_slots.is_none());
    world_rx.try_recv().unwrap();
    reliable_rx.try_recv().unwrap();
    let snapshot = registry.load_queues().unwrap();
    assert_eq!(snapshot.reliable.queued_or_reserved_slots, 0);
    assert_eq!(snapshot.world.queued_or_reserved_slots, 1);
}

#[test]
fn registry_insert_remove_roster() {
    let registry = ConnectionRegistry::new();
    let id = registry.joining_id();
    let (reliable_tx, _) = mpsc::channel(2);
    let (world_tx, _) = world_queue(2, WORLD_QUEUE_BYTES);
    registry
        .register(id, ConnectionHandles::new(reliable_tx, world_tx), || 0)
        .unwrap();
    assert_eq!(registry.len(), 1);
    registry.set_name(id, "Alex".to_owned());
    assert_eq!(registry.roster(), vec!["Alex".to_owned()]);
    registry.remove(id);
    assert!(registry.is_empty());
}

#[test]
fn routing_snapshot_copies_handles_only_on_change() {
    let registry = ConnectionRegistry::new();
    let mut generation = 0u64;
    let (handles, roster) = registry.routing_snapshot(&mut generation);
    assert!(handles.is_none(), "no connection, nothing to copy");
    assert!(roster.is_none());

    let id = registry.joining_id();
    let (reliable_tx, _reliable_rx) = mpsc::channel(2);
    let (world_tx, _world_rx) = world_queue(2, WORLD_QUEUE_BYTES);
    registry
        .register(id, ConnectionHandles::new(reliable_tx, world_tx), || 0)
        .unwrap();
    let (handles, _) = registry.routing_snapshot(&mut generation);
    let handles = handles.expect("a join copies the routing view once");
    assert_eq!(handles.len(), 1, "routing follows the join");
    assert_eq!(handles[0].0, id);

    let (handles, _) = registry.routing_snapshot(&mut generation);
    assert!(handles.is_none(), "an unchanged tick clones no handle");

    registry.set_name(id, "Alex".to_owned());
    let (handles, roster) = registry.routing_snapshot(&mut generation);
    assert!(handles.is_none(), "a roster change is not a routing change");
    assert_eq!(roster, Some(vec!["Alex".to_owned()]));

    registry.remove(id);
    let (handles, _) = registry.routing_snapshot(&mut generation);
    assert!(
        handles.expect("a leave copies the routing view").is_empty(),
        "routing follows the leave"
    );
    assert!(registry.routing_snapshot(&mut generation).0.is_none());
}

/// Informative microbench (debug build, shared host): per-tick routing cost at
/// K = 1000 connections — the previous per-tick clone of every handle versus
/// one generation compare. No threshold; it prints both numbers. Ignored by
/// default: it is a measurement, not a gate (`cargo test -- --ignored`).
#[test]
#[ignore = "informative microbench, not a gate"]
fn routing_snapshot_tick_cost_at_1k() {
    const K: usize = 1000;
    const TICKS: u32 = 100;
    let registry = ConnectionRegistry::new();
    let mut queues = Vec::with_capacity(K);
    for _ in 0..K {
        let id = registry.joining_id();
        let (reliable_tx, reliable_rx) = mpsc::channel(2);
        let (world_tx, world_rx) = world_queue(2, WORLD_QUEUE_BYTES);
        registry
            .register(id, ConnectionHandles::new(reliable_tx, world_tx), || 0)
            .unwrap();
        queues.push((reliable_rx, world_rx));
    }
    let mut generation = 0u64;
    let (handles, _) = registry.routing_snapshot(&mut generation);
    assert_eq!(handles.expect("first tick copies").len(), K);

    // Before: one clone of every handle per tick (the old `routing_snapshot`).
    let start = std::time::Instant::now();
    for _ in 0..TICKS {
        std::hint::black_box(registry.snapshot());
    }
    let before = start.elapsed() / TICKS;

    // After: an unchanged tick takes the mutex and compares one counter.
    let start = std::time::Instant::now();
    for _ in 0..TICKS {
        let (handles, _) = registry.routing_snapshot(&mut generation);
        assert!(handles.is_none(), "no session change, no copy");
    }
    let after = start.elapsed() / TICKS;
    println!(
        "routing view K={K} per tick (debug, shared host, informative): before {before:?}, after {after:?}"
    );
    drop(queues);
}

#[test]
fn dashboard_gate_requires_allow_remote() {
    let mut cfg = EdgeConfig::default();
    cfg.dashboard.bind_address = "0.0.0.0".to_owned();
    assert!(!dashboard_allowed(&cfg));
    cfg.dashboard.allow_remote = true;
    assert!(dashboard_allowed(&cfg));
    cfg.dashboard.bind_address = "127.0.0.1".to_owned();
    cfg.dashboard.allow_remote = false;
    assert!(dashboard_allowed(&cfg));
}

#[test]
fn core_config_carries_distances_and_airtime_limit() {
    let mut cfg = EdgeConfig::default();
    cfg.server.view_distance = 4;
    cfg.server.simulation_distance = 3;
    cfg.anti_cheat.max_airtime_ticks = 9;
    let core = core_config(&cfg);
    assert_eq!(core.view_distance, 4);
    assert_eq!(core.simulation_distance, 3);
    assert_eq!(core.max_airtime_ticks, 9);
}

#[test]
fn core_config_carries_pause_and_mob_griefing_switches() {
    let mut cfg = EdgeConfig::default();
    assert_eq!(core_config(&cfg).pause_when_empty_seconds, 60);
    cfg.server.pause_when_empty_seconds = 5;
    cfg.mob_griefing.creeper_block_damage = false;
    cfg.mob_griefing.villager_farming = false;
    let core = core_config(&cfg);
    assert_eq!(core.pause_when_empty_seconds, 5);
    assert!(!core.mob_griefing.creeper_block_damage);
    assert!(!core.mob_griefing.villager_farming);
    assert!(core.mob_griefing.ghast_block_damage);
    assert!(core.mob_griefing.wither_block_damage);
    assert!(core.mob_griefing.sheep_grazing);
}

#[test]
fn core_config_carries_block_log_policy_and_preserves_defaults() {
    use mc_core::config::BlockLogSettings;
    let mut cfg = EdgeConfig::default();
    let defaults = BlockLogSettings::default();
    assert_eq!(
        defaults,
        BlockLogSettings {
            enabled: true,
            retention_secs: 30 * 24 * 60 * 60,
            max_records: 200_000,
        }
    );
    assert_eq!(core_config(&cfg).block_log, defaults);
    cfg.block_log.enabled = false;
    cfg.block_log.retention_days = 7;
    cfg.block_log.max_records = 41;
    assert_eq!(
        core_config(&cfg).block_log,
        BlockLogSettings {
            enabled: false,
            retention_secs: 7 * 24 * 60 * 60,
            max_records: 41,
        }
    );
}

#[test]
fn core_config_carries_checkpoint_cadence() {
    let mut cfg = EdgeConfig::default();
    let core_default = mc_core::config::CoreConfig::new(cfg.data.world_dir.clone(), 0);
    assert_eq!(
        core_config(&cfg).checkpoint_every_ticks,
        core_default.checkpoint_every_ticks
    );
    cfg.data.checkpoint_every_ticks = 1_000_000;
    assert_eq!(core_config(&cfg).checkpoint_every_ticks, 1_000_000);
}

/// Two simultaneous joins both resolve; a late waiter sees the stash.
#[tokio::test]
async fn simultaneous_joins_both_resolve() {
    let registry = ConnectionRegistry::new();
    let a = registry.joining_id();
    let b = registry.joining_id();
    // Both register interest BEFORE either Joined arrives (simultaneous).
    let rx_a = registry.wait_join(a);
    let rx_b = registry.wait_join(b);
    // Pump routes A's Joined: fires A's waiter, stashes for fast path.
    let joined_a = (
        mc_core::boundary::EntityId(11),
        [1.0, 2.0, 3.0],
        0u8,
        Default::default(),
    );
    registry.stash_join(a, joined_a);
    // B's waiter must NOT fire from A's Joined.
    assert!(registry.take_join(b).is_none());
    // Pump routes B's Joined.
    let joined_b = (
        mc_core::boundary::EntityId(22),
        [4.0, 5.0, 6.0],
        1u8,
        Default::default(),
    );
    registry.stash_join(b, joined_b);
    assert_eq!(rx_a.await.expect("A resolves"), joined_a);
    assert_eq!(rx_b.await.expect("B resolves"), joined_b);
    // Stash fast path still holds both for `take_join` callers.
    assert_eq!(registry.take_join(a), Some(joined_a));
    assert_eq!(registry.take_join(b), Some(joined_b));
}

/// Fast path: Joined stashed before interest registered resolves
/// immediately (covers the pump-beat-us-to-it window).
#[tokio::test]
async fn late_waiter_sees_stash() {
    let registry = ConnectionRegistry::new();
    let id = registry.joining_id();
    let joined = (
        mc_core::boundary::EntityId(7),
        [8.0, 80.0, 8.0],
        0u8,
        Default::default(),
    );
    registry.stash_join(id, joined);
    // wait_for_join's fast path.
    assert_eq!(registry.take_join(id), Some(joined));
}

/// P1: the whole pre-registration reliable burst is stashed in arrival
/// order (not just the Joined), so registration can replay it.
#[test]
fn pending_reliable_keeps_burst_order() {
    use mc_core::boundary::PlayerChange;
    let registry = ConnectionRegistry::new();
    let id = registry.joining_id();
    let burst = vec![
        PlayerChange::Joined {
            entity: mc_core::boundary::EntityId(41),
            dimension: Default::default(),
            position: [8.0, 64.0, 8.0],
            yaw: 0.0,
            pitch: 0.0,
            game_mode: 0,
        },
        PlayerChange::MenuContents {
            window: 0,
            state_id: 1,
            slots: Vec::new(),
            cursor: None,
        },
        PlayerChange::Health {
            health: 20.0,
            food: 20,
            saturation: 5.0,
        },
        PlayerChange::Experience {
            progress: 0.0,
            level: 0,
            total: 0,
        },
    ];
    let mut first = burst.clone();
    assert!(registry.handles_or_stash(id, &mut first).is_none());
    // A second pre-registration drain appends in order (never replaces).
    let mut late = vec![PlayerChange::Chat {
        message: "late".to_owned(),
    }];
    assert!(registry.handles_or_stash(id, &mut late).is_none());
    let mut expected = burst;
    expected.push(PlayerChange::Chat {
        message: "late".to_owned(),
    });
    let (reliable_tx, _reliable_rx) = mpsc::channel(1);
    let (world_tx, _world_rx) = world_queue(1, WORLD_QUEUE_BYTES);
    let handles = ConnectionHandles::new(reliable_tx, world_tx);
    assert_eq!(
        registry.register(id, handles.clone(), || 0).unwrap(),
        expected
    );
    // Once registered the pump routes instead of stashing.
    let mut routed = vec![expected[1].clone()];
    assert!(registry.handles_or_stash(id, &mut routed).is_some());
    assert_eq!(routed.len(), 1);
    // Remove also clears a stash.
    registry.remove(id);
    assert!(registry.handles_or_stash(id, &mut routed).is_none());
    registry.remove(id);
    assert!(registry.register(id, handles, || 0).is_err());
}

#[test]
fn queue_registration_seals_the_native_count_inside_its_critical_section() {
    let registry = ConnectionRegistry::new();
    let id = registry.joining_id();
    let (reliable_tx, _reliable_rx) = mpsc::channel(1);
    let (world_tx, _world_rx) = world_queue(1, WORLD_QUEUE_BYTES);
    registry
        .register(id, ConnectionHandles::new(reliable_tx, world_tx), || {
            assert!(registry.inner.try_lock().is_none());
            7
        })
        .unwrap();
    assert_eq!(registry.get(id).unwrap().knockback_baseline, 7);
}

/// P1: the stashed Joined is the viewer entity source, so repeated
/// `take_join` calls keep returning it (registration + pump ensures).
#[test]
fn stashed_join_survives_repeated_takes() {
    let registry = ConnectionRegistry::new();
    let id = registry.joining_id();
    let joined = (
        mc_core::boundary::EntityId(99),
        [1.0, 2.0, 3.0],
        1u8,
        Default::default(),
    );
    registry.stash_join(id, joined);
    assert_eq!(registry.take_join(id), Some(joined));
    assert_eq!(registry.take_join(id), Some(joined));
}

/// The join-burst replay must skip the stale `Joined` sync itself: the
/// burst already sent teleport id 1 + empty inventory/health, and the
/// `Joined` arm would re-emit all four (second teleport + reset to
/// empty). Only the authoritative tail replays.
#[test]
fn join_replay_skips_stale_joined_sync() {
    use mc_core::boundary::PlayerChange;
    let id = ConnectionId(3);
    let stashed = vec![
        PlayerChange::Joined {
            entity: mc_core::boundary::EntityId(41),
            dimension: Default::default(),
            position: [8.0, 80.0, 8.0],
            yaw: 0.0,
            pitch: 0.0,
            game_mode: 0,
        },
        PlayerChange::MenuContents {
            window: 0,
            state_id: 1,
            slots: Vec::new(),
            cursor: None,
        },
        PlayerChange::Health {
            health: 20.0,
            food: 20,
            saturation: 5.0,
        },
    ];
    // Mirror of the registration replay: drop through the Joined.
    let tail: Vec<PlayerChange> = stashed
        .into_iter()
        .skip_while(|change| !matches!(change, PlayerChange::Joined { .. }))
        .skip(1)
        .collect();
    assert_eq!(tail.len(), 2);
    let mut viewer =
        crate::plan::ViewerState::new(id, mc_core::boundary::EntityId(41), [8.0, 80.0, 8.0]);
    use mc_protocol::packets::Packet as _;
    use mc_protocol::packets::play::{
        ClientboundContainerSetContent, ClientboundSetHealth, SynchronizePlayerPosition,
    };
    let planned = crate::plan::plan_player_changes(&tail, &mut viewer);
    assert!(
        planned
            .iter()
            .all(|p| p.id != SynchronizePlayerPosition::ID),
        "replay must not re-send the stale join teleport"
    );
    assert!(
        planned
            .iter()
            .any(|p| p.id == ClientboundContainerSetContent::ID)
    );
    assert!(planned.iter().any(|p| p.id == ClientboundSetHealth::ID));
}

#[test]
fn native_settlements_c_binding_decodes_to_its_declared_owner() {
    use mc_protocol::packets::CustomPayload;
    use mc_protocol::packets::Packet;
    use mc_protocol::packets::play::ServerboundCustomPayload;

    let manifest =
        mc_loader::LoaderManifest::from_script_bundles(&[native_settlements_bundle().unwrap()])
            .unwrap();
    let packet = ServerboundCustomPayload {
        payload: CustomPayload::Unknown {
            channel: mc_loader::loader_view_action_channel().clone(),
            payload: br#"{"protocol":3,"message":"view_request","request_kind":"krasnokamen-settlements:overview"}"#.to_vec(),
        },
    };
    let mut body = Vec::new();
    packet.encode(&mut body).unwrap();

    let LoaderViewIngress::Request(PlayerInput::LoaderViewRequest {
        connection,
        owner,
        request_kind,
    }) = loader_view_ingress(&manifest, ConnectionId(41), &body).unwrap()
    else {
        panic!("C binding must route a declared view request");
    };
    assert_eq!(connection, ConnectionId(41));
    assert_eq!(owner, NATIVE_SETTLEMENTS_OWNER);
    assert_eq!(request_kind.as_str(), "krasnokamen-settlements:overview");
}

#[test]
fn native_settlements_bundle_is_optional_and_offered_to_loader_clients_only() {
    let manifest = loader_manifest(&[])
        .unwrap()
        .expect("native bundle manifest");
    assert_eq!(
        manifest.bundles.len(),
        1,
        "only the native settlements bundle"
    );
    assert!(
        !native_settlements_bundle().unwrap().required(),
        "the native bundle never forces the Loader on a vanilla client"
    );
    assert!(
        !manifest.bundles[0]
            .loaders
            .contains(&mc_loader::LoaderPlatform::Native),
        "the native client must not parse Java Loader input bindings"
    );
}
#[test]
fn packet_limiter_is_per_type_refills_and_can_be_disabled() {
    let config = crate::config::AntiCheatSection {
        packet_rate_per_second: 2,
        packet_burst: 2,
        max_airtime_ticks: 80,
    };
    let mut limiter = PacketLimiter::new(&config);
    let now = Instant::now();
    assert!(limiter.allow_at(5, now));
    assert!(limiter.allow_at(5, now));
    assert!(!limiter.allow_at(5, now));
    assert!(
        limiter.allow_at(6, now),
        "packet ids have independent buckets"
    );
    assert!(limiter.allow_at(5, now + std::time::Duration::from_millis(500)));

    let disabled = crate::config::AntiCheatSection {
        packet_rate_per_second: 0,
        packet_burst: 0,
        max_airtime_ticks: 80,
    };
    let mut limiter = PacketLimiter::new(&disabled);
    assert!(limiter.allow_at(5, now));
    assert!(limiter.allow_at(5, now));
}

#[tokio::test]
async fn close_wakes_the_connection_owner_with_its_cause() {
    let registry = ConnectionRegistry::new();
    let id = ConnectionId(7);
    let (close, cancelled) = tokio::sync::oneshot::channel();
    registry.inner.lock().closers.insert(id, close);
    registry.close(id, "reliable-overflow");
    assert_eq!(cancelled.await, Ok("reliable-overflow"));
    // A plain remove names no abnormal cause.
    let (close, cancelled) = tokio::sync::oneshot::channel();
    registry.inner.lock().closers.insert(id, close);
    registry.remove(id);
    assert_eq!(cancelled.await, Ok("removed"));
}
