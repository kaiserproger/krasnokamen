//! A block the core writes at its dimension coordinator (here a cross-dimension
//! respawn spending an anchor charge) reaches a viewer already holding the chunk.
use super::*;
use mc_core::boundary::PlayerInput;
use mc_core::dimensions::Dimension;
use mc_core::registry::CoreRegistry;
use mc_core::tick::{TickState, run_tick};
use mc_protocol::packets::play::{
    BlockUpdate, ForgetLevelChunk, LevelChunkWithLight, SectionBlocksUpdate, pack_section_pos,
    pack_section_relative_pos,
};

const ANCHOR: [i32; 3] = [4, 150, 4];

fn world(dimension: Dimension, registry: &Arc<CoreRegistry>) -> TickState {
    let blocks = Arc::clone(&registry.blocks);
    let generator: Arc<dyn mc_world::ChunkGenerator> = match dimension {
        Dimension::Nether => Arc::new(mc_worldgen::NetherGenerator::try_new(42, blocks).unwrap()),
        _ => Arc::new(mc_worldgen::TerrainGenerator::new(42, blocks)),
    };
    let world = mc_core::world::ChunkWorld::new(
        42,
        generator,
        registry,
        mc_world::BlockStateId(registry.states.air),
        [8, 150, 8],
        dimension.geometry(),
    );
    let mut state = TickState::new(Arc::clone(registry), world, 0, 4096);
    state.dimension = dimension;
    state.gamerules.insert("spawn_mobs".into(), "false".into());
    // Respawns wait for the 3x3 area around the spawn and the respawn point.
    let centre = mc_world::ChunkPos { x: 0, z: 0 };
    state
        .tickets
        .insert("coordinator-block".into(), (centre, 1));
    let want = (-1..=1)
        .flat_map(|x| (-1..=1).map(move |z| (mc_world::ChunkPos { x, z }, 0)))
        .collect();
    state.world.ensure_blocking(&want);
    for x in 0..16 {
        for z in 0..16 {
            state.world.set_block(x, 149, z, registry.states.stone);
        }
    }
    state.world.fold_overlay();
    state
}

fn tick(root: &mut TickState, inputs: &[PlayerInput]) -> TickOutput {
    run_tick(root, inputs).0
}

fn command(root: &mut TickState, connection: u64, command: &str) -> TickOutput {
    tick(
        root,
        &[PlayerInput::Command {
            connection: ConnectionId(connection),
            command: command.into(),
            permission_level: 2,
        }],
    )
}

fn anchor(registry: &CoreRegistry, charges: u8) -> u32 {
    registry
        .blocks
        .by_name_and_props(
            &mc_data::Identifier::parse("minecraft:respawn_anchor").unwrap(),
            &[("charges".into(), charges.to_string())],
        )
        .unwrap()
        .0
}

/// Apply one planned packet to the client's view of the anchor cell. A chunk
/// placeholder stands for the frame of the planned output (the frame cache is
/// keyed by its revision), so the model reads the cell from that snapshot.
fn apply(client: &mut Option<u32>, packet: &PlannedPacket, output: &TickOutput) {
    let [x, y, z] = ANCHOR;
    if let Some(chunk) = packet.chunk {
        assert_eq!(packet.id, LevelChunkWithLight::ID);
        if chunk == (0, 0) {
            let image = output.in_dimension(Dimension::Nether).unwrap();
            *client = find_chunk(&image.snapshot.chunks, chunk)
                .and_then(|chunk| chunk.chunk.get_block(x as u8, y, z as u8))
                .map(|state| state.0);
        }
        return;
    }
    let body = body_of(packet);
    if packet.id == BlockUpdate::ID {
        let update = BlockUpdate::decode(&mut &body[..]).unwrap();
        if update.position == pack_block_pos(x, y, z) {
            *client = Some(update.state_id as u32);
        }
    } else if packet.id == SectionBlocksUpdate::ID {
        let update = SectionBlocksUpdate::decode(&mut &body[..]).unwrap();
        if update.section_pos == pack_section_pos(x >> 4, y >> 4, z >> 4) {
            for change in update.changes {
                if change.relative_pos == pack_section_relative_pos(x, y, z) {
                    *client = Some(change.state_id as u32);
                }
            }
        }
    } else if packet.id == ForgetLevelChunk::ID {
        let forget = ForgetLevelChunk::decode(&mut &body[..]).unwrap();
        if (forget.chunk_x, forget.chunk_z) == (0, 0) {
            *client = None;
        }
    }
}

