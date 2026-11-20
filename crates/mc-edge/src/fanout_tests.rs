//! fanout.rs unit tests: priority tagging, framing, overflow rule.

use super::*;
use mc_core::boundary::{ChangeSet, TickOutput, WorldSnapshot};
use mc_protocol::packets::Packet;
use std::sync::Arc;

fn tick() -> TickOutput {
    TickOutput {
        tick: 1,
        snapshot: Arc::new(WorldSnapshot {
            tick: 1,
            day_time: 0,
            chunks: Default::default(),
            day_time_rate: 1.0,
            ..Default::default()
        }),
        changes: Arc::new(ChangeSet::default()),
        times: Default::default(),
    }
}

#[test]
fn publish_tags_reliable_and_best_effort() {
    let out = tick();
    let mut v = ViewerState::new(
        ConnectionId(1),
        mc_core::boundary::EntityId(1),
        [0.5, 64.0, 0.5],
    );
    let packets = publish(&out, [&mut v].into_iter(), 0);
    assert!(!packets.is_empty());
    // Center/radius are reliable; all carry framed bytes.
    assert!(
        packets
            .iter()
            .any(|p| p.priority == Priority::Reliable && !p.data.is_empty())
    );
    for p in &packets {
        assert_eq!(p.connection, ConnectionId(1));
    }
}

#[test]
fn reliable_overflow_threshold() {
    let mk = |n: usize| -> Vec<AddressedPacket> {
        (0..n)
            .map(|i| AddressedPacket {
                connection: ConnectionId((i % 2) as u64 + 1),
                data: bytes::Bytes::from_static(b"x"),
                priority: Priority::Reliable,
            })
            .collect()
    };
    assert!(reliable_overflow(&mk(10)).is_empty());
    let victims = reliable_overflow(&mk(RELIABLE_QUEUE_BOUND * 2 + 2));
    assert_eq!(victims.len(), 2);
    // Best-effort never overflows.
    let best: Vec<AddressedPacket> = (0..RELIABLE_QUEUE_BOUND * 3)
        .map(|_| AddressedPacket {
            connection: ConnectionId(9),
            data: bytes::Bytes::from_static(b"x"),
            priority: Priority::BestEffort,
        })
        .collect();
    assert!(reliable_overflow(&best).is_empty());
}

#[test]
fn publish_empty_viewers_empty_packets() {
    let out = tick();
    let packets: Vec<AddressedPacket> = publish(&out, [].into_iter(), 2);
    assert!(packets.is_empty());
}

fn registries_fixture() -> (
    mc_data::Registry,
    mc_world::BlockRegistry,
    mc_data::items::ItemRegistry,
    mc_data::block_entity_types::BlockEntityTypeRegistry,
) {
    (
        crate::chunks::test_biome_registry(&["minecraft:plains"]),
        mc_world::BlockRegistry::from_report(&mc_data::blocks::krasnokamen_required_blocks_report())
            .expect("blocks build"),
        mc_data::items::krasnokamen_required_items(),
        mc_data::block_entity_types::krasnokamen_required_block_entity_types(),
    )
}

fn chunk_tick(
    revision: u64,
    chunk: Arc<mc_world::chunk::Chunk>,
    east: Option<(u64, Arc<mc_world::chunk::Chunk>)>,
) -> TickOutput {
    let mut chunks = vec![mc_core::boundary::SnapshotChunk {
        pos: mc_world::chunk::ChunkPos { x: 0, z: 0 },
        revision,
        chunk,
    }];
    if let Some((revision, chunk)) = east {
        chunks.push(mc_core::boundary::SnapshotChunk {
            pos: mc_world::chunk::ChunkPos { x: 1, z: 0 },
            revision,
            chunk,
        });
    }
    TickOutput {
        tick: revision,
        snapshot: Arc::new(WorldSnapshot {
            tick: revision,
            day_time: 0,
            chunks: std::sync::Arc::new(chunks),
            day_time_rate: 1.0,
            ..Default::default()
        }),
        changes: Arc::new(ChangeSet::default()),
        times: Default::default(),
    }
}

