//! One-shot knockback survives exhausted world credit and a refused world send.
use super::*;
use mc_core::boundary::{ChangeSet, EntityId, ExplosionEvent, TickOutput, WorldSnapshot};

#[tokio::test]
async fn explosions_deliver_or_explicitly_close_under_queue_pressure() {
    // A closed world receiver models a competing producer taking the planned
    // capacity: sends fail even though the credit read still reports free slots.
    let (server, dir) = fixture();
    let server = Arc::new(server);
    for world_pressure in ["slots", "bytes", "send-refused"] {
        for reliable_full in [false, true] {
            let id = server.connections.joining_id();
            let (reliable_tx, mut reliable_rx) = mpsc::channel(8);
            if reliable_full {
                for _ in 0..8 {
                    reliable_tx
                        .try_send(bytes::Bytes::from_static(b"queued"))
                        .unwrap();
                }
            }
            let (world_tx, world_rx) = world_queue(1, 1);
            let _world_rx = if world_pressure == "send-refused" {
                drop(world_rx);
                None
            } else {
                // The slot case fills both bounds; the byte case below has
                // spare slots and only its byte budget exhausted.
                Some(world_rx)
            };
            let (world_tx, _byte_rx) = if world_pressure == "bytes" {
                let (tx, rx) = world_queue(8, 1);
                tx.try_send(bytes::Bytes::from_static(b"x")).unwrap();
                (tx, Some(rx))
            } else {
                if world_pressure == "slots" {
                    world_tx.try_send(bytes::Bytes::from_static(b"x")).unwrap();
                }
                (world_tx, None)
            };
            server
                .connections
                .register(
                    id,
                    ConnectionHandles::new(reliable_tx, world_tx, crate::plan::CHUNKS_PER_TICK),
                    || server.hub.published_knockback_count(id),
                )
                .unwrap();
            let (close, mut closed) = tokio::sync::oneshot::channel();
            server.connections.inner.lock().closers.insert(id, close);
            let (mut connections, _) = server.connections.routing_view();
            let mut viewer = crate::plan::ViewerState::new(id, EntityId(1), [8.0, 64.0, 8.0]);
            // Seed the clock with an earlier, quiet publication so only the
            // explosion can cause the reliable overflow asserted below.
            crate::plan::plan_viewer(
                &TickOutput {
                    tick: 1,
                    snapshot: Arc::new(WorldSnapshot {
                        tick: 1,
                        day_time: 1,
                        day_time_rate: 1.0,
                        ..Default::default()
                    }),
                    changes: Arc::new(ChangeSet::default()),
                    times: Default::default(),
                },
                &mut viewer,
                0,
                &crate::plan::ChunkPacketCache::new(),
            );
            let mut viewers = HashMap::from([(id, viewer)]);
            let output = TickOutput {
                tick: 2,
                snapshot: Arc::new(WorldSnapshot {
                    tick: 2,
                    day_time: 2,
                    day_time_rate: 1.0,
                    ..Default::default()
                }),
                changes: Arc::new(ChangeSet {
                    explosions: vec![ExplosionEvent {
                        center: [8.0, 64.0, 8.0],
                        radius: 4.0,
                        blocks: 1,
                        knockback: vec![(EntityId(1), [0.1, 0.2, 0.3])],
                    }],
                    ..Default::default()
                }),
                times: Default::default(),
            };
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(1)
                .build()
                .unwrap();
            publish_and_route(
                &server,
                &pool,
                &mut viewers,
                &output,
                &mut crate::plan::ChunkPacketCache::new(),
                &mut connections,
            )
            .await;
            if reliable_full {
                assert_eq!(
                    tokio::time::timeout(std::time::Duration::from_secs(2), closed)
                        .await
                        .unwrap_or_else(|_| panic!("no close under {world_pressure} pressure"))
                        .unwrap(),
                    "reliable-queue-full",
                    "{world_pressure}"
                );
                assert!(server.connections.get(id).is_none());
                assert!(!viewers.contains_key(&id));
            } else {
                assert!(matches!(
                    closed.try_recv(),
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty)
                ));
                let frame = tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    receive(&mut reliable_rx),
                )
                .await
                .unwrap();
                assert_eq!(frame.id, ClientboundExplode::ID, "{world_pressure}");
                let blast = ClientboundExplode::decode(&mut &frame.body[..]).unwrap();
                assert_eq!(
                    blast.knockback.map(|push| [push.x, push.y, push.z]),
                    Some([0.1, 0.2, 0.3])
                );
                assert!(server.connections.get(id).is_some());
            }
            server.connections.remove(id);
        }
    }
    server.hub.shutdown();
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn six_hundred_explosions_coalesce_knockback_without_disconnect() {
    for visual_only in [false, true] {
        let (server, dir) = fixture();
        let server = Arc::new(server);
        let id = server.connections.joining_id();
        let (reliable_tx, mut reliable_rx) = mpsc::channel(crate::fanout::RELIABLE_QUEUE_BOUND);
        let (world_tx, _world_rx) = world_queue(4, WORLD_QUEUE_BYTES);
        server
            .connections
            .register(
                id,
                ConnectionHandles::new(reliable_tx, world_tx, crate::plan::CHUNKS_PER_TICK),
                || server.hub.published_knockback_count(id),
            )
            .unwrap();
        let (close, mut closed) = tokio::sync::oneshot::channel();
        server.connections.inner.lock().closers.insert(id, close);
        let (mut connections, _) = server.connections.routing_view();
        let mut viewers = HashMap::from([(
            id,
            crate::plan::ViewerState::new(id, EntityId(1), [8.0, 64.0, 8.0]),
        )]);
        let output = TickOutput {
            tick: 1,
            snapshot: Arc::new(WorldSnapshot {
                tick: 1,
                ..Default::default()
            }),
            changes: Arc::new(ChangeSet {
                explosions: (0..600)
                    .map(|_| ExplosionEvent {
                        center: [8.0, 64.0, 8.0],
                        radius: 4.0,
                        blocks: 1,
                        knockback: vec![(
                            EntityId(1),
                            if visual_only {
                                [0.0; 3]
                            } else {
                                [0.125, 0.25, 0.5]
                            },
                        )],
                    })
                    .collect(),
                ..Default::default()
            }),
            times: Default::default(),
        };
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap();
        publish_and_route(
            &server,
            &pool,
            &mut viewers,
            &output,
            &mut crate::plan::ChunkPacketCache::new(),
            &mut connections,
        )
        .await;
        assert!(server.connections.get(id).is_some());
        assert!(matches!(
            closed.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        let mut impulses = Vec::new();
        for _ in 0..reliable_rx.len() {
            let frame = receive(&mut reliable_rx).await;
            if frame.id == ClientboundExplode::ID {
                let packet = ClientboundExplode::decode(&mut &frame.body[..]).unwrap();
                let push = packet
                    .knockback
                    .expect("reliable explosion carries an impulse");
                impulses.push([push.x, push.y, push.z]);
            }
        }
        assert_eq!(
            impulses,
            if visual_only {
                vec![]
            } else {
                vec![[75.0, 150.0, 300.0]]
            }
        );
        server.connections.remove(id);
        server.hub.shutdown();
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[tokio::test]
async fn broadcast_lag_closes_only_recipients_of_skipped_knockback() {
    let (server, dir) = fixture();
    let server = Arc::new(server);
    let (lost, _lost_rx) = register(&server, "Lost", false);
    let (current, _current_rx) = register(&server, "Current", false);
    let (delivered, _delivered_rx) = register(&server, "Delivered", false);
    let mut closers = Vec::new();
    for id in [lost, current, delivered] {
        let (close, closed) = tokio::sync::oneshot::channel();
        server.connections.inner.lock().closers.insert(id, close);
        closers.push(closed);
    }
    let (mut connections, _) = server.connections.routing_view();
    let mut viewers = [lost, current, delivered]
        .into_iter()
        .map(|id| {
            (
                id,
                crate::plan::ViewerState::new(id, EntityId(id.0 as i32), [8.0, 64.0, 8.0]),
            )
        })
        .collect();
    let mut seen = std::collections::BTreeMap::new();
    let output = |tick, epochs: std::collections::BTreeMap<ConnectionId, (u64, u64)>| {
        let knockback = epochs
            .iter()
            .filter(|(_, (_, last_tick))| *last_tick == tick)
            .map(|(id, _)| (EntityId(id.0 as i32), [0.1, 0.2, 0.3]))
            .collect();
        TickOutput {
            tick,
            snapshot: Arc::new(WorldSnapshot {
                tick,
                player_knockback_ticks: Arc::new(epochs),
                ..Default::default()
            }),
            changes: Arc::new(ChangeSet {
                explosions: vec![ExplosionEvent {
                    center: [8.0, 64.0, 8.0],
                    radius: 4.0,
                    blocks: 1,
                    knockback,
                }],
                ..Default::default()
            }),
            times: Default::default(),
        }
    };
    let (tx, mut rx) = broadcast::channel(1);
    tx.send(output(1, [(delivered, (1, 1))].into())).unwrap();
    close_skipped_knockback(
        &server,
        &rx.recv().await.unwrap(),
        &mut seen,
        &mut viewers,
        &mut connections,
    );
    tx.send(output(2, [(delivered, (1, 1)), (lost, (1, 2))].into()))
        .unwrap();
    tx.send(output(
        3,
        [(delivered, (1, 1)), (lost, (2, 3)), (current, (1, 3))].into(),
    ))
    .unwrap();
    assert!(matches!(
        rx.recv().await,
        Err(broadcast::error::RecvError::Lagged(1))
    ));
    let retained = rx.recv().await.unwrap();
    close_skipped_knockback(
        &server,
        &retained,
        &mut seen,
        &mut viewers,
        &mut connections,
    );
    assert_eq!(
        closers[0].try_recv().unwrap(),
        "skipped-explosion-knockback"
    );
    assert!(server.connections.get(lost).is_none());
    for (index, id) in [(1, current), (2, delivered)] {
        assert!(server.connections.get(id).is_some());
        assert!(matches!(
            closers[index].try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        server.connections.remove(id);
    }
    server.hub.shutdown();
    std::fs::remove_dir_all(dir).unwrap();
}

/// The pump also skips an output that waited past its queue budget while a
/// newer one is queued, before the broadcast overflows. Accepted policy, the
/// same as after `Lagged`: a recipient whose impulse was in the skipped output
/// is closed even though the next output knocks it again; recipients whose
/// impulses all arrive stay open; every viewer reconciles from the next
/// snapshot.
#[tokio::test]
async fn stale_skip_closes_only_recipients_of_skipped_knockback() {
    let (server, dir) = fixture();
    let server = Arc::new(server);
    let (lost, _lost_rx) = register(&server, "Lost", false);
    let (current, _current_rx) = register(&server, "Current", false);
    let (delivered, _delivered_rx) = register(&server, "Delivered", false);
    let mut closers = Vec::new();
    for id in [lost, current, delivered] {
        let (close, closed) = tokio::sync::oneshot::channel();
        server.connections.inner.lock().closers.insert(id, close);
        closers.push(closed);
    }
    let (mut connections, _) = server.connections.routing_view();
    let mut viewers: HashMap<ConnectionId, crate::plan::ViewerState> = [lost, current, delivered]
        .into_iter()
        .map(|id| {
            (
                id,
                crate::plan::ViewerState::new(id, EntityId(id.0 as i32), [8.0, 64.0, 8.0]),
            )
        })
        .collect();
    let mut seen = std::collections::BTreeMap::new();
    let output = |tick,
                  epochs: std::collections::BTreeMap<ConnectionId, (u64, u64)>,
                  published_at: std::time::Instant| {
        let knockback = epochs
            .iter()
            .filter(|(_, (_, last_tick))| *last_tick == tick)
            .map(|(id, _)| (EntityId(id.0 as i32), [0.1, 0.2, 0.3]))
            .collect();
        TickOutput {
            tick,
            snapshot: Arc::new(WorldSnapshot {
                tick,
                player_knockback_ticks: Arc::new(epochs),
                ..Default::default()
            }),
            changes: Arc::new(ChangeSet {
                explosions: vec![ExplosionEvent {
                    center: [8.0, 64.0, 8.0],
                    radius: 4.0,
                    blocks: 1,
                    knockback,
                }],
                ..Default::default()
            }),
            times: mc_core::boundary::TickTimes {
                published_at: Some(published_at),
                ..Default::default()
            },
        }
    };
    // A wide window: nothing overflows, only the queue budget can skip.
    let (tx, mut rx) = broadcast::channel(8);
    let now = std::time::Instant::now();
    tx.send(output(1, [(delivered, (1, 1))].into(), now))
        .unwrap();
    let first = next_publication(&server, &mut rx, &mut viewers)
        .await
        .unwrap();
    assert_eq!(first.tick, 1);
    close_skipped_knockback(&server, &first, &mut seen, &mut viewers, &mut connections);
    let skipped_before = server.dashboard.snapshot().pump_skipped_ticks;
    let waited = now
        .checked_sub(PUBLISH_QUEUE_BUDGET + std::time::Duration::from_millis(1))
        .unwrap();
    tx.send(output(
        2,
        [(delivered, (1, 1)), (lost, (1, 2))].into(),
        waited,
    ))
    .unwrap();
    tx.send(output(
        3,
        [(delivered, (1, 1)), (lost, (2, 3)), (current, (1, 3))].into(),
        std::time::Instant::now(),
    ))
    .unwrap();
    let retained = next_publication(&server, &mut rx, &mut viewers)
        .await
        .unwrap();
    assert_eq!(retained.tick, 3, "the stale output is skipped");
    assert_eq!(
        server.dashboard.snapshot().pump_skipped_ticks,
        skipped_before + 1
    );
    assert!(
        viewers.values().all(|viewer| viewer.skipped),
        "every viewer reconciles from the next snapshot"
    );
    close_skipped_knockback(
        &server,
        &retained,
        &mut seen,
        &mut viewers,
        &mut connections,
    );
    assert_eq!(
        closers[0].try_recv().unwrap(),
        "skipped-explosion-knockback"
    );
    assert!(server.connections.get(lost).is_none());
    for (index, id) in [(1, current), (2, delivered)] {
        assert!(server.connections.get(id).is_some());
        assert!(matches!(
            closers[index].try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        server.connections.remove(id);
    }
    server.hub.shutdown();
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn same_tick_dimension_switch_discards_only_old_world_knockback() {
    dimension_switch_knockback(false).await;
    dimension_switch_knockback(true).await;
}

async fn dimension_switch_knockback(destination_impulse: bool) {
    use mc_core::boundary::PlayerChange;
    use mc_core::dimensions::Dimension;
    let (server, dir) = fixture();
    let server = Arc::new(server);
    let (id, mut reliable_rx) = register(&server, "Traveler", false);
    let (close, mut closed) = tokio::sync::oneshot::channel();
    server.connections.inner.lock().closers.insert(id, close);
    let (other, mut other_rx) = register(&server, "Witness", false);
    let (mut connections, _) = server.connections.routing_view();
    let mut viewers = HashMap::from([(
        id,
        crate::plan::ViewerState::new(id, EntityId(81), [8.0, 64.0, 8.0]),
    )]);
    viewers.insert(
        other,
        crate::plan::ViewerState::new(other, EntityId(82), [8.0, 64.0, 8.0]),
    );
    let output = TickOutput {
        tick: 2,
        snapshot: Arc::new(WorldSnapshot {
            tick: 2,
            player_knockback_ticks: Arc::new([(id, (1, 2)), (other, (1, 2))].into()),
            dimensions: [(
                Dimension::Nether,
                TickOutput {
                    tick: 2,
                    snapshot: Arc::new(WorldSnapshot {
                        tick: 2,
                        dimension: Dimension::Nether,
                        ..Default::default()
                    }),
                    changes: Arc::new(ChangeSet {
                        explosions: if destination_impulse {
                            vec![ExplosionEvent {
                                center: [8.0, 64.0, 8.0],
                                radius: 4.0,
                                blocks: 1,
                                knockback: vec![(EntityId(81), [0.7, 0.8, 0.9])],
                            }]
                        } else {
                            Vec::new()
                        },
                        ..Default::default()
                    }),
                    times: Default::default(),
                },
            )]
            .into(),
            ..Default::default()
        }),
        changes: Arc::new(ChangeSet {
            explosions: vec![ExplosionEvent {
                center: [8.0, 64.0, 8.0],
                radius: 4.0,
                blocks: 1,
                knockback: vec![
                    (EntityId(81), [0.1, 0.2, 0.3]),
                    (EntityId(82), [0.3, 0.2, 0.1]),
                ],
            }],
            ..Default::default()
        }),
        times: Default::default(),
    };
    route_outbox_batches(
        &server,
        &mut viewers,
        &mut connections,
        vec![(
            id,
            vec![PlayerChange::DimensionChanged {
                teleport_id: 2,
                dimension: Dimension::Nether,
                position: [8.0, 64.0, 8.0],
                yaw: 0.0,
                pitch: 0.0,
                game_mode: 0,
                portal_cooldown: 10,
            }],
        )],
        &mut Arc::from([]),
    );
    assert_eq!(viewers[&id].dimension, Dimension::Nether);
    assert!(!reliable_rx.is_empty(), "dimension reset was not routed");
    close_skipped_knockback(
        &server,
        &output,
        &mut Default::default(),
        &mut viewers,
        &mut connections,
    );
    assert!(matches!(
        closed.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    assert!(viewers.contains_key(&id));
    assert!(connections.contains_key(&id));
    assert!(server.connections.get(id).is_some());
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    publish_and_route(
        &server,
        &pool,
        &mut viewers,
        &output,
        &mut crate::plan::ChunkPacketCache::new(),
        &mut connections,
    )
    .await;
    assert_eq!(
        received_impulses(&mut reliable_rx).await,
        if destination_impulse {
            vec![Some([0.7, 0.8, 0.9])]
        } else {
            Vec::new()
        },
        "only destination-world knockback may cross the dimension reset"
    );
    assert_eq!(
        received_impulses(&mut other_rx).await,
        vec![Some([0.3, 0.2, 0.1])]
    );
    // A later impulse in the new world still reaches the traveler.
    let next = TickOutput {
        tick: 3,
        snapshot: Arc::new(WorldSnapshot {
            tick: 3,
            dimension: Dimension::Nether,
            ..Default::default()
        }),
        changes: Arc::new(ChangeSet {
            explosions: vec![ExplosionEvent {
                center: [8.0, 64.0, 8.0],
                radius: 4.0,
                blocks: 1,
                knockback: vec![(EntityId(81), [0.4, 0.5, 0.6])],
            }],
            ..Default::default()
        }),
        times: Default::default(),
    };
    publish_and_route(
        &server,
        &pool,
        &mut viewers,
        &next,
        &mut crate::plan::ChunkPacketCache::new(),
        &mut connections,
    )
    .await;
    assert_eq!(
        received_impulses(&mut reliable_rx).await,
        vec![Some([0.4, 0.5, 0.6])]
    );
    reliable_rx.close();
    server.hub.shutdown();
    std::fs::remove_dir_all(dir).unwrap();
}

async fn publication_command(core: &Core, text: String) {
    let (command, completed) = mc_core::command_console::ConsoleCommand::new(text);
    assert!(core.submit(mc_core::boundary::PlayerInput::ConsoleCommand(command)));
    tokio::time::timeout(std::time::Duration::from_secs(60), completed)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn published_knockback_epochs_count_ticks_and_fence_delayed_queue_registration() {
    let (server, dir) = fixture();
    let server = Arc::new(server);
    let mut outputs = server.hub.subscribe();
    let connection = server.connections.joining_id();
    let (entity, position) = join_impulse_observer(&server, &mut outputs, connection).await;
    publication_command(&server.hub, "tick freeze".into()).await;
    for _ in 0..2 {
        publication_command(
            &server.hub,
            format!(
                "summon minecraft:tnt {} {} {} {{fuse:1s,explosion_power:1f}}",
                position[0] + 1.0,
                position[1] + 1.0,
                position[2]
            ),
        )
        .await;
    }
    publication_command(&server.hub, "tick step 1".into()).await;
    let first_impulse = tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            let out = outputs.recv().await.unwrap();
            server.hub.try_drain_outbox_through(out.tick);
            let hits = out
                .changes
                .explosions
                .iter()
                .filter(|explosion| {
                    explosion
                        .knockback
                        .iter()
                        .any(|&(id, push)| id == entity && push != [0.0; 3])
                })
                .count();
            if !out.changes.explosions.is_empty() {
                assert_eq!(hits, 2, "both frozen TNT bodies explode in one step");
                assert_eq!(
                    out.snapshot.player_knockback_ticks[&connection],
                    (1, out.tick)
                );
                break out;
            }
        }
    })
    .await
    .unwrap();
    let quiet = outputs.recv().await.unwrap();
    assert!(quiet.changes.explosions.is_empty());
    assert_eq!(
        quiet.snapshot.player_knockback_ticks[&connection],
        (1, first_impulse.tick)
    );
    let mut delayed = vec![first_impulse];
    let mut position = quiet
        .snapshot
        .entities
        .iter()
        .find(|e| e.id == entity)
        .unwrap()
        .position;
    for count in 2..=3 {
        publication_command(
            &server.hub,
            format!(
                "summon minecraft:tnt {} {} {} {{fuse:1s,explosion_power:1f}}",
                position[0] + 1.0,
                position[1] + 1.0,
                position[2]
            ),
        )
        .await;
        publication_command(&server.hub, "tick step 1".into()).await;
        let out = next_impulse(&server.hub, &mut outputs, connection, count).await;
        position = out
            .snapshot
            .entities
            .iter()
            .find(|e| e.id == entity)
            .unwrap()
            .position;
        delayed.push(out);
    }
    let quiet = outputs.recv().await.unwrap();
    assert_eq!(server.hub.published_knockback_count(connection), 3);
    assert!(quiet.changes.explosions.is_empty());
    let (reliable_tx, mut reliable_rx) = mpsc::channel(64);
    let (world_tx, _world_rx) = world_queue(8, WORLD_QUEUE_BYTES);
    server
        .connections
        .register(
            connection,
            ConnectionHandles::new(reliable_tx, world_tx, crate::plan::CHUNKS_PER_TICK),
            || server.hub.published_knockback_count(connection),
        )
        .unwrap();
    let (close, mut closed) = tokio::sync::oneshot::channel();
    server
        .connections
        .inner
        .lock()
        .closers
        .insert(connection, close);
    let (mut connections, _) = server.connections.routing_view();
    let mut viewers = HashMap::from([(
        connection,
        crate::plan::ViewerState::new(connection, entity, position),
    )]);
    let mut seen = std::collections::BTreeMap::new();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let mut cache = crate::plan::ChunkPacketCache::new();
    // The Core is already at count 3 when queues register, but the pump's
    // first processed outputs still contain counts 1 and 3.
    for out in [&delayed[0], &delayed[2]] {
        close_skipped_knockback(&server, out, &mut seen, &mut viewers, &mut connections);
        assert!(server.connections.get(connection).is_some());
        assert_eq!(seen[&connection], 3, "old outputs must not lower the fence");
        publish_and_route(
            &server,
            &pool,
            &mut viewers,
            out,
            &mut cache,
            &mut connections,
        )
        .await;
        while !reliable_rx.is_empty() {
            let frame = receive(&mut reliable_rx).await;
            assert_ne!(
                frame.id,
                ClientboundExplode::ID,
                "pre-registration impulses must not be delivered to the new client"
            );
        }
    }
    close_skipped_knockback(&server, &quiet, &mut seen, &mut viewers, &mut connections);
    assert!(
        server.connections.get(connection).is_some(),
        "no post-registration impulse was lost"
    );
    assert!(matches!(
        closed.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));

    publication_command(
        &server.hub,
        format!(
            "summon minecraft:tnt {} {} {} {{fuse:1s,explosion_power:1f}}",
            position[0] + 1.0,
            position[1] + 1.0,
            position[2]
        ),
    )
    .await;
    publication_command(&server.hub, "tick step 1".into()).await;
    let _skipped = next_impulse(&server.hub, &mut outputs, connection, 4).await;
    let retained = outputs.recv().await.unwrap();
    assert!(retained.changes.explosions.is_empty());
    close_skipped_knockback(
        &server,
        &retained,
        &mut seen,
        &mut viewers,
        &mut connections,
    );
    assert_eq!(closed.try_recv().unwrap(), "skipped-explosion-knockback");
    server.hub.shutdown();
    std::fs::remove_dir_all(dir).unwrap();
}

async fn next_impulse(
    core: &Core,
    outputs: &mut broadcast::Receiver<TickOutput>,
    connection: ConnectionId,
    count: u64,
) -> TickOutput {
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            let out = outputs.recv().await.unwrap();
            core.try_drain_outbox_through(out.tick);
            if !out.changes.explosions.is_empty() {
                assert_eq!(
                    out.snapshot.player_knockback_ticks[&connection],
                    (count, out.tick)
                );
                return out;
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn first_processed_output_after_registration_still_closes_a_skipped_impulse() {
    let (server, dir) = fixture();
    let server = Arc::new(server);
    let (id, _rx) = register(&server, "FirstLagged", false);
    let (close, mut closed) = tokio::sync::oneshot::channel();
    server.connections.inner.lock().closers.insert(id, close);
    let (mut connections, _) = server.connections.routing_view();
    let mut viewers = HashMap::from([(
        id,
        crate::plan::ViewerState::new(id, EntityId(id.0 as i32), [8.0, 64.0, 8.0]),
    )]);
    let (tx, mut rx) = broadcast::channel(1);
    for tick in 1..=2 {
        tx.send(TickOutput {
            tick,
            snapshot: Arc::new(WorldSnapshot {
                tick,
                player_knockback_ticks: Arc::new([(id, (tick, tick))].into()),
                ..Default::default()
            }),
            changes: Arc::new(ChangeSet {
                explosions: vec![ExplosionEvent {
                    center: [8.0, 64.0, 8.0],
                    radius: 4.0,
                    blocks: 0,
                    knockback: vec![(EntityId(id.0 as i32), [0.1, 0.2, 0.3])],
                }],
                ..Default::default()
            }),
            times: Default::default(),
        })
        .unwrap();
    }
    assert!(matches!(
        rx.recv().await,
        Err(broadcast::error::RecvError::Lagged(1))
    ));
    close_skipped_knockback(
        &server,
        &rx.recv().await.unwrap(),
        &mut Default::default(),
        &mut viewers,
        &mut connections,
    );
    assert_eq!(closed.try_recv().unwrap(), "skipped-explosion-knockback");
    server.hub.shutdown();
    std::fs::remove_dir_all(dir).unwrap();
}

async fn join_impulse_observer(
    server: &Arc<EdgeServer>,
    outputs: &mut broadcast::Receiver<TickOutput>,
    connection: ConnectionId,
) -> (EntityId, [f64; 3]) {
    use mc_core::boundary::PlayerInput;
    let uuid = uuid::Uuid::from_u128(80);
    assert!(server.hub.submit(PlayerInput::SetViewDistance {
        connection,
        view_distance: 0,
    }));
    assert!(server.hub.submit(PlayerInput::Join {
        connection,
        uuid,
        name: "Observer".into(),
        permission_level: 4,
    }));
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            let out = outputs.recv().await.unwrap();
            server.hub.try_drain_outbox_through(out.tick);
            if let Some(player) = out.snapshot.entities.iter().find(|e| e.uuid == uuid)
                && out.snapshot.chunks.iter().any(|chunk| {
                    chunk.pos.x == (player.position[0].floor() as i32).div_euclid(16)
                        && chunk.pos.z == (player.position[2].floor() as i32).div_euclid(16)
                })
            {
                break (player.id, player.position);
            }
        }
    })
    .await
    .unwrap()
}

async fn stepped_impulse(
    server: &Arc<EdgeServer>,
    outputs: &mut broadcast::Receiver<TickOutput>,
    connection: ConnectionId,
    position: [f64; 3],
    count: u64,
) -> TickOutput {
    publication_command(
        &server.hub,
        format!(
            "summon minecraft:tnt {} {} {} {{fuse:1s,explosion_power:1f}}",
            position[0] + 1.0,
            position[1] + 1.0,
            position[2]
        ),
    )
    .await;
    publication_command(&server.hub, "tick step 1".into()).await;
    next_impulse(&server.hub, outputs, connection, count).await
}

async fn received_impulses(
    reliable_rx: &mut mpsc::Receiver<bytes::Bytes>,
) -> Vec<Option<[f64; 3]>> {
    let mut impulses = Vec::new();
    while !reliable_rx.is_empty() {
        let frame = receive(reliable_rx).await;
        if frame.id == ClientboundExplode::ID {
            let blast = ClientboundExplode::decode(&mut &frame.body[..]).unwrap();
            impulses.push(blast.knockback.map(|push| [push.x, push.y, push.z]));
        }
    }
    impulses
}

#[tokio::test]
async fn publication_before_registration_atomically_fences_the_impulse() {
    synchronized_registration_publication(true).await;
}

#[tokio::test]
async fn registration_before_publication_delivers_the_impulse_exactly_once() {
    synchronized_registration_publication(false).await;
}

async fn synchronized_registration_publication(publication_first: bool) {
    let (server, dir) = fixture();
    let server = Arc::new(server);
    let mut outputs = server.hub.subscribe();
    let connection = server.connections.joining_id();
    let (entity, position) = join_impulse_observer(&server, &mut outputs, connection).await;
    publication_command(&server.hub, "tick freeze".into()).await;
    // Both orders start with one historical impulse. A first-output/zero
    // baseline must not mistake this history for a post-registration loss.
    let historical = stepped_impulse(&server, &mut outputs, connection, position, 1).await;
    let position = historical
        .snapshot
        .entities
        .iter()
        .find(|e| e.id == entity)
        .unwrap()
        .position;
    let (reliable_tx, mut reliable_rx) = mpsc::channel(64);
    let (world_tx, _world_rx) = world_queue(8, WORLD_QUEUE_BYTES);
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(0);
    let registering = Arc::clone(&server);
    let registration = tokio::task::spawn_blocking(move || {
        registering.connections.register(
            connection,
            ConnectionHandles::new(reliable_tx, world_tx, crate::plan::CHUNKS_PER_TICK),
            || {
                // Pause inside the real registry critical section, on either
                // side of its canonical-count read (the admission boundary).
                let count = if publication_first {
                    None
                } else {
                    Some(registering.hub.published_knockback_count(connection))
                };
                entered_tx.send(count).unwrap();
                release_rx.recv().unwrap();
                count.unwrap_or_else(|| registering.hub.published_knockback_count(connection))
            },
        )
    });
    let captured = tokio::time::timeout(std::time::Duration::from_secs(60), entered_rx)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(captured, (!publication_first).then_some(1));
    assert!(
        server.connections.inner.try_lock().is_none(),
        "the registration must remain inside its critical section"
    );
    // This actual native publication completes while registration is paused:
    // the publisher must not enter the edge registry or invert the lock order.
    let boundary = stepped_impulse(&server, &mut outputs, connection, position, 2).await;
    release_tx.send(()).unwrap();
    registration.await.unwrap().unwrap();
    let baseline = if publication_first { 2 } else { 1 };
    assert_eq!(
        server
            .connections
            .get(connection)
            .unwrap()
            .knockback_baseline,
        baseline
    );
    let (close, mut closed) = tokio::sync::oneshot::channel();
    server
        .connections
        .inner
        .lock()
        .closers
        .insert(connection, close);
    let (mut connections, _) = server.connections.routing_view();
    let position = boundary
        .snapshot
        .entities
        .iter()
        .find(|e| e.id == entity)
        .unwrap()
        .position;
    let mut viewers = HashMap::from([(
        connection,
        crate::plan::ViewerState::new(connection, entity, position),
    )]);
    let mut seen = std::collections::BTreeMap::new();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let mut cache = crate::plan::ChunkPacketCache::new();
    close_skipped_knockback(
        &server,
        &boundary,
        &mut seen,
        &mut viewers,
        &mut connections,
    );
    assert!(
        server.connections.get(connection).is_some(),
        "no impulse was skipped after admission"
    );
    publish_and_route(
        &server,
        &pool,
        &mut viewers,
        &boundary,
        &mut cache,
        &mut connections,
    )
    .await;
    let received = received_impulses(&mut reliable_rx).await;
    let expected: Vec<_> = if publication_first {
        Vec::new()
    } else {
        vec![Some(
            boundary
                .changes
                .explosions
                .iter()
                .flat_map(|blast| &blast.knockback)
                .find(|&&(id, _)| id == entity)
                .unwrap()
                .1,
        )]
    };
    assert_eq!(
        received, expected,
        "the boundary impulse must be fenced or delivered once, never replayed or lost"
    );
    let quiet = outputs.recv().await.unwrap();
    assert!(quiet.changes.explosions.is_empty());
    close_skipped_knockback(&server, &quiet, &mut seen, &mut viewers, &mut connections);
    publish_and_route(
        &server,
        &pool,
        &mut viewers,
        &quiet,
        &mut cache,
        &mut connections,
    )
    .await;
    assert!(
        received_impulses(&mut reliable_rx).await.is_empty(),
        "the next quiet publication must not duplicate the impulse"
    );
    assert!(matches!(
        closed.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    // A fresh post-registration impulse must still be delivered in both
    // orders; registration is not a blanket suppression of later impulses.
    let next = stepped_impulse(&server, &mut outputs, connection, position, 3).await;
    close_skipped_knockback(&server, &next, &mut seen, &mut viewers, &mut connections);
    publish_and_route(
        &server,
        &pool,
        &mut viewers,
        &next,
        &mut cache,
        &mut connections,
    )
    .await;
    let expected = next
        .changes
        .explosions
        .iter()
        .flat_map(|blast| &blast.knockback)
        .find(|&&(id, _)| id == entity)
        .unwrap()
        .1;
    assert_eq!(
        received_impulses(&mut reliable_rx).await,
        vec![Some(expected)]
    );
    assert!(matches!(
        closed.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    server.hub.shutdown();
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn dimension_reset_fences_queued_old_world_explosions_at_real_writer() {
    use mc_core::boundary::PlayerChange;
    use mc_core::dimensions::Dimension;
    let (server, dir) = fixture();
    let server = Arc::new(server);
    let id = server.connections.joining_id();
    let (reliable_tx, mut reliable_rx) = mpsc::channel(64);
    let (world_tx, mut world_rx) = world_queue(4, WORLD_QUEUE_BYTES);
    server
        .connections
        .register(
            id,
            ConnectionHandles::new(reliable_tx, world_tx.clone(), crate::plan::CHUNKS_PER_TICK),
            || 0,
        )
        .unwrap();
    let (mut connections, _) = server.connections.routing_view();
    let mut viewers = HashMap::from([(
        id,
        crate::plan::ViewerState::new(id, EntityId(81), [8.0, 64.0, 8.0]),
    )]);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let mut cache = crate::plan::ChunkPacketCache::new();
    let old = TickOutput {
        tick: 1,
        snapshot: Arc::new(WorldSnapshot {
            tick: 1,
            ..Default::default()
        }),
        changes: Arc::new(ChangeSet {
            explosions: vec![ExplosionEvent {
                center: [8.0, 64.0, 8.0],
                radius: 4.0,
                blocks: 1,
                knockback: Vec::new(),
            }],
            ..Default::default()
        }),
        times: Default::default(),
    };
    publish_and_route(
        &server,
        &pool,
        &mut viewers,
        &old,
        &mut cache,
        &mut connections,
    )
    .await;
    while reliable_rx.try_recv().is_ok() {}
    let mut old_frame = None;
    while let Ok(frame) = world_rx.try_recv() {
        let (tx, mut rx) = mpsc::channel(1);
        tx.try_send(frame.clone()).unwrap();
        if receive(&mut rx).await.id == ClientboundExplode::ID {
            old_frame = Some(frame);
        }
    }
    let old_frame = old_frame.expect("queued visual explosion");
    for _ in 0..4 {
        world_tx.try_send(old_frame.clone()).unwrap();
    }
    assert_eq!(world_tx.capacity(), 0, "old world queue must be full");
    route_outbox_batches(
        &server,
        &mut viewers,
        &mut connections,
        vec![(
            id,
            vec![PlayerChange::DimensionChanged {
                teleport_id: 2,
                dimension: Dimension::Nether,
                position: [8.0, 64.0, 8.0],
                yaw: 0.0,
                pitch: 0.0,
                game_mode: 0,
                portal_cooldown: 10,
            }],
        )],
        &mut Arc::from([]),
    );
    let destination = TickOutput {
        tick: 2,
        snapshot: Arc::new(WorldSnapshot {
            tick: 2,
            dimension: Dimension::Nether,
            ..Default::default()
        }),
        changes: Arc::new(ChangeSet {
            explosions: vec![ExplosionEvent {
                center: [18.0, 64.0, 8.0],
                radius: 4.0,
                blocks: 1,
                knockback: vec![(EntityId(81), [0.7, 0.8, 0.9])],
            }],
            ..Default::default()
        }),
        times: Default::default(),
    };
    publish_and_route(
        &server,
        &pool,
        &mut viewers,
        &destination,
        &mut cache,
        &mut connections,
    )
    .await;
    let reliable_count = reliable_rx.len();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client_socket = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (server_socket, _) = listener.accept().await.unwrap();
    let mut connection = crate::connection::Connection::new(server_socket);
    connection.set_compression(mc_protocol::frame::Compression::Threshold(256));
    let (_, writer) = connection.into_play_halves();
    let task = tokio::spawn(play_writer(
        writer,
        Arc::clone(&server.net_counters),
        Arc::clone(&server.connections),
        id,
        uuid::Uuid::nil(),
        PlayReceivers {
            reliable: reliable_rx,
            world: world_rx,
            dh: mpsc::channel(1).1,
            keepalive: mpsc::channel(1).1,
        },
    ));
    let mut client = crate::connection::Connection::new(client_socket);
    client.set_compression(mc_protocol::frame::Compression::Threshold(256));
    let mut respawned = false;
    let mut impulses = 0;
    for _ in 0..reliable_count {
        let frame =
            tokio::time::timeout(std::time::Duration::from_secs(5), client.read_frame_play())
                .await
                .unwrap()
                .unwrap();
        if frame.id == ClientboundRespawn::ID {
            respawned = true;
        }
        if frame.id == ClientboundExplode::ID {
            assert!(respawned);
            let blast = ClientboundExplode::decode(&mut &frame.body[..]).unwrap();
            assert_eq!(blast.center.x, 18.0, "old world explosion followed respawn");
            assert_eq!(
                blast.knockback.map(|v| [v.x, v.y, v.z]),
                Some([0.7, 0.8, 0.9])
            );
            impulses += 1;
        }
    }
    assert!(respawned);
    assert_eq!(
        impulses, 1,
        "same-tick destination impulse must be delivered"
    );
    // Receiving the reliable burst proves the writer has drained the full old
    // queue into either its byte stream (broken) or the discard fence (fixed).
    let mut body = Vec::new();
    ForgetLevelChunk {
        chunk_x: 77,
        chunk_z: 88,
    }
    .encode(&mut body)
    .unwrap();
    let mut frame = bytes::BytesMut::new();
    mc_protocol::frame::append_frame(
        &mut frame,
        ForgetLevelChunk::ID,
        &body,
        mc_protocol::frame::Compression::Threshold(256),
    )
    .unwrap();
    world_tx.try_send(frame.freeze()).unwrap();
    let frame = tokio::time::timeout(std::time::Duration::from_secs(5), client.read_frame_play())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        frame.id,
        ForgetLevelChunk::ID,
        "queued old world frame leaked after respawn"
    );
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    server.connections.remove(id);
    server.hub.shutdown();
    std::fs::remove_dir_all(dir).unwrap();
}
