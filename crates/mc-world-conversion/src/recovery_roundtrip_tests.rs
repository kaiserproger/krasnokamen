use mc_core::dimensions::Dimension;
use mc_core::native_checkpoint as bridge;
use mc_data::Identifier;
use mc_nbt::Tag;
use mc_world::native::NativeCheckpoint;
use std::sync::Arc;

#[path = "live_capture_tests.rs"]
mod live_capture;

#[tokio::test]
async fn imported_bee_hive_release_checkpoints_after_kill_and_unload() {
    use crate::test_support::Running;
    use mc_core::boundary::PlayerInput;
    use mc_core::command_console::ConsoleCommand;
    use mc_world::{BlockStateId, Chunk, ChunkPos};

    async fn command(runtime: &Running, text: String) {
        let (command, completed) = ConsoleCommand::new(text);
        assert!(runtime.core.submit(PlayerInput::ConsoleCommand(command)));
        tokio::time::timeout(std::time::Duration::from_secs(60), completed)
            .await
            .expect("console command tick")
            .expect("live console acknowledgement");
    }

    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("fixture");
    let anvil = temporary.path().join("anvil");
    let imported = temporary.path().join("imported");
    let source_config = crate::test_support::config(source.clone(), 42);
    let registry = Arc::new(crate::test_support::registry().unwrap());
    let store = crate::test_support::fixture_store(&source_config).unwrap();
    let mut chunk = Chunk::empty(
        ChunkPos { x: 0, z: 0 },
        BlockStateId(registry.states.air),
        Identifier::parse("minecraft:plains").unwrap(),
    );
    for x in 0..16 {
        for z in 0..16 {
            chunk
                .set_block(x, 200, z, BlockStateId(registry.states.stone))
                .unwrap();
        }
    }
    let hive = registry
        .blocks
        .block(&Identifier::parse("minecraft:beehive").unwrap())
        .unwrap()
        .default;
    chunk.set_block(4, 201, 4, hive).unwrap();
    store
        .fill_generated(0, &[chunk], &registry.blocks, &registry.items)
        .unwrap();
    let mut recovered = bridge::recover_with_reader(
        &source_config,
        Arc::clone(&registry),
        store.checkpoint_reader().unwrap(),
    )
    .unwrap();
    for replay in recovered.dimensions.values_mut() {
        replay.tick = 1;
    }
    let bee_uuid = uuid::Uuid::from_u128(0xbee);
    let cow_uuid = uuid::Uuid::from_u128(0xc01);
    let summoned_uuid = uuid::Uuid::from_u128(0x711);
    for (wire_id, id, uuid, pos, extra_fields) in [
        (
            1,
            "minecraft:bee",
            bee_uuid,
            [4.5, 201.0, 4.5],
            vec![
                ("NoAI".into(), Tag::Byte(1)),
                ("HasNectar".into(), Tag::Byte(1)),
                ("hive_pos".into(), Tag::IntArray(vec![4, 201, 4])),
            ],
        ),
        (
            2,
            "minecraft:cow",
            cow_uuid,
            [6.5, 201.0, 6.5],
            vec![("NoAI".into(), Tag::Byte(1))],
        ),
    ] {
        let entity = mc_data::entity_nbt::VanillaEntity {
            id: id.into(),
            uuid,
            wire_id,
            pos,
            health: 20.0,
            extra_fields,
            ..Default::default()
        };
        let payload =
            Tag::Compound(mc_data::entity_nbt::entity_to_fields(&registry.items, &entity).unwrap());
        bridge::hydrate_entity(
            &registry,
            recovered.dimensions.get_mut(&Dimension::Overworld).unwrap(),
            &payload,
            1,
        )
        .unwrap();
    }
    let image = bridge::encode_checkpoint(&recovered).unwrap();
    drop(recovered);
    store
        .commit_checkpoint(
            NativeCheckpoint {
                tick: 1,
                previous_tick: 0,
                chunks: &[],
                entities: &image.entities,
                players: &image.players,
                globals: &image.globals,
            },
            &registry.blocks,
            &registry.items,
        )
        .unwrap();
    drop(store);
    crate::export_anvil(
        &source_config,
        &source,
        &anvil,
        &temporary.path().join("export.json"),
    )
    .unwrap();
    let config = crate::test_support::config(imported.clone(), 42);
    let receipt = crate::import_anvil(
        &config,
        &anvil,
        &imported,
        &temporary.path().join("import.json"),
    )
    .unwrap();
    assert_eq!(receipt.counts["entities"], 2);

    let mut runtime = Running::open(config.clone());
    command(&runtime, "forceload add 0 0".into()).await;
    runtime
        .until(|output, _| {
            let entities = &output.snapshot.entities;
            (output
                .snapshot
                .chunks
                .iter()
                .any(|chunk| chunk.pos == (ChunkPos { x: 0, z: 0 }))
                && entities.iter().any(|entity| entity.uuid == cow_uuid)
                && !entities.iter().any(|entity| entity.uuid == bee_uuid))
            .then_some(())
        })
        .await;
    command(&runtime, format!("kill {cow_uuid}")).await;
    command(
        &runtime,
        format!("summon minecraft:villager 8.5 201 8.5 {{UUID:\"{summoned_uuid}\",NoAI:1b}}"),
    )
    .await;
    runtime
        .until(|output, _| {
            let entities = &output.snapshot.entities;
            (!entities.iter().any(|entity| entity.uuid == cow_uuid)
                && entities.iter().any(|entity| entity.uuid == summoned_uuid))
            .then_some(())
        })
        .await;
    assert!(runtime.core.checkpoint());
    drop(runtime);

    // A new runtime owns only the checkpointed hive; the old bee's dense slot
    // and the removed cow cannot leak into its full entity capture.
    let mut runtime = Running::open(config.clone());
    runtime
        .until(|output, _| {
            output
                .snapshot
                .chunks
                .iter()
                .any(|chunk| chunk.pos == (ChunkPos { x: 0, z: 0 }))
                .then_some(())
        })
        .await;
    command(&runtime, "setblock 4 201 4 minecraft:air destroy".into()).await;
    let bee_type = registry
        .entity_types
        .id_of(&Identifier::parse("minecraft:bee").unwrap())
        .unwrap();
    let released_uuid = runtime
        .until(|output, _| {
            output
                .snapshot
                .entities
                .iter()
                .find(|entity| entity.type_id == bee_type)
                .map(|entity| entity.uuid)
        })
        .await;
    assert_ne!(released_uuid, bee_uuid);
    assert!(runtime.core.checkpoint());
    drop(runtime);
    let store = crate::test_support::fixture_store(&config).unwrap();
    for uuid in [bee_uuid, cow_uuid] {
        assert!(
            store
                .entity(*uuid.as_bytes())
                .unwrap()
                .unwrap()
                .payload
                .is_none()
        );
    }
    for uuid in [released_uuid, summoned_uuid] {
        assert!(
            store
                .entity(*uuid.as_bytes())
                .unwrap()
                .unwrap()
                .payload
                .is_some()
        );
    }
    drop(store);
    let recovered = bridge::recover_offline(&config).unwrap();
    let entities = &recovered.dimensions[&Dimension::Overworld].entities;
    let released = entities
        .values()
        .find(|entity| entity.uuid == released_uuid.to_string())
        .unwrap();
    assert!(released.state.controls.no_ai);
    assert!(
        entities
            .values()
            .any(|entity| entity.uuid == summoned_uuid.to_string())
    );
    for uuid in [bee_uuid, cow_uuid] {
        assert!(
            !entities
                .values()
                .any(|entity| entity.uuid == uuid.to_string())
        );
    }
}