/// With one cache across publishes, an unchanged revision serves the same
/// encoded chunk, and a changed chunk (new revision, no change set reached
/// the pump) is re-encoded with its new content, never served stale.
#[test]
fn cached_chunk_packets_follow_revisions() {
    let (biomes, blocks, items, block_entity_types) = registries_fixture();
    let registries = PlanRegistries {
        biomes: &biomes,
        blocks: &blocks,
        items: &items,
        block_entity_types: &block_entity_types,
    };
    let mut cache = crate::plan::ChunkPacketCache::new();
    let mut chunk_packet = |output: &TickOutput, viewer: u64| {
        let mut v = ViewerState::new(
            ConnectionId(viewer),
            mc_core::boundary::EntityId(viewer as i32),
            [8.0, 64.0, 8.0],
        );
        publish_with_registries(output, [&mut v].into_iter(), 0, &registries, &mut cache)
            .into_iter()
            .find(|p| {
                let mut framed = bytes::BytesMut::from(&p.data[..]);
                mc_protocol::frame::try_decode_frame(
                    &mut framed,
                    mc_protocol::frame::Compression::Threshold(256),
                )
                .ok()
                .flatten()
                .is_some_and(|frame| {
                    frame.id == mc_protocol::packets::play::LevelChunkWithLight::ID
                })
            })
            .expect("chunk packet routed")
            .data
    };
    let empty = |x| {
        let mut chunk = mc_world::chunk::Chunk::empty(
            mc_world::chunk::ChunkPos { x, z: 0 },
            mc_world::BlockStateId(0),
            mc_data::Identifier::parse("minecraft:plains").expect("static id"),
        );
        chunk.section_lights.fill(mc_world::chunk::SectionLight {
            block: Some(mc_world::chunk::LightSection::uniform(0)),
            sky: Some(mc_world::chunk::LightSection::uniform(0xFF)),
        });
        Arc::new(chunk)
    };
    let base = empty(0);
    let mut edited = (*base).clone();
    edited.set_block(4, 64, 4, mc_world::BlockStateId(1));
    let light_table = mc_data::block_light::load(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../data/vanilla/reports/block_light.json"
    ))
    .expect("block light report loads");
    let mut neighbours = [[None; 3]; 3];
    neighbours[1][1] = Some(&edited);
    let baked = mc_world::light::compute_chunk_light(neighbours, &light_table);
    edited.set_baked_light(&baked);
    let first = chunk_packet(&chunk_tick(1, Arc::clone(&base), None), 1);
    let repeat = chunk_packet(&chunk_tick(1, Arc::clone(&base), None), 2);
    let changed = chunk_packet(&chunk_tick(2, Arc::new(edited), None), 3);
    assert!(!first.is_empty());
    assert_eq!(first, repeat, "same revision, same packet");
    assert_ne!(first, changed, "new revision re-encoded, not served stale");
    // The core bakes neighbour relights and bumps the affected centre's
    // revision. Neighbour revisions alone no longer affect packet encoding.
    let glowstone = blocks
        .block(&mc_data::Identifier::parse("minecraft:glowstone").expect("id"))
        .expect("glowstone")
        .default;
    let mut lit = (*empty(1)).clone();
    lit.set_block(0, 64, 8, glowstone);
    let dark = chunk_packet(&chunk_tick(1, Arc::clone(&base), Some((10, empty(1)))), 4);
    let mut relit = (*base).clone();
    relit.section_lights[8].block = Some(mc_world::chunk::LightSection::uniform(1));
    let bright = chunk_packet(
        &chunk_tick(3, Arc::new(relit), Some((11, Arc::new(lit)))),
        5,
    );
    assert_ne!(dark, bright, "baked light and the centre revision changed");
}

