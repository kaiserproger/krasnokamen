//! One-shot knockback survives exhausted world credit and a refused world send.
use super::*;
use mc_core::boundary::{ChangeSet, EntityId, ExplosionEvent, TickOutput, WorldSnapshot};

#[tokio::test]
async fn explosions_deliver_or_explicitly_close_under_queue_pressure() {
    // A closed world receiver models a competing producer taking the planned
    // capacity: sends fail even though the credit read still reports free slots.
    for world_pressure in ["slots", "bytes", "send-refused"] {
        for reliable_full in [false, true] {
            let (server, dir) = fixture();
            let server = Arc::new(server);
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
                    closed.await.unwrap(),
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
            server.hub.shutdown();
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
}
