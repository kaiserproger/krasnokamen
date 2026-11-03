//! A public shelf update against the real publish, routing and Play writer.
//!
//! A reliable frame is written before any world frame, and `loaded_chunks`
//! records chunks planned, not delivered. The client applies a block-entity
//! update only to a chunk it already holds, so an update that overtakes a
//! queued chunk is dropped and the older chunk then lands over it.

use super::*;
use mc_core::boundary::{BlockPos, EntityId, PlayerChange, SnapshotChunk, WorldSnapshot};
use mc_data::block_entity_kinds::BlockEntityKind;
use mc_nbt::{ListTag, Tag, tag_type};
use mc_world::chunk::{Chunk, ChunkPos, LightSection, SectionLight};

const SHELF: BlockPos = BlockPos { x: 2, y: 64, z: 3 };

fn shelf_tag(items: usize) -> Tag {
    let elements: Vec<_> = (0..items)
        .map(|slot| {
            Tag::Compound(vec![
                ("Slot".into(), Tag::Byte(slot as i8)),
                ("id".into(), Tag::String("minecraft:diamond".into())),
                ("count".into(), Tag::Int(1)),
            ])
        })
        .collect();
    Tag::Compound(vec![
        ("id".into(), Tag::String("minecraft:shelf".into())),
        ("x".into(), Tag::Int(SHELF.x)),
        ("y".into(), Tag::Int(SHELF.y)),
        ("z".into(), Tag::Int(SHELF.z)),
        (
            "Items".into(),
            Tag::List(ListTag {
                element_type: if elements.is_empty() {
                    tag_type::END
                } else {
                    tag_type::COMPOUND
                },
                elements,
            }),
        ),
        ("align_items_to_bottom".into(), Tag::Byte(0)),
    ])
}

fn item_count(tag: &Tag) -> usize {
    let Tag::Compound(fields) = tag else { return 0 };
    fields
        .iter()
        .find(|(key, _)| key == "Items")
        .map_or(0, |(_, items)| match items {
            Tag::List(list) => list.elements.len(),
            _ => 0,
        })
}

/// An empty-shelf chunk; later snapshots clone it, so, as in the core's
/// copy-on-write chunks, an update that only changes the shelf shares every
/// light allocation with the one before.
fn shelf_base(server: &EdgeServer) -> Chunk {
    let id = |name: &str| mc_data::Identifier::parse(name).unwrap();
    let air = server.wire.blocks.block(&id("minecraft:air")).unwrap().default;
    let shelf = server
        .wire
        .blocks
        .block(&id("minecraft:oak_shelf"))
        .unwrap()
        .default;
    let mut chunk = Chunk::empty(ChunkPos { x: 0, z: 0 }, air, id("minecraft:plains"));
    chunk.section_lights.fill(SectionLight {
        block: Some(LightSection::uniform(0)),
        sky: Some(LightSection::uniform(0xFF)),
    });
    chunk
        .set_block(SHELF.x as u8, SHELF.y, SHELF.z as u8, shelf)
        .unwrap();
    chunk
}

fn shelf_output(base: &Chunk, tick: u64, revision: u64, items: usize) -> TickOutput {
    let mut chunk = base.clone();
    let mut bytes = Vec::new();
    mc_nbt::write_network(&mut bytes, &shelf_tag(items)).unwrap();
    chunk.block_entities.insert(SHELF, bytes);
    TickOutput {
        tick,
        snapshot: Arc::new(WorldSnapshot {
            tick,
            day_time: 1000,
            day_time_rate: 1.0,
            chunks: Arc::new(vec![SnapshotChunk {
                pos: ChunkPos { x: 0, z: 0 },
                revision,
                chunk: Arc::new(chunk),
            }]),
            ..Default::default()
        }),
        changes: Arc::new(ChangeSet::default()),
        times: Default::default(),
    }
}

/// What a vanilla client holds of the shelf: block-entity updates apply only
/// to a held chunk, and a chunk replaces the block entities it carries.
#[derive(Default)]
struct ClientShelf {
    chunk_held: bool,
    chunks_received: usize,
    items: Option<usize>,
    updates_dropped: usize,
    /// Frames in arrival order, for failure messages.
    arrivals: Vec<String>,
}