#[test]
fn mooshroom_to_cow_fanout_removes_then_respawns_the_same_identity_for_every_viewer() {
    use mc_core::boundary::{EntityAppearance, EntityDataValue, EntityId, EntityView};
    use mc_protocol::frame::{Compression, try_decode_frame};
    use mc_protocol::packets::play::{AddEntity, ClientboundSetEntityData, RemoveEntities};
    let types = mc_data::entity_types::krasnokamen_required_entity_types();
    let type_id = |name: &str| {
        types
            .id_of(&mc_data::Identifier::parse(name).unwrap())
            .unwrap()
    };
    let pos = mc_world::chunk::ChunkPos { x: 0, z: 0 };
    let mut chunk = mc_world::chunk::Chunk::empty(
        pos,
        mc_world::BlockStateId(0),
        mc_data::Identifier::parse("minecraft:plains").unwrap(),
    );
    chunk.section_lights.fill(mc_world::chunk::SectionLight {
        block: Some(mc_world::chunk::LightSection::uniform(0)),
        sky: Some(mc_world::chunk::LightSection::uniform(0xFF)),
    });
    let mut output = chunk_tick(1, Arc::new(chunk), None);
    let uuid = uuid::Uuid::from_u128(0x1234);
    let resident = EntityView {
        id: EntityId(2),
        uuid,
        type_id: type_id("minecraft:mooshroom"),
        position: [8.0, 80.0, 8.0],
        velocity: [0.0; 3],
        yaw: 0.0,
        pitch: 0.0,
        head_yaw: 0.0,
        on_ground: true,
        faction: 0,
        appearance: Arc::new(EntityAppearance {
            extra: vec![(18, EntityDataValue::VarInt(1))],
            ..Default::default()
        }),
    };
    let snapshot = Arc::get_mut(&mut output.snapshot).unwrap();
    snapshot.entities = vec![resident].into();
    let mut viewers = [
        ViewerState::new(ConnectionId(1), EntityId(1), [8.0, 80.0, 8.0]),
        ViewerState::new(ConnectionId(3), EntityId(3), [8.0, 80.0, 8.0]),
    ];
    let first = publish(&output, viewers.iter_mut(), 0);
    for viewer in &viewers {
        assert!(viewer.tracked_entities.contains_key(&EntityId(2)));
        assert!(
            first
                .iter()
                .filter(|packet| packet.connection == viewer.connection)
                .any(|packet| {
                    let mut bytes = bytes::BytesMut::from(packet.data.as_ref());
                    let frame = try_decode_frame(&mut bytes, Compression::Threshold(256))
                        .unwrap()
                        .unwrap();
                    frame.id == AddEntity::ID
                        && AddEntity::decode(&mut frame.body.clone())
                            .unwrap()
                            .entity_type_id
                            == type_id("minecraft:mooshroom") as i32
                })
        );
    }
    output.tick = 2;
    let snapshot = Arc::get_mut(&mut output.snapshot).unwrap();
    snapshot.tick = 2;
    let mut rows: Vec<EntityView> = snapshot.entities.iter().cloned().collect();
    rows[0].type_id = type_id("minecraft:cow");
    Arc::make_mut(&mut rows[0].appearance).extra.clear();
    snapshot.entities = rows.into();
    // Core changed type in place; same-id removal/spawn tombstones are absent.
    Arc::get_mut(&mut output.changes)
        .unwrap()
        .entities_appearance_changed
        .push(EntityId(2));
    let packets = publish(&output, viewers.iter_mut(), 0);
    for viewer in &viewers {
        let frames: Vec<_> = packets
            .iter()
            .filter(|packet| packet.connection == viewer.connection)
            .map(|packet| {
                let mut bytes = bytes::BytesMut::from(packet.data.as_ref());
                let frame = try_decode_frame(&mut bytes, Compression::Threshold(256))
                    .unwrap()
                    .unwrap();
                if matches!(
                    frame.id,
                    AddEntity::ID | RemoveEntities::ID | ClientboundSetEntityData::ID
                ) {
                    assert_eq!(packet.priority, Priority::BestEffort);
                }
                frame
            })
            .collect();
        let removal = frames
            .iter()
            .position(|frame| frame.id == RemoveEntities::ID)
            .unwrap();
        let spawn = frames
            .iter()
            .position(|frame| frame.id == AddEntity::ID)
            .unwrap();
        let metadata = frames
            .iter()
            .position(|frame| frame.id == ClientboundSetEntityData::ID)
            .unwrap();
        assert!(removal < spawn && spawn < metadata);
        assert_eq!(
            RemoveEntities::decode(&mut frames[removal].body.clone())
                .unwrap()
                .entity_ids,
            [2]
        );
        let added = AddEntity::decode(&mut frames[spawn].body.clone()).unwrap();
        assert_eq!(added.entity_id, 2);
        assert_eq!(added.uuid, uuid);
        assert_eq!(added.entity_type_id, type_id("minecraft:cow") as i32);
        let data = ClientboundSetEntityData::decode(&mut frames[metadata].body.clone()).unwrap();
        assert_eq!(data.entity_id, 2);
        assert!(
            !data.values.iter().any(|value| matches!(
                value,
                mc_protocol::packets::play::EntityDataValue::Int { index: 18, .. }
            )),
            "mooshroom variant must not leak into the cow"
        );
        assert!(viewer.tracked_entities.contains_key(&EntityId(2)));
    }
    let repeat = publish(&output, viewers.iter_mut(), 0);
    assert!(repeat.iter().all(|packet| {
        let mut bytes = bytes::BytesMut::from(packet.data.as_ref());
        let frame = try_decode_frame(&mut bytes, Compression::Threshold(256))
            .unwrap()
            .unwrap();
        frame.id != RemoveEntities::ID && frame.id != AddEntity::ID
    }));
}