/// One viewer's planner and its client's view of the anchor cell.
struct Client {
    viewer: ViewerState,
    anchor: Option<u32>,
    cache: ChunkPacketCache,
}

impl Client {
    /// Plan every published output in order, as the pump does, and apply it.
    fn publish(&mut self, output: TickOutput) -> TickOutput {
        for packet in plan_viewer(&output, &mut self.viewer, 0, &self.cache) {
            apply(&mut self.anchor, &packet, &output);
        }
        output
    }
}

/// Bob holds the Nether chunk with a charged anchor while Alice, dead in the
/// Overworld, respawns at that anchor. The coordinator spends the charge and
/// the next tick's light follows it; Bob's client must end with the spent
/// anchor, not the charged one it held.
#[test]
fn coordinator_respawn_anchor_charge_reaches_a_viewer_holding_the_chunk() {
    let content = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla");
    let registry = Arc::new(CoreRegistry::load(Some(&content)).unwrap());
    let mut root = world(Dimension::Overworld, &registry);
    let mut nether = world(Dimension::Nether, &registry);
    nether
        .world
        .set_block(ANCHOR[0], ANCHOR[1], ANCHOR[2], anchor(&registry, 2));
    nether.world.fold_overlay();
    root.other_dimensions
        .insert(Dimension::Nether, Box::new(nether));
    tick(
        &mut root,
        &[1, 2].map(|id: u64| PlayerInput::Join {
            connection: ConnectionId(id),
            uuid: uuid::Uuid::from_u128(u128::from(id)),
            name: if id == 1 { "Alice" } else { "Bob" }.into(),
            permission_level: 0,
        }),
    );
    let alice = root.connections.entity_of(ConnectionId(1)).unwrap();
    let bob = root.connections.entity_of(ConnectionId(2)).unwrap();
    command(&mut root, 2, "execute in the_nether run tp Bob 8 150 8");
    let player = root.players.get_mut(&alice).unwrap();
    player.respawn_point = Some(ANCHOR);
    player.respawn_dimension = Dimension::Nether;
    player.respawn_forced = false;

    let mut viewer = ViewerState::new(ConnectionId(2), bob, [8.5, 150.0, 8.5]);
    viewer.dimension = Dimension::Nether;
    viewer.chunk_batches.lock().acknowledge(64.0);
    let mut client = Client {
        viewer,
        anchor: None,
        cache: ChunkPacketCache::new(),
    };
    for _ in 0..3 {
        client.publish(tick(&mut root, &[]));
    }
    assert_eq!(
        client.anchor,
        Some(anchor(&registry, 2)),
        "Bob holds the charged anchor"
    );
    client.publish(command(&mut root, 1, "kill"));
    let respawned = client.publish(tick(
        &mut root,
        &[PlayerInput::Respawn {
            connection: ConnectionId(1),
        }],
    ));
    let at = mc_world::chunk::BlockPos {
        x: ANCHOR[0],
        y: ANCHOR[1],
        z: ANCHOR[2],
    };
    assert!(
        respawned
            .in_dimension(Dimension::Nether)
            .unwrap()
            .changes
            .blocks
            .iter()
            .any(|change| change.position == at && change.state == anchor(&registry, 1)),
        "the coordinator spends the charge in the respawn tick"
    );
    for _ in 0..3 {
        client.publish(tick(&mut root, &[]));
    }
    let nether = &root.other_dimensions[&Dimension::Nether];
    assert!(nether.players.contains_key(&alice));
    let server = nether.world.block_at(ANCHOR[0], ANCHOR[1], ANCHOR[2]);
    assert_eq!(server, Some(anchor(&registry, 1)));
    assert_eq!(
        client.anchor, server,
        "Bob's client converges to the spent anchor"
    );
}