impl ClientShelf {
    fn receive(&mut self, id: i32, body: &[u8]) {
        if id == LevelChunkWithLight::ID {
            let chunk = LevelChunkWithLight::decode(&mut &body[..]).unwrap();
            self.chunk_held = true;
            self.chunks_received += 1;
            self.items = chunk
                .block_entities
                .iter()
                .find(|record| record.packed_xz == 0x23 && record.y == 64)
                .map(|record| item_count(&record.nbt));
            self.arrivals.push(format!("chunk{{items: {:?}}}", self.items));
        } else if id == BlockUpdate::ID {
            let update = BlockUpdate::decode(&mut &body[..]).unwrap();
            assert_eq!(update.position, pack_block_pos(SHELF.x, SHELF.y, SHELF.z));
            self.arrivals.push("block".into());
        } else if id == ClientboundBlockEntityData::ID {
            let update = ClientboundBlockEntityData::decode(&mut &body[..]).unwrap();
            assert_eq!(update.position, pack_block_pos(SHELF.x, SHELF.y, SHELF.z));
            let items = item_count(&update.nbt);
            if self.chunk_held {
                self.items = Some(items);
                self.arrivals.push(format!("update{{items: {items}}}"));
            } else {
                self.updates_dropped += 1;
                self.arrivals
                    .push(format!("update{{items: {items}}} dropped: no chunk"));
            }
        } else {
            self.arrivals.push(format!("frame {id:#x}"));
        }
    }

    fn receive_framed(&mut self, framed: &bytes::Bytes) {
        let mut buf = bytes::BytesMut::from(&framed[..]);
        while !buf.is_empty() {
            let frame = mc_protocol::frame::try_decode_frame(
                &mut buf,
                mc_protocol::frame::Compression::Threshold(256),
            )
            .unwrap()
            .unwrap();
            self.receive(frame.id, &frame.body);
        }
    }
}

struct Wire {
    server: Arc<EdgeServer>,
    dir: std::path::PathBuf,
    id: ConnectionId,
    reliable_rx: mpsc::Receiver<bytes::Bytes>,
    world_rx: mpsc::Receiver<bytes::Bytes>,
    viewers: HashMap<ConnectionId, crate::plan::ViewerState>,
    connections: HashMap<ConnectionId, ConnectionHandles>,
    pool: rayon::ThreadPool,
    cache: crate::plan::ChunkPacketCache,
    base: Chunk,
}

impl Wire {
    fn new() -> Self {
        let (mut server, dir) = fixture();
        // A window small enough that unseen chunks cannot starve a repair of
        // the held one within a tick's chunk budget.
        server.cfg.server.view_distance = 1;
        let server = Arc::new(server);
        let id = server.connections.joining_id();
        let (reliable_tx, reliable_rx) = mpsc::channel(64);
        let (world_tx, world_rx) = mpsc::channel(64);
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
        let (connections, _) = server.connections.routing_snapshot();
        let viewers = HashMap::from([(
            id,
            crate::plan::ViewerState::new(id, EntityId(1), [2.5, 65.0, 3.5]),
        )]);
        Self {
            base: shelf_base(&server),
            server,
            dir,
            id,
            reliable_rx,
            world_rx,
            viewers,
            connections,
            pool: rayon::ThreadPoolBuilder::new()
                .num_threads(1)
                .build()
                .unwrap(),
            cache: crate::plan::ChunkPacketCache::new(),
        }
    }

    /// Publish the world after tick `tick` left `items` on the shelf.
    async fn publish_shelf(&mut self, tick: u64, items: usize) {
        // Revisions are the tick numbers: every tick here changes the chunk.
        let output = shelf_output(&self.base, tick, tick, items);
        publish_and_route(
            &self.server,
            &self.pool,
            &mut self.viewers,
            &output,
            &mut self.cache,
            &mut self.connections,
        )
        .await;
    }

    /// The core's drained outbox for this tick, routed like the pump does.
    fn route_shelf_update(&mut self, items: usize) {
        let change = PlayerChange::BlockEntityData {
            position: SHELF,
            block_entity_type: BlockEntityKind::Shelf.protocol_id(),
            nbt: shelf_tag(items),
        };
        assert!(crate::server::pump::route_changes(
            &[change],
            self.viewers.get_mut(&self.id).unwrap(),
            &self.connections[&self.id],
            &self.server,
        ));
    }