/// A spawn burst of mounted and leashed entities larger than the reliable
/// queue (battle cavalry: 2,000 horses at `battle setup 42 field-10k`). Links
/// are world state: each one rides the world FIFO right after its own
/// AddEntity, so one publish's reliable output does not grow with the burst
/// and a client that keeps reading is never closed by `reliable_overflow`.
#[test]
fn mounted_spawn_burst_links_ride_the_world_queue_without_reliable_overflow() {
    use mc_core::boundary::{EntityAppearance, EntityId, EntityView};
    use mc_protocol::frame::{Compression, try_decode_frame};
    use mc_protocol::packets::play::{
        AddEntity, ClientboundSetEntityLeash, ClientboundSetPassengers,
    };
    let types = mc_data::entity_types::krasnokamen_required_entity_types();
    let type_id = |name: &str| {
        types
            .id_of(&mc_data::Identifier::parse(name).unwrap())
            .unwrap()
    };
    let pos = mc_world::chunk::ChunkPos { x: 0, z: 0 };
    let mut chunk = mc_world::chunk::Chunk::empty(
        pos,
        mc_world::BlockStateId(0),
        mc_data::Identifier::parse("minecraft:plains").unwrap(),
    );
    chunk.section_lights.fill(mc_world::chunk::SectionLight {
        block: Some(mc_world::chunk::LightSection::uniform(0)),
        sky: Some(mc_world::chunk::LightSection::uniform(0xFF)),
    });
    let mut output = chunk_tick(1, Arc::new(chunk), None);
    let horses = RELIABLE_QUEUE_BOUND + 88;
    let mut rows = Vec::with_capacity(2 * horses);
    for i in 0..horses {
        let horse = EntityId(10 + 2 * i as i32);
        let rider = EntityId(horse.0 + 1);
        // View distance 0 tracks only the viewer's own column: stack them.
        let y = 64.0 + i as f64 * 0.05;
        let row = |id: EntityId, type_id: u32, y: f64, appearance: EntityAppearance| EntityView {
            id,
            uuid: uuid::Uuid::from_u128(id.0 as u128),
            type_id,
            position: [8.0, y, 8.0],
            velocity: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            head_yaw: 0.0,
            on_ground: true,
            faction: 0,
            appearance: Arc::new(appearance),
        };
        rows.push(row(
            horse,
            type_id("minecraft:horse"),
            y,
            EntityAppearance {
                passengers: vec![rider],
                leash_holder: (i % 2 == 0).then_some(rider),
                ..Default::default()
            },
        ));
        rows.push(row(
            rider,
            type_id("minecraft:zombie"),
            y + 0.025,
            EntityAppearance::default(),
        ));
    }
    Arc::get_mut(&mut output.snapshot).unwrap().entities = rows.into();
    let mut viewer = ViewerState::new(ConnectionId(1), EntityId(1), [8.0, 80.0, 8.0]);
    let packets = publish(&output, [&mut viewer].into_iter(), 0);
    assert_eq!(viewer.tracked_entities.len(), 2 * horses);
    assert!(
        reliable_overflow(&packets).is_empty(),
        "a spawn burst must not overflow the reliable queue"
    );
    let mut spawned = std::collections::HashSet::new();
    let (mut passengers, mut leashes, mut riders_after_vehicle) = (0, 0, 0);
    for packet in &packets {
        let mut bytes = bytes::BytesMut::from(packet.data.as_ref());
        let frame = try_decode_frame(&mut bytes, Compression::Threshold(256))
            .unwrap()
            .unwrap();
        let entity = match frame.id {
            AddEntity::ID => {
                let id = AddEntity::decode(&mut frame.body.clone())
                    .unwrap()
                    .entity_id;
                // Riders have odd ids, one above their horse's.
                if id % 2 == 1 && spawned.contains(&(id - 1)) {
                    riders_after_vehicle += 1;
                }
                spawned.insert(id);
                continue;
            }
            ClientboundSetPassengers::ID => {
                passengers += 1;
                ClientboundSetPassengers::decode(&mut frame.body.clone())
                    .unwrap()
                    .vehicle_id
            }
            ClientboundSetEntityLeash::ID => {
                leashes += 1;
                ClientboundSetEntityLeash::decode(&mut frame.body.clone())
                    .unwrap()
                    .source_entity_id
            }
            _ => continue,
        };
        assert_eq!(
            packet.priority,
            Priority::BestEffort,
            "links are world state"
        );
        assert!(
            spawned.contains(&entity),
            "link before the AddEntity of {entity}"
        );
    }
    // One SetPassengers in each horse's pairing, and one more in the pairing of
    // each rider spawned after its horse (vanilla `sendPairingData`).
    assert!(riders_after_vehicle > 0, "the burst mixes spawn orders");
    assert_eq!(
        (passengers, leashes),
        (horses + riders_after_vehicle, horses / 2)
    );
}

