use super::*;
use crate::boundary::{ConnectionId, Hand, PlayerInput};
use std::sync::Arc;

struct LootWorld { air: u32, stone: u32, chests: Vec<(BlockPos, u32)>, table: Identifier }
impl mc_world::ChunkGenerator for LootWorld {
    fn generate(&self, pos: mc_world::ChunkPos) -> mc_world::Chunk {
        let mut chunk = mc_world::Chunk::empty(pos, mc_world::BlockStateId(self.air), Identifier::parse("minecraft:plains").unwrap());
        for x in 0..16 { for z in 0..16 { chunk.set_block(x, 63, z, mc_world::BlockStateId(self.stone)); } }
        if pos == (mc_world::ChunkPos { x: 0, z: 0 }) {
            for &(at, block) in &self.chests {
                chunk.set_block(at.x as u8, at.y, at.z as u8, mc_world::BlockStateId(block));
                chunk.chests.insert(at, mc_world::ChestBlockEntity {
                    loot_table: Some(self.table.clone()), loot_seed: i64::from(17 + at.x), ..Default::default()
                });
            }
        }
        chunk
    }
}

fn checkpoint(state: &TickState, store: &mc_world::native::NativeStore, generator: &dyn mc_world::ChunkGenerator) {
    use crate::native_checkpoint::{record, refresh_metadata};
    let mut previous = store.read_checkpoint().unwrap();
    let mut manifest: crate::journal::CheckpointManifest = serde_json::from_slice(&previous.globals["checkpoint"].payload).unwrap();
    manifest.tick = state.tick;
    manifest.world_spawn = Some([4, 64, 2]);
    for child in manifest.dimensions.values_mut() { child.tick = state.tick; }
    previous.globals.insert("checkpoint".into(), record(state.tick, serde_json::to_vec(&manifest).unwrap()));
    refresh_metadata(&mut previous.globals, &manifest).unwrap();
    for global in previous.globals.values_mut() { global.captured_tick = state.tick; }
    let pos = mc_world::ChunkPos { x: 0, z: 0 };
    let base = generator.generate(pos);
    store.commit_checkpoint(mc_world::native::NativeCheckpoint {
        tick: state.tick, previous_tick: previous.tick,
        chunks: &[mc_world::native::NativeChunkSnapshot { dimension: 0, base: &base,
            effective: state.world.chunk(pos).unwrap(), dirty_sections: Default::default() }],
        entities: &[], players: &[], globals: &previous.globals,
    }, &state.registry.blocks, &state.registry.items).unwrap();
}

