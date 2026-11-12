//! A world frame the queue refuses after planning leaves the client without
//! the entity state the viewer recorded as delivered.
//!
//! Planning spends the world queue's free slots, read as credit before the
//! plan, but the queue has a second producer (an LOD tile), so a planned
//! frame can find it full. `send` failing is the same event whatever filled
//! the queue; here the queue's receiver is gone, which fails every send
//! while `capacity` still reports the free slots the plan spends.

use super::*;
use mc_core::boundary::{
    ChangeSet, EntityAppearance, EntityId, EntityView, SnapshotChunk, TickOutput, WorldSnapshot,
};
use mc_protocol::frame::{Compression, try_decode_frame};
use mc_protocol::packets::play::{
    AddEntity, ClientboundSetEntityData, ClientboundSetPassengers, RemoveEntities,
};

const VIEWER: EntityId = EntityId(1);
const HORSE: EntityId = EntityId(10);
const RIDER: EntityId = EntityId(11);

/// The rider never changes: like the core's copy-on-write rows, its appearance
/// keeps one allocation across ticks, so only the horse is stale on a dismount.
static RIDER_APPEARANCE: std::sync::LazyLock<Arc<EntityAppearance>> =
    std::sync::LazyLock::new(Arc::default);

fn horse_appearance(passengers: Vec<EntityId>, name: Option<&str>) -> Arc<EntityAppearance> {
    Arc::new(EntityAppearance {
        passengers,
        custom_name: name.map(str::to_owned),
        ..Default::default()
    })
}

/// A horse and its rider (mounted or not) in the viewer's chunk.
fn horse_and_rider(tick: u64, mounted: bool) -> TickOutput {
    let passengers = if mounted { vec![RIDER] } else { vec![] };
    scene(
        tick,
        Some(horse_appearance(passengers, None)),
        Arc::clone(&RIDER_APPEARANCE),
        None,
    )
}

/// The rider alone: the horse left the viewer's range.
fn rider_alone(tick: u64) -> TickOutput {
    scene(tick, None, Arc::clone(&RIDER_APPEARANCE), None)
}

/// The tick's rows: an optional horse, the rider, and the viewer's own entity.
fn scene(
    tick: u64,
    horse: Option<Arc<EntityAppearance>>,
    rider: Arc<EntityAppearance>,
    own: Option<Arc<EntityAppearance>>,
) -> TickOutput {
    let types = mc_data::entity_types::krasnokamen_required_entity_types();
    let type_id = |name: &str| {
        types
            .id_of(&mc_data::Identifier::parse(name).unwrap())
            .unwrap()
    };
    let mut chunk = mc_world::chunk::Chunk::empty(
        mc_world::chunk::ChunkPos { x: 0, z: 0 },
        mc_world::BlockStateId(0),
        mc_data::Identifier::parse("minecraft:plains").unwrap(),
    );
    chunk.section_lights.fill(mc_world::chunk::SectionLight {
        block: Some(mc_world::chunk::LightSection::uniform(0)),
        sky: Some(mc_world::chunk::LightSection::uniform(0xFF)),
    });
    let row = |id: EntityId, name: &str, y: f64, appearance: Arc<EntityAppearance>| EntityView {
        id,
        uuid: uuid::Uuid::from_u128(id.0 as u128),
        type_id: type_id(name),
        position: [8.0, y, 8.0],
        velocity: [0.0; 3],
        yaw: 0.0,
        pitch: 0.0,
        head_yaw: 0.0,
        on_ground: true,
        appearance,
    };
    TickOutput {
        tick,
        snapshot: Arc::new(WorldSnapshot {
            tick,
            day_time_rate: 1.0,
            chunks: Arc::new(vec![SnapshotChunk {
                pos: mc_world::chunk::ChunkPos { x: 0, z: 0 },
                revision: 1,
                chunk: Arc::new(chunk),
            }]),
            entities: own
                .map(|appearance| row(VIEWER, "minecraft:player", 64.05, appearance))
                .into_iter()
                .chain(horse.map(|appearance| row(HORSE, "minecraft:horse", 64.0, appearance)))
                .chain([row(RIDER, "minecraft:zombie", 64.025, rider)])
                .collect::<Vec<_>>()
                .into(),
            ..Default::default()
        }),
        changes: Arc::new(ChangeSet::default()),
        times: Default::default(),
    }
}