    fn finish(self) {
        self.server.connections.remove(self.id);
        self.server.hub.shutdown();
        std::fs::remove_dir_all(self.dir).unwrap();
    }
}

#[tokio::test]
async fn shelf_update_cannot_be_lost_to_a_chunk_still_queued_for_the_client() {
    let mut wire = Wire::new();
    // The viewer is planned its chunk, but the frame waits in the world queue
    // (a slow link, or the join burst ahead of it).
    wire.publish_shelf(1, 0).await;
    assert!(!wire.world_rx.is_empty(), "the chunk frame is still queued");
    // Someone puts a diamond on the shelf; the update is routed before the
    // chunk frame has left the queue, then the world moves on.
    wire.route_shelf_update(1);
    wire.publish_shelf(2, 1).await;

    // Start the real writer only now, with both queues populated.
    let queued = wire.reliable_rx.len() + wire.world_rx.len();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client_socket = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (server_socket, _) = listener.accept().await.unwrap();
    let mut server_connection = crate::connection::Connection::new(server_socket);
    server_connection.set_compression(mc_protocol::frame::Compression::Threshold(256));
    let (_, writer) = server_connection.into_play_halves();
    let writer_task = tokio::spawn(play_writer(
        writer,
        Arc::clone(&wire.server.net_counters),
        Arc::clone(&wire.server.connections),
        wire.id,
        uuid::Uuid::nil(),
        PlayReceivers {
            reliable: std::mem::replace(&mut wire.reliable_rx, mpsc::channel(1).1),
            world: std::mem::replace(&mut wire.world_rx, mpsc::channel(1).1),
            dh: mpsc::channel(1).1,
            keepalive: mpsc::channel(1).1,
        },
    ));
    let mut client_connection = crate::connection::Connection::new(client_socket);
    client_connection.set_compression(mc_protocol::frame::Compression::Threshold(256));
    let mut client = ClientShelf::default();
    for _ in 0..queued {
        let frame = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            client_connection.read_frame_play(),
        )
        .await
        .unwrap()
        .unwrap();
        client.receive(frame.id, &frame.body);
    }
    writer_task.abort();
    assert!(writer_task.await.unwrap_err().is_cancelled());
    wire.finish();
    assert_eq!(
        client.items,
        Some(1),
        "the client must end with the diamond; wire order: {:?}",
        client.arrivals
    );
}

#[tokio::test]
async fn shelf_update_after_the_chunk_was_taken_by_the_writer_needs_no_resend() {
    let mut wire = Wire::new();
    wire.publish_shelf(1, 0).await;
    // The writer has taken every queued frame: reliable first, then world.
    let mut client = ClientShelf::default();
    while let Ok(frame) = wire.reliable_rx.try_recv() {
        client.receive_framed(&frame);
    }
    while let Ok(frame) = wire.world_rx.try_recv() {
        client.receive_framed(&frame);
    }
    assert_eq!(client.chunks_received, 1);
    assert_eq!(client.items, Some(0));

    wire.route_shelf_update(1);
    wire.publish_shelf(2, 1).await;
    while let Ok(frame) = wire.reliable_rx.try_recv() {
        client.receive_framed(&frame);
    }
    while let Ok(frame) = wire.world_rx.try_recv() {
        client.receive_framed(&frame);
    }
    wire.finish();
    assert_eq!(
        client.chunks_received, 1,
        "an update behind a drained world queue explains the revision: no resend"
    );
    assert_eq!(client.items, Some(1), "the valid update is applied");
    assert_eq!(client.updates_dropped, 0);
}

