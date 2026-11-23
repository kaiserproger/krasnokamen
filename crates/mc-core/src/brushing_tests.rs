//! Brush reset residency: a non-resident in-progress cell is paused, not
//! reread through the cold overlay and decremented there.

use super::*;
use crate::boundary::{ChunkPos, ConnectionId, PlayerInput};
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

fn state_id(state: &TickState, name: &str, dusted: &str) -> u32 {
    let id = Identifier::parse(format!("minecraft:{name}")).expect("block id");
    let props = [("dusted".to_owned(), dusted.to_owned())];
    state
        .registry
        .blocks
        .by_name_and_props(&id, &props)
        .unwrap_or_else(|| panic!("registered {name} dusted={dusted}"))
        .0
}

fn fixture() -> (TickState, BlockPos) {
    let registry = Arc::new(crate::tick_tests::full_registry());
    let world = ChunkWorld::new(
        42,
        Arc::new(Empty(registry.states.air)),
        &registry,
        mc_world::BlockStateId(registry.states.air),
        [8, 65, 8],
        mc_world::OVERWORLD_GEOMETRY,
    );
    let mut state = TickState::new(registry, world, 0, 512);
    run_tick(
        &mut state,
        &[PlayerInput::Join {
            connection: ConnectionId(1),
            uuid: uuid::Uuid::new_v4(),
            name: "brush".to_owned(),
            permission_level: 0,
        }],
    );
    state.world.ensure_blocking(&state.desired_chunks());
    (state, BlockPos { x: 8, y: 64, z: 8 })
}

#[test]
fn brush_reset_pauses_on_a_cold_cell_instead_of_decrementing_its_overlay() {
    let (mut state, pos) = fixture();
    let clean = state_id(&state, "suspicious_sand", "0");
    let dusted = state_id(&state, "suspicious_sand", "1");
    state.world.set_block(pos.x, pos.y, pos.z, clean);
    state.world.fold_overlay();
    let key = [pos.x, pos.y, pos.z];
    let entry = || Progress {
        count: 5,
        cooldown: 0,
        reset: 0,
    };
    state.brushing.progress.insert(key, entry());
    // Resident cell: the due reset decrements and writes the dusted stage.
    run(&mut state);
    assert_eq!(state.brushing.progress[&key].count, 3);
    assert_eq!(
        state.world.block_at(pos.x, pos.y, pos.z),
        Some(state_id(&state, "suspicious_sand", "2"))
    );
    // Cold cell: the same due reset must pause and leave the overlay alone.
    state.brushing.progress.insert(key, entry());
    let chunk = ChunkPos {
        x: pos.x.div_euclid(16),
        z: pos.z.div_euclid(16),
    };
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
    assert!(!state.world.has_chunk(chunk.x, chunk.z));
    state.world.set_block(pos.x, pos.y, pos.z, dusted);
    for _ in 0..3 {
        state.tick += 4;
        run(&mut state);
        assert_eq!(
            state.brushing.progress[&key].count, 5,
            "a cold cell keeps its progress"
        );
        assert_eq!(
            state.world.block_at(pos.x, pos.y, pos.z),
            Some(dusted),
            "no dusted stage is written into the cold overlay"
        );
    }
}