/// Smoke for the R3k-6 line: the real logging init (`EnvFilter::try_from_default_env`, exactly
/// what the server binary runs) with the GO's RUST_LOG and WITHOUT `MC_CORE_PHASE_LOG` must reach
/// the publish and emit one `pump plan` line with counters and a non-zero plan clock. A bare
/// `enabled!(DEBUG)` guard asks this module's target (`mc_edge::fanout`) instead, which that
/// filter leaves off, and `plan_us` used to hang off the item probe.
#[test]
fn plan_debug_line_appears_under_the_go_filter_without_the_phase_log() {
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::filter::EnvFilter;

    #[derive(Clone)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    assert!(
        std::env::var_os("MC_CORE_PHASE_LOG").is_none(),
        "this smoke proves the plan clock does not depend on the phase log"
    );
    // The same filter string the GO puts in RUST_LOG, parsed by the same EnvFilter the binary
    // installs (`EnvFilter::try_from_default_env`); the crate denies `unsafe`, so the env var
    // itself is not set here.
    let filter = EnvFilter::new(GO_RUST_LOG);
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let writer = Capture(Arc::clone(&buffer));
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(move || writer.clone())
        .without_time()
        .finish();

    let (biomes, blocks, items, block_entity_types) = registries_fixture();
    let registries = PlanRegistries {
        biomes: &biomes,
        blocks: &blocks,
        items: &items,
        block_entity_types: &block_entity_types,
    };
    let mut cache = crate::plan::ChunkPacketCache::new();
    let mut chunk = mc_world::chunk::Chunk::empty(
        mc_world::chunk::ChunkPos { x: 0, z: 0 },
        mc_world::BlockStateId(0),
        mc_data::Identifier::parse("minecraft:plains").expect("static id"),
    );
    chunk.section_lights.fill(mc_world::chunk::SectionLight {
        block: Some(mc_world::chunk::LightSection::uniform(0)),
        sky: Some(mc_world::chunk::LightSection::uniform(0xFF)),
    });
    let output = chunk_tick(1, Arc::new(chunk), None);
    let mut viewer = ViewerState::new(
        ConnectionId(1),
        mc_core::boundary::EntityId(1),
        [8.0, 64.0, 8.0],
    );
    viewer.loaded_chunks.insert((0, 0), 0);

    tracing::subscriber::with_default(subscriber, || {
        let packets = publish_with_registries(
            &output,
            [&mut viewer].into_iter(),
            1,
            &registries,
            &mut cache,
        );
        assert!(!packets.is_empty(), "the publish must plan something");
    });
    let text = String::from_utf8(buffer.lock().unwrap().clone()).expect("utf8 log");
    assert!(
        text.contains("pump plan"),
        "the GO filter must reach the line, got: {text}"
    );
    for field in [
        "move_misses=",
        "move_hits=",
        "rescans=",
        "entities_scanned=",
        "chunk_packets=",
    ] {
        assert!(text.contains(field), "counters missing {field}: {text}");
    }
    let plan_ns: u64 = text
        .split("plan_ns=")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| panic!("no plan_ns field: {text}"));
    assert!(
        plan_ns > 0,
        "the plan clock must run without MC_CORE_PHASE_LOG: {text}"
    );
}

