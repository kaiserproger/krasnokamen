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
                    ConnectionHandles {
                        reliable_tx,
                        world_tx,
                    },
                )
                .unwrap();
            let (close, mut closed) = tokio::sync::oneshot::channel();
            server.connections.inner.lock().closers.insert(id, close);
            let (mut connections, _) = server.connections.routing_snapshot();
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
                ConnectionHandles {
                    reliable_tx,
                    world_tx,
                },
            )
            .unwrap();
        let (close, mut closed) = tokio::sync::oneshot::channel();
        server.connections.inner.lock().closers.insert(id, close);
        let (mut connections, _) = server.connections.routing_snapshot();
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
    let (mut connections, _) = server.connections.routing_snapshot();
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

#[tokio::test]
async fn same_tick_dimension_switch_explicitly_closes_unprojected_knockback() {
    use mc_core::boundary::PlayerChange;
    use mc_core::dimensions::Dimension;
    let (server, dir) = fixture();
    let server = Arc::new(server);
    let (id, mut reliable_rx) = register(&server, "Traveler", false);
    let (close, mut closed) = tokio::sync::oneshot::channel();
    server.connections.inner.lock().closers.insert(id, close);
    let (mut connections, _) = server.connections.routing_snapshot();
    let mut viewers = HashMap::from([(
        id,
        crate::plan::ViewerState::new(id, EntityId(81), [8.0, 64.0, 8.0]),
    )]);
    let output = TickOutput {
        tick: 2,
        snapshot: Arc::new(WorldSnapshot {
            tick: 2,
            player_knockback_ticks: Arc::new([(id, (1, 2))].into()),
            dimensions: [(
                Dimension::Nether,
                TickOutput {
                    tick: 2,
                    snapshot: Arc::new(WorldSnapshot {
                        tick: 2,
                        dimension: Dimension::Nether,
                        ..Default::default()
                    }),
                    changes: Arc::new(ChangeSet::default()),
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
                knockback: vec![(EntityId(81), [0.1, 0.2, 0.3])],
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
    assert_eq!(
        closed.try_recv().unwrap(),
        "dimension-switch-explosion-knockback"
    );
    assert!(!viewers.contains_key(&id));
    assert!(!connections.contains_key(&id));
    assert!(server.connections.get(id).is_none());
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
async fn published_knockback_epochs_count_ticks_not_explosions_and_survive_quiet_ticks() {
    use mc_core::boundary::PlayerInput;
    let (server, dir) = fixture();
    let mut outputs = server.hub.subscribe();
    let connection = ConnectionId(80);
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
    let (entity, position) = tokio::time::timeout(std::time::Duration::from_secs(60), async {
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
    .unwrap();
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
    let impulse_tick = tokio::time::timeout(std::time::Duration::from_secs(60), async {
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
                break out.tick;
            }
        }
    })
    .await
    .unwrap();
    let quiet = outputs.recv().await.unwrap();
    assert!(quiet.changes.explosions.is_empty());
    assert_eq!(
        quiet.snapshot.player_knockback_ticks[&connection],
        (1, impulse_tick)
    );
    server.hub.shutdown();
    std::fs::remove_dir_all(dir).unwrap();
}