/// The entity frames queued, in order: `add 10`, `passengers 10 [11]`,
/// `remove [10]`.
fn entity_frames(world_rx: &mut mpsc::Receiver<bytes::Bytes>) -> Vec<String> {
    let mut frames = Vec::new();
    while let Ok(data) = world_rx.try_recv() {
        let mut bytes = bytes::BytesMut::from(data.as_ref());
        let frame = try_decode_frame(&mut bytes, Compression::Threshold(256))
            .unwrap()
            .unwrap();
        let mut body = frame.body.clone();
        match frame.id {
            AddEntity::ID => {
                frames.push(format!(
                    "add {}",
                    AddEntity::decode(&mut body).unwrap().entity_id
                ));
            }
            ClientboundSetPassengers::ID => {
                let link = ClientboundSetPassengers::decode(&mut body).unwrap();
                frames.push(format!(
                    "passengers {} {:?}",
                    link.vehicle_id, link.passenger_ids
                ));
            }
            ClientboundSetEntityData::ID => {
                frames.push(format!(
                    "data {}",
                    ClientboundSetEntityData::decode(&mut body)
                        .unwrap()
                        .entity_id
                ));
            }
            RemoveEntities::ID => {
                frames.push(format!(
                    "remove {:?}",
                    RemoveEntities::decode(&mut body).unwrap().entity_ids
                ));
            }
            _ => {}
        }
    }
    frames
}

struct Pump {
    server: Arc<EdgeServer>,
    dir: std::path::PathBuf,
    id: ConnectionId,
    viewers: HashMap<ConnectionId, crate::plan::ViewerState>,
    connections: HashMap<ConnectionId, ConnectionHandles>,
    pool: rayon::ThreadPool,
    cache: crate::plan::ChunkPacketCache,
    /// Nothing reads the reliable queue; it only has to stay open.
    _reliable_rx: mpsc::Receiver<bytes::Bytes>,
}

impl Pump {
    fn new() -> Self {
        let (server, dir) = fixture();
        let server = Arc::new(server);
        let id = server.connections.joining_id();
        let (reliable_tx, reliable_rx) = mpsc::channel(512);
        let (world_tx, _) = mpsc::channel(1);
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
        Self {
            viewers: HashMap::from([(
                id,
                crate::plan::ViewerState::new(id, VIEWER, [8.0, 80.0, 8.0]),
            )]),
            connections,
            pool: rayon::ThreadPoolBuilder::new()
                .num_threads(1)
                .build()
                .unwrap(),
            cache: crate::plan::ChunkPacketCache::new(),
            _reliable_rx: reliable_rx,
            server,
            dir,
            id,
        }
    }

    /// Publish `output` into a world queue that refuses every frame after the
    /// plan has spent its free slots.
    async fn publish_refused(&mut self, output: &TickOutput) {
        let (world_tx, world_rx) = mpsc::channel(512);
        drop(world_rx);
        self.publish(output, world_tx).await;
    }

    /// Publish `output` into a healthy world queue; the entity frames queued.
    async fn publish_delivered(&mut self, output: &TickOutput) -> Vec<String> {
        self.publish_delivered_into(output, 512).await
    }

    /// As [`Self::publish_delivered`], into a world queue of `slots`: the plan's credit.
    async fn publish_delivered_into(&mut self, output: &TickOutput, slots: usize) -> Vec<String> {
        let (world_tx, mut world_rx) = mpsc::channel(slots);
        self.publish(output, world_tx).await;
        entity_frames(&mut world_rx)
    }

    async fn publish(&mut self, output: &TickOutput, world_tx: mpsc::Sender<bytes::Bytes>) {
        self.connections.get_mut(&self.id).unwrap().world_tx = world_tx;
        publish_and_route(
            &self.server,
            &self.pool,
            &mut self.viewers,
            output,
            &mut self.cache,
            &mut self.connections,
        )
        .await;
    }

    fn finish(self) {
        self.server.connections.remove(self.id);
        self.server.hub.shutdown();
        std::fs::remove_dir_all(self.dir).unwrap();
    }
}

/// A spawn whose frames the queue refused is sent again: the viewer had
/// recorded the horse and its rider as delivered, so without a repair the
/// client never sees the mob, or the rider mounted.
#[tokio::test]
async fn refused_spawn_frames_are_sent_again_on_the_next_publication() {
    let mut pump = Pump::new();
    let output = horse_and_rider(1, true);
    pump.publish_refused(&output).await;
    assert!(
        pump.viewers[&pump.id].tracked_entities.contains_key(&HORSE),
        "the plan spawned the horse"
    );
    let frames = pump.publish_delivered(&output).await;
    assert!(frames.contains(&"add 10".into()), "horse: {frames:?}");
    assert!(frames.contains(&"add 11".into()), "rider: {frames:?}");
    assert!(
        frames.contains(&"passengers 10 [11]".into()),
        "mount: {frames:?}"
    );
    pump.finish();
}