#[test]
fn data_unopened_loot_metadata_preserves_single_and_double_chests_through_checkpoint_and_wal() {
    for double in [false, true] {
        for restart in ["live", "checkpoint", "wal"] {
            let directory = tempfile::tempdir().unwrap();
            let config = crate::CoreConfig::new(directory.path().to_owned(), 42);
            let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
            let mut registry = crate::registry::CoreRegistry::load(None).unwrap();
            let table = Identifier::parse("test:unopened").unwrap();
            Arc::make_mut(&mut registry.chest_loot).insert(mc_data::loot::chest_26_1_2::ChestLootTable::compile(table.clone(), r#"{
                "type":"minecraft:chest","pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:emerald",
                "functions":[{"function":"minecraft:set_count","count":7}]}]}]}"#).unwrap());
            let mut chests = Vec::new();
            for (x, kind) in if double { vec![(4, "left"), (5, "right")] } else { vec![(4, "single")] } {
                let name = Identifier::parse("minecraft:chest").unwrap();
                let default = registry.blocks.block(&name).unwrap().default;
                let mut properties = registry.blocks.by_id(default).unwrap().properties.clone();
                for (key, value) in &mut properties {
                    if key == "type" { *value = kind.into(); }
                    if key == "facing" { *value = "north".into(); }
                }
                chests.push((BlockPos { x, y: 64, z: 4 }, registry.blocks.by_name_and_props(&name, &properties).unwrap().0));
            }
            let registry = Arc::new(registry);
            let generator: Arc<dyn mc_world::ChunkGenerator> = Arc::new(LootWorld {
                air: registry.states.air, stone: registry.states.stone, chests: chests.clone(), table: table.clone(),
            });
            let mut world = crate::world::ChunkWorld::new_native(42, store.clone(), 0, generator.clone(), &registry,
                mc_world::BlockStateId(registry.states.air), [4, 64, 2], mc_world::OVERWORLD_GEOMETRY);
            let want = [(crate::boundary::ChunkPos { x: 0, z: 0 }, 0)].into_iter().collect();
            world.ensure_blocking(&want);
            let mut state = TickState::new(registry.clone(), world, 0, 512);
            state.tick = 1;
            state.tickets.insert("loot-test".into(), (crate::boundary::ChunkPos { x: 0, z: 0 }, 0));
            state.refresh_active_chunks(state.tick);
            state.gamerules.insert("spawn_mobs".into(), "false".into());
            if restart == "wal" { checkpoint(&state, &store, &*generator); }
            let context = Context::source(&state, crate::command_console::CONNECTION, crate::command_console::EXECUTOR, 2);
            for &(pos, _) in &chests {
                mutate(&mut state, context, &format!("data merge block {} 64 4 {{CustomName:'kept'}}", pos.x)).unwrap();
                assert!(!state.block_entities.contains_key(&(pos.x, pos.y, pos.z)), "metadata cannot materialize empty storage");
                let chest = &state.world.chunk(crate::boundary::ChunkPos { x: 0, z: 0 }).unwrap().chests[&pos];
                assert_eq!(chest.loot_table.as_ref(), Some(&table));
                assert_eq!(chest.loot_seed, i64::from(17 + pos.x));
            }
            if restart == "checkpoint" { checkpoint(&state, &store, &*generator); }
            if restart == "wal" {
                let (output, _, _) = crate::tick::run_tick(&mut state, &[]);
                let entries = crate::core::journal_entries_for_tick(&mut state, &output);
                let mut writer = crate::journal::JournalWriter::open(directory.path(), 1).unwrap();
                writer.append(output.tick, output.tick, &entries).unwrap();
                writer.flush().unwrap();
            }
            if restart != "live" {
                drop(state);
                drop(store);
                let mut recovered = crate::native_checkpoint::recover_offline(&config).unwrap();
                let replay = recovered.dimensions.remove(&crate::dimensions::Dimension::Overworld).unwrap();
                let tick = replay.tick.max(recovered.manifest.tick);
                drop(recovered);
                let reopened = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
                let mut world = crate::world::ChunkWorld::new_native(42, reopened, 0, generator.clone(), &registry,
                    mc_world::BlockStateId(registry.states.air), [4, 64, 2], mc_world::OVERWORLD_GEOMETRY);
                world.ensure_blocking(&want);
                // Use the production replay postimage installer, never synthesize a typed loot shadow.
                for ((x, y, z), bytes) in replay.opaque_block_entities {
                    world.set_opaque_block_entity(BlockPos { x, y, z }, bytes);
                }
                state = TickState::new(registry.clone(), world, 0, 512);
                state.tick = tick;
                state.gamerules.insert("spawn_mobs".into(), "false".into());
            }
            crate::tick::run_tick(&mut state, &[PlayerInput::Join {
                connection: ConnectionId(1), uuid: uuid::Uuid::new_v4(), name: "opener".into(), permission_level: 2,
            }]);
            let actor = state.connections.entity_of(ConnectionId(1)).unwrap();
            let slot = state.entities.slot_of(actor).unwrap();
            state.entities.set_pos(slot, [4.5, 64.0, 2.5]);
            let input = PlayerInput::UseItemOn { connection: ConnectionId(1), hand: Hand::Main,
                position: BlockPos { x: 4, y: 64, z: 4 }, face: 1, cursor: [0.5; 3], sequence: 1 };
            crate::tick::run_tick(&mut state, &[input.clone()]);
            assert!(state.players[&actor].open.is_some(), "{double}/{restart}: native first open");
            let emerald = item(&state, "minecraft:emerald", 1).item_id;
            for &(pos, _) in &chests {
                let inventory = &state.block_entities[&(pos.x, pos.y, pos.z)];
                assert_eq!(inventory.slots.iter().filter(|stack| stack.item_id == emerald).map(|stack| stack.count).sum::<i32>(), 7,
                    "{double}/{restart}: each half rolls exactly its original table");
                assert_eq!(*get(&block(&state, pos).unwrap(), "CustomName").unwrap(), Tag::String("kept".into()));
            }
            let before: Vec<_> = chests.iter().map(|(pos, _)| state.block_entities[&(pos.x, pos.y, pos.z)].slots.clone()).collect();
            crate::tick::run_tick(&mut state, &[input]);
            for ((pos, _), before) in chests.iter().zip(before) {
                assert_eq!(state.block_entities[&(pos.x, pos.y, pos.z)].slots, before, "second open must not roll again");
            }
        }
    }
}