/// The GO's exact RUST_LOG for the R3k-6 leg.
const GO_RUST_LOG: &str = "info,mc_edge::server::pump=debug,\
mc_core::tick::input::session=debug,mc_edge::server::plan=debug";

/// A viewer that already holds the published revision needs no delta at all,
/// so it must not take the index away from a viewer that is behind and covered
/// by the pump's chain: the fan-out used to pick that `same` viewer first and
/// every lagging viewer paid a full rescan.
#[test]
fn a_current_viewer_does_not_preempt_the_chain_for_a_stale_one() {
    use crate::plan::{ChunkPacketCache, PublicationHistory, ViewerState};
    use mc_core::boundary::EntityId;
    use mc_core::entities::{ColdFields, EntityKind, EntityStore};

    let (biomes, blocks, items, block_entity_types) = registries_fixture();
    let registries = PlanRegistries {
        biomes: &biomes,
        blocks: &blocks,
        items: &items,
        block_entity_types: &block_entity_types,
    };
    let type_id = mc_data::entity_types::entity_type_contract_26_1_2_by_name("minecraft:cow")
        .expect("cow contract")
        .protocol_id;
    let cold = |pos: [f64; 3]| ColdFields {
        pos,
        vel: [0.0; 3],
        yaw: 0.0,
        pitch: 0.0,
        ground: true,
        health: 20.0,
        breed_cooldown: 0,
        pickup_delay: 0,
        item: None,
        appearance: Arc::new(mc_core::boundary::EntityAppearance::default()),
        movement: mc_core::blueprint::MovementClass::Walker,
        half_width: 0.45,
        height: 1.4,
        speed: 0.1,
        attack_damage: 1.0,
        sight_range: 16.0,
        attack_reach: 1.0,
    };
    let mut store = EntityStore::default();
    let first = store.spawn(
        EntityId(1_000),
        uuid::Uuid::from_u128(1_000),
        EntityKind::Mob { type_id },
        cold([8.0, 64.0, 8.0]),
    );
    let second = store.spawn(
        EntityId(1_001),
        uuid::Uuid::from_u128(1_001),
        EntityKind::Mob { type_id },
        cold([12.0, 64.0, 8.0]),
    );
    let output = |tick: u64, entities| TickOutput {
        tick,
        snapshot: Arc::new(WorldSnapshot {
            tick,
            day_time: 0,
            chunks: Default::default(),
            day_time_rate: 1.0,
            entities,
            ..Default::default()
        }),
        changes: Arc::new(ChangeSet::default()),
        times: Default::default(),
    };
    let mut history = PublicationHistory::new(8);
    let one = output(1, store.views());
    history.record(&one);
    store.set_pos(first, [9.0, 64.0, 8.0]);
    store.set_pos(second, [13.0, 64.0, 8.0]);
    store.spawn(
        EntityId(1_002),
        uuid::Uuid::from_u128(1_002),
        EntityKind::Mob { type_id },
        cold([16.0, 64.0, 8.0]),
    );
    let two = output(2, store.views());
    history.record(&two);
    store.set_pos(first, [10.0, 64.0, 8.0]);
    store.set_pos(second, [14.0, 64.0, 8.0]);
    let three = output(3, store.views());
    history.record(&three);
    assert_ne!(
        three.snapshot.entities.revision(),
        one.snapshot.entities.revision()
    );

    let viewer = |connection: u64| {
        let mut viewer = ViewerState::new(
            ConnectionId(connection),
            EntityId(9_000 + connection as i32),
            [8.0, 64.0, 8.0],
        );
        viewer.dimension = mc_core::dimensions::Dimension::Overworld;
        viewer.loaded_chunks.insert((0, 0), 0);
        viewer
    };
    let plan = |out: &TickOutput,
                viewer: &mut ViewerState,
                cache: &mut ChunkPacketCache,
                history: Option<&PublicationHistory>| {
        crate::fanout::publish_with_registries_chain(
            out,
            std::iter::once(&mut *viewer),
            0,
            &registries,
            cache,
            history,
        )
    };

    // The current viewer really holds the published revision.
    let mut current = viewer(1);
    let mut cache = ChunkPacketCache::new();
    plan(&three, &mut current, &mut cache, Some(&history));
    assert_eq!(
        current.previous_entities().unwrap().revision(),
        three.snapshot.entities.revision()
    );

    // The stale viewer holds the first revision: not the published snapshot's
    // direct predecessor, so only the chain can keep it contiguous.
    let mut stale = viewer(2);
    let mut stale_cache = ChunkPacketCache::new();
    plan(&one, &mut stale, &mut stale_cache, None);
    assert_eq!(
        stale.previous_entities().unwrap().revision(),
        one.snapshot.entities.revision()
    );
    assert_ne!(
        one.snapshot.entities.revision(),
        three.snapshot.entities.base_revision()
    );

    // A third viewer from the same base, planned against no history at all:
    // the rescan reference for the frames the stale viewer must receive.
    let mut reference = viewer(3);
    let mut reference_cache = ChunkPacketCache::new();
    plan(&one, &mut reference, &mut reference_cache, None);
    let stale_rescans_before = stale.rescans;
    let current_rescans_before = current.rescans;
    let reference_rescans_before = reference.rescans;

    let mut cache = ChunkPacketCache::new();
    let mut both = vec![current, stale];
    let packets = crate::fanout::publish_with_registries_chain(
        &three,
        both.iter_mut(),
        0,
        &registries,
        &mut cache,
        Some(&history),
    );
    let frames = |packets: &[AddressedPacket], connection: ConnectionId| {
        packets
            .iter()
            .filter(|packet| packet.connection == connection)
            .map(|packet| (packet.priority, packet.data.clone()))
            .collect::<Vec<_>>()
    };
    let stale_frames = frames(&packets, ConnectionId(2));
    let current_tracked = both[0].tracked_entities.len();
    assert_eq!(
        both[1].rescans, stale_rescans_before,
        "the lagging viewer with covered history must not rescan because a current viewer shares the publish"
    );
    assert_eq!(
        both[0].rescans, current_rescans_before,
        "the current viewer is `same` and rescans nothing"
    );

    // The rescan reference from the same base, for the ordered frames.
    let mut reference_cache = ChunkPacketCache::new();
    let reference_packets = plan(&three, &mut reference, &mut reference_cache, None);
    assert_eq!(
        reference.rescans,
        reference_rescans_before + 1,
        "the reference really rescanned"
    );
    assert!(
        !stale_frames.is_empty(),
        "the lagging viewer received frames"
    );
    assert_eq!(
        stale_frames,
        frames(&reference_packets, ConnectionId(3)),
        "the bridged viewer must receive exactly the frames a rescan of its base delivers"
    );
    assert_eq!(
        current_tracked,
        both[0].tracked_entities.len(),
        "the current viewer keeps its tracked set"
    );
}