#[tokio::test]
async fn shelf_update_waits_for_block_credit_and_uses_the_newest_snapshot() {
    let mut wire = Wire::new();
    let mut client = ClientShelf::default();
    wire.publish_shelf(1, 0).await;
    while let Ok(frame) = wire.reliable_rx.try_recv() {
        client.receive_framed(&frame);
    }
    while let Ok(frame) = wire.world_rx.try_recv() {
        client.receive_framed(&frame);
    }

    let mut body = Vec::new();
    ForgetLevelChunk { chunk_x: 100, chunk_z: 100 }.encode(&mut body).unwrap();
    let mut dummy = bytes::BytesMut::new();
    mc_protocol::frame::append_frame(&mut dummy, ForgetLevelChunk::ID, &body,
        mc_protocol::frame::Compression::Threshold(256)).unwrap();
    let sender = &wire.connections[&wire.id].world_tx;
    while sender.capacity() > 1 {
        sender.try_send(dummy.clone().freeze()).unwrap();
    }
    wire.route_shelf_update(1);
    let mut output = shelf_output(&wire.base, 2, 2, 1);
    Arc::get_mut(&mut output.changes).unwrap().blocks.push(mc_core::boundary::BlockChange {
        position: SHELF,
        state: wire.base.get_block(2, 64, 3).unwrap().0,
    });
    publish_and_route(&wire.server, &wire.pool, &mut wire.viewers, &output,
        &mut wire.cache, &mut wire.connections).await;
    while let Ok(frame) = wire.reliable_rx.try_recv() {
        client.receive_framed(&frame);
    }
    while let Ok(frame) = wire.world_rx.try_recv() {
        client.receive_framed(&frame);
    }
    assert_eq!(client.items, Some(0), "one credit sends the block, not its entity");

    // A newer event and snapshot supersede the still-owed first entity update.
    wire.route_shelf_update(1);
    wire.publish_shelf(3, 2).await;
    while let Ok(frame) = wire.reliable_rx.try_recv() {
        client.receive_framed(&frame);
    }
    while let Ok(frame) = wire.world_rx.try_recv() {
        client.receive_framed(&frame);
    }
    wire.finish();
    assert_eq!(client.items, Some(2), "{:?}", client.arrivals);
    let update = client.arrivals.iter().position(|s| s == "update{items: 2}").unwrap();
    let block = client.arrivals.iter().rposition(|s| s == "block").unwrap();
    assert!(block < update, "block must precede its entity: {:?}", client.arrivals);
    assert!(!client.arrivals.iter().any(|s| s == "update{items: 1}"));
}

#[tokio::test]
async fn shelf_update_does_not_starve_during_continuous_single_credit_events() {
    let mut wire = Wire::new();
    let mut client = ClientShelf::default();
    wire.publish_shelf(1, 0).await;
    while let Ok(frame) = wire.reliable_rx.try_recv() {
        client.receive_framed(&frame);
    }
    while let Ok(frame) = wire.world_rx.try_recv() {
        client.receive_framed(&frame);
    }
    let mut body = Vec::new();
    ForgetLevelChunk { chunk_x: 100, chunk_z: 100 }.encode(&mut body).unwrap();
    let mut dummy = bytes::BytesMut::new();
    mc_protocol::frame::append_frame(&mut dummy, ForgetLevelChunk::ID, &body,
        mc_protocol::frame::Compression::Threshold(256)).unwrap();
    for tick in 2..=4 {
        let sender = &wire.connections[&wire.id].world_tx;
        while sender.capacity() > 1 {
            sender.try_send(dummy.clone().freeze()).unwrap();
        }
        let items = (tick - 1) as usize;
        wire.route_shelf_update(items);
        wire.publish_shelf(tick, items).await;
        while let Ok(frame) = wire.reliable_rx.try_recv() {
            client.receive_framed(&frame);
        }
        while let Ok(frame) = wire.world_rx.try_recv() {
            client.receive_framed(&frame);
        }
        assert_eq!(client.items, Some(items), "tick {tick}: {:?}", client.arrivals);
    }
    wire.finish();
}

