//! Menu-coordinate writes: an anvil menu's stored cell outlives its chunk, so
//! `damage_anvil` must ask residency before it reads or writes that cell.

use super::stations::damage_anvil;
use crate::boundary::{BlockPos, ChangeSet, ChunkPos, ConnectionId, PlayerInput};
use crate::tick::{TickState, run_tick};
use crate::world::ChunkWorld;
use mc_data::Identifier;
use std::sync::Arc;

struct Empty(u32);
impl mc_world::ChunkGenerator for Empty {
    fn generate(&self, pos: mc_world::ChunkPos) -> mc_world::Chunk {
        mc_world::Chunk::empty(
            pos,
            mc_world::BlockStateId(self.0),
            Identifier::parse("minecraft:plains").unwrap(),
        )
    }
}

fn block_state(name: &str) -> u32 {
    let registry = crate::registry::CoreRegistry::load(None).unwrap();
    registry
        .blocks
        .states()
        .find(|entry| entry.block.id.as_str() == name)
        .unwrap()
        .id
        .0
}

fn fixture() -> (TickState, crate::boundary::EntityId) {
    let registry = Arc::new(crate::registry::CoreRegistry::load(None).unwrap());
    let world = ChunkWorld::new(
        42,
        Arc::new(Empty(registry.states.air)),
        &registry,
        mc_world::BlockStateId(registry.states.air),
        [8, 65, 8],
        mc_world::OVERWORLD_GEOMETRY,
    );
    let mut state = TickState::new(registry, world, 2, 512);
    run_tick(
        &mut state,
        &[PlayerInput::Join {
            connection: ConnectionId(1),
            uuid: uuid::Uuid::new_v4(),
            name: "anvil".to_owned(),
            permission_level: 0,
        }],
    );
    state.world.ensure_blocking(&state.desired_chunks());
    let actor = state.connections.entity_of(ConnectionId(1)).unwrap();
    (state, actor)
}

#[test]
fn anvil_damage_waits_for_a_resident_menu_coordinate() {
    let (mut state, actor) = fixture();
    let anvil = block_state("minecraft:anvil");
    let chipped = block_state("minecraft:chipped_anvil");
    let cell = BlockPos { x: 8, y: 65, z: 8 };
    state.world.set_block(cell.x, cell.y, cell.z, anvil);
    state.world.fold_overlay();
    state
        .open_containers
        .insert(actor, (cell.x, cell.y, cell.z));
    let chunk = ChunkPos { x: 0, z: 0 };
    state.world.finish_inhabited_tick(state.tick + 1);
    state
        .world
        .set_active_chunks(Default::default(), state.tick + 1);
    state.world.capture_dirty(&Default::default());
    state
        .world
        .mark_flushed(&[(chunk, state.world.storage_revision(chunk))]);
    assert!(
        state
            .world
            .evict_outside(&Default::default())
            .contains(&chunk)
    );
    assert!(!state.world.has_chunk(0, 0));
    // A later write leaves an anvil-family image in the cold overlay; the old
    // code would damage it (0.12 per tick) and write the next stage into it.
    state.world.set_block(cell.x, cell.y, cell.z, chipped);
    assert_eq!(state.world.block_at(cell.x, cell.y, cell.z), Some(chipped));
    for _ in 0..200 {
        state.tick += 1;
        let mut changes = ChangeSet::default();
        damage_anvil(&mut state, actor, &mut changes);
        assert!(
            changes.blocks.is_empty(),
            "damage wrote a cold menu coordinate into the overlay"
        );
    }
    assert_eq!(state.world.block_at(cell.x, cell.y, cell.z), Some(chipped));
}