/// A dismount (`SetPassengers(horse, [])`) the queue refused leaves the
/// client's rider mounted; the horse is removed and spawned again, empty.
#[tokio::test]
async fn refused_dismount_is_repaired_by_respawning_the_vehicle() {
    let mut pump = Pump::new();
    let frames = pump.publish_delivered(&horse_and_rider(1, true)).await;
    assert!(
        frames.contains(&"passengers 10 [11]".into()),
        "the mount was delivered: {frames:?}"
    );
    let dismounted = horse_and_rider(2, false);
    pump.publish_refused(&dismounted).await;
    let frames = pump.publish_delivered(&dismounted).await;
    let position = |frame: &str| frames.iter().position(|seen| seen == frame);
    let (removed, added) = (position("remove [10]"), position("add 10"));
    assert!(removed.is_some() && removed < added, "{frames:?}");
    assert!(
        !frames.contains(&"passengers 10 [11]".into()),
        "the respawned horse carries no rider: {frames:?}"
    );
    pump.finish();
}

/// A removal the queue refused leaves the entity on the client, which the
/// viewer had already forgotten: it is removed again.
#[tokio::test]
async fn refused_removal_is_sent_again_on_the_next_publication() {
    let mut pump = Pump::new();
    pump.publish_delivered(&horse_and_rider(1, true)).await;
    let gone = rider_alone(2);
    pump.publish_refused(&gone).await;
    let frames = pump.publish_delivered(&gone).await;
    assert!(frames.contains(&"remove [10]".into()), "{frames:?}");
    pump.finish();
}

/// Lost frames of many entities must not hide more of them than the credit can
/// spawn again: a removal rides with its own spawn.
#[tokio::test]
async fn refused_repairs_remove_only_what_the_credit_respawns() {
    let mut pump = Pump::new();
    pump.publish_delivered(&horse_and_rider(1, true)).await;
    let changed = scene(
        2,
        Some(horse_appearance(vec![RIDER], Some("Epona"))),
        horse_appearance(vec![], Some("Rider")),
        None,
    );
    pump.publish_refused(&changed).await;
    // One chunk resend and one entity group fit; two entities were refused.
    let frames = pump.publish_delivered_into(&changed, 8).await;
    let adds: Vec<_> = frames
        .iter()
        .filter(|frame| frame.starts_with("add"))
        .collect();
    assert!(!adds.is_empty(), "{frames:?}");
    for (index, frame) in frames.iter().enumerate() {
        if let Some(ids) = frame
            .strip_prefix("remove [")
            .and_then(|ids| ids.strip_suffix(']'))
        {
            assert!(!ids.contains(','), "one removal per spawn: {frames:?}");
            assert_eq!(
                frames.get(index + 1).map(String::as_str),
                Some(format!("add {ids}").as_str()),
                "a removal is followed by its own spawn: {frames:?}"
            );
        }
    }
    pump.finish();
}

/// The viewer's own vehicle is never removed (the client would dismount the
/// player): its state is sent again over the client's copy.
#[tokio::test]
async fn refused_frames_of_the_own_vehicle_are_resent_without_removing_it() {
    let mut pump = Pump::new();
    let mounted = |name| {
        scene(
            if name { 2 } else { 1 },
            Some(horse_appearance(vec![VIEWER], name.then_some("Epona"))),
            Arc::clone(&RIDER_APPEARANCE),
            None,
        )
    };
    pump.publish_delivered(&mounted(false)).await;
    let renamed = mounted(true);
    pump.publish_refused(&renamed).await;
    let frames = pump.publish_delivered(&renamed).await;
    assert!(
        frames.iter().all(|frame| !frame.starts_with("remove")),
        "{frames:?}"
    );
    assert!(frames.contains(&"passengers 10 [1]".into()), "{frames:?}");
    assert!(frames.contains(&"data 10".into()), "{frames:?}");
    pump.finish();
}

/// A refused frame of the viewer's own entity (its data) is sent again.
#[tokio::test]
async fn refused_own_entity_data_is_sent_again() {
    let mut pump = Pump::new();
    let own = |health| {
        Arc::new(EntityAppearance {
            health,
            ..Default::default()
        })
    };
    let before = scene(1, None, Arc::clone(&RIDER_APPEARANCE), Some(own(20.0)));
    let frames = pump.publish_delivered(&before).await;
    assert!(frames.contains(&"data 1".into()), "{frames:?}");
    let hurt = scene(2, None, Arc::clone(&RIDER_APPEARANCE), Some(own(5.0)));
    pump.publish_refused(&hurt).await;
    let frames = pump.publish_delivered(&hurt).await;
    assert!(frames.contains(&"data 1".into()), "{frames:?}");
    pump.finish();
}