#[tokio::test]
async fn shelf_update_pending_during_skip_is_covered_by_latest_chunk_repair() {
    let mut wire = Wire::new();
    let mut client = ClientShelf::default();
    wire.publish_shelf(1, 0).await;
    while let Ok(frame) = wire.reliable_rx.try_recv() {
        client.receive_framed(&frame);
    }
    while let Ok(frame) = wire.world_rx.try_recv() {
        client.receive_framed(&frame);
    }
    let mut body = Vec::new();
    ForgetLevelChunk { chunk_x: 100, chunk_z: 100 }.encode(&mut body).unwrap();
    let mut dummy = bytes::BytesMut::new();
    mc_protocol::frame::append_frame(&mut dummy, ForgetLevelChunk::ID, &body,
        mc_protocol::frame::Compression::Threshold(256)).unwrap();
    let sender = &wire.connections[&wire.id].world_tx;
    while sender.capacity() > 0 {
        sender.try_send(dummy.clone().freeze()).unwrap();
    }
    wire.route_shelf_update(1);
    wire.publish_shelf(2, 1).await;
    while let Ok(frame) = wire.reliable_rx.try_recv() {
        client.receive_framed(&frame);
    }
    while let Ok(frame) = wire.world_rx.try_recv() {
        client.receive_framed(&frame);
    }
    wire.viewers.get_mut(&wire.id).unwrap().skipped = true;
    wire.publish_shelf(3, 2).await;
    while let Ok(frame) = wire.reliable_rx.try_recv() {
        client.receive_framed(&frame);
    }
    while let Ok(frame) = wire.world_rx.try_recv() {
        client.receive_framed(&frame);
    }
    wire.finish();
    assert_eq!(client.chunks_received, 2, "{:?}", client.arrivals);
    assert_eq!(client.items, Some(2), "{:?}", client.arrivals);
    assert!(!client.arrivals.iter().any(|s| s.starts_with("update{")));
}

#[tokio::test]
async fn full_world_queue_does_not_disconnect_or_starve_shelf_updates() {
    let mut wire = Wire::new();
    wire.publish_shelf(1, 0).await;
    let mut body = Vec::new();
    ForgetLevelChunk { chunk_x: 100, chunk_z: 100 }.encode(&mut body).unwrap();
    let mut dummy = bytes::BytesMut::new();
    mc_protocol::frame::append_frame(&mut dummy, ForgetLevelChunk::ID, &body,
        mc_protocol::frame::Compression::Threshold(256)).unwrap();
    let sender = &wire.connections[&wire.id].world_tx;
    while sender.capacity() > 0 {
        sender.try_send(dummy.clone().freeze()).unwrap();
    }
    // Two old FIFO updates are drained against the latest tick-3 snapshot.
    // Neither may overwrite that snapshot with its stale item count.
    let changes: Vec<_> = [1, 2].into_iter().map(|items| PlayerChange::BlockEntityData {
        position: SHELF,
        block_entity_type: BlockEntityKind::Shelf.protocol_id(),
        nbt: shelf_tag(items),
    }).collect();
    assert!(crate::server::pump::route_changes(&changes, wire.viewers.get_mut(&wire.id).unwrap(),
        &wire.connections[&wire.id], &wire.server));
    wire.publish_shelf(3, 2).await;
    let mut client = ClientShelf::default();
    while let Ok(frame) = wire.reliable_rx.try_recv() {
        client.receive_framed(&frame);
    }
    while let Ok(frame) = wire.world_rx.try_recv() {
        client.receive_framed(&frame);
    }
    wire.publish_shelf(3, 2).await;
    let queued = wire.reliable_rx.len() + wire.world_rx.len();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client_socket = TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
    let (server_socket, _) = listener.accept().await.unwrap();
    let mut server_connection = crate::connection::Connection::new(server_socket);
    server_connection.set_compression(mc_protocol::frame::Compression::Threshold(256));
    let (_, writer) = server_connection.into_play_halves();
    let writer_task = tokio::spawn(play_writer(
        writer,
        Arc::clone(&wire.server.net_counters),
        Arc::clone(&wire.server.connections),
        wire.id,
        uuid::Uuid::nil(),
        PlayReceivers {
            reliable: std::mem::replace(&mut wire.reliable_rx, mpsc::channel(1).1),
            world: std::mem::replace(&mut wire.world_rx, mpsc::channel(1).1),
            dh: mpsc::channel(1).1,
            keepalive: mpsc::channel(1).1,
        },
    ));
    let mut client_connection = crate::connection::Connection::new(client_socket);
    client_connection.set_compression(mc_protocol::frame::Compression::Threshold(256));
    for _ in 0..queued {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(5), client_connection.read_frame_play()).await.unwrap().unwrap();
        client.receive(frame.id, &frame.body);
    }
    writer_task.abort();
    assert!(writer_task.await.unwrap_err().is_cancelled());
    wire.finish();
    assert_eq!(client.items, Some(2), "latest update must arrive despite full world queue: {:?}", client.arrivals);
    assert_eq!(client.updates_dropped, 0);
    assert!(!client.arrivals.iter().any(|arrival| arrival == "update{items: 1}"), "stale event must not replace the newer snapshot: {:?}", client.arrivals);
}
