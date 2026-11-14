#[path = "live_capture_tests.rs"]
mod live_capture;

use mc_core::dimensions::Dimension;
use mc_core::native_checkpoint as bridge;
use mc_data::{Identifier, ItemStack};
use mc_nbt::Tag;
use mc_world::native::NativeCheckpoint;
use std::sync::Arc;

fn entity_entry(id: i32, entity: mc_core::journal::ReplayEntity) -> mc_core::journal::JournalEntry {
    mc_core::journal::JournalEntry::Entity {
        id,
        uuid: entity.uuid,
        type_id: entity.type_id,
        health: entity.health,
        alive: entity.alive,
        age: entity.age,
        breed_cooldown: entity.breed_cooldown,
        pickup_delay: entity.pickup_delay,
        item: entity.item,
        appearance: entity.appearance,
        active_effects: entity.active_effects,
        villager: entity.villager,
        state: Arc::new(entity.state),
        projectile: entity.projectile,
        tick: entity.tick,
        pos: entity.pos,
        vel: entity.vel,
        yaw: entity.yaw,
        pitch: entity.pitch,
    }
}

#[test]
fn wandering_trader_checkpoint_and_wal_tail_survive_anvil_roundtrip() {
    use mc_core::journal::{JournalEntry, JournalWriter};
    use mc_core::villager::{VillagerState, VillagerTradeOffer};

    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("native-source");
    let anvil = temporary.path().join("anvil");
    let imported = temporary.path().join("native-imported");
    let config = crate::test_support::config(source.clone(), 42);
    let registry = Arc::new(crate::test_support::registry().unwrap());
    let store = crate::test_support::fixture_store(&config).unwrap();
    let mut recovered = bridge::recover_with_reader(
        &config,
        Arc::clone(&registry),
        store.checkpoint_reader().unwrap(),
    )
    .unwrap();
    let tick = 24_000;
    for replay in recovered.dimensions.values_mut() {
        replay.tick = tick;
    }
    let replay = recovered.dimensions.get_mut(&Dimension::Overworld).unwrap();
    let owner = uuid::Uuid::from_u128(0x761);
    for (wire_id, id) in [
        (1, "minecraft:wandering_trader"),
        (2, "minecraft:trader_llama"),
        (3, "minecraft:trader_llama"),
    ] {
        let entity = mc_data::entity_nbt::VanillaEntity {
            id: id.into(),
            uuid: uuid::Uuid::from_u128(0x760 + wire_id as u128),
            wire_id,
            pos: [wire_id as f64, 80.0, 1.0],
            health: 20.0,
            ..Default::default()
        };
        let payload =
            Tag::Compound(mc_data::entity_nbt::entity_to_fields(&registry.items, &entity).unwrap());
        bridge::hydrate_entity(&registry, replay, &payload, tick).unwrap();
    }
    let item = |name: &str| {
        registry
            .items
            .id_of(&Identifier::parse(name).unwrap())
            .unwrap()
    };
    let mut expected = VillagerState::default();
    expected.wandering = true;
    expected.data_finalized = true;
    expected.despawn_at = Some(tick + 48_000);
    expected.offers.push(VillagerTradeOffer {
        cost_a: ItemStack::new(item("minecraft:emerald"), 5),
        cost_b: None,
        result: ItemStack::new(item("minecraft:blue_ice"), 1),
        uses: 0,
        max_uses: 6,
        reward_exp: true,
        xp: 1,
        special_price: 0,
        demand: 0,
        price_multiplier: 0.05,
    });
    replay.entities.get_mut(&1).unwrap().villager = Some(Box::new(expected.clone()));
    for id in [2, 3] {
        replay
            .entities
            .get_mut(&id)
            .unwrap()
            .state
            .entity_life
            .leash = Some(mc_core::leash::Holder::Entity(owner));
    }
    let clock = replay.wandering_trader.get_or_insert_with(Default::default);
    clock.delay = 12_345;
    clock.chance = 50;
    let clock = *clock;
    let image = bridge::encode_checkpoint(&recovered).unwrap();
    let mut trader = recovered
        .dimensions
        .get_mut(&Dimension::Overworld)
        .unwrap()
        .entities
        .remove(&1)
        .unwrap();
    drop(recovered);
    store
        .commit_checkpoint(
            NativeCheckpoint {
                tick,
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

    // The changed offer and spawn clock must come from replay, not the old
    // checkpoint. Simulation capture/despawn is tested by mc-core's owner test.
    expected.offers[0].uses = 1;
    trader.villager = Some(Box::new(expected.clone()));
    trader.tick = tick + 1;
    let mut clock = clock;
    clock.delay -= 1;
    let mut writer = JournalWriter::open(&source, 1).unwrap();
    writer
        .append(
            tick + 1,
            tick + 1,
            &[
                JournalEntry::WanderingTrader { value: clock },
                entity_entry(1, trader),
            ],
        )
        .unwrap();
    writer.flush().unwrap();
    drop(writer);
    crate::export_anvil(
        &config,
        &source,
        &anvil,
        &temporary.path().join("export.json"),
    )
    .unwrap();
    crate::import_anvil(
        &config,
        &anvil,
        &imported,
        &temporary.path().join("import.json"),
    )
    .unwrap();
    let imported_config = crate::test_support::config(imported, 42);
    let recovered = bridge::recover_offline(&imported_config).unwrap();
    let replay = &recovered.dimensions[&Dimension::Overworld];
    assert_eq!(replay.wandering_trader, Some(clock));
    assert_eq!(replay.entities.len(), 3);
    let owner_text = owner.to_string();
    let trader = replay
        .entities
        .values()
        .find(|entity| entity.uuid == owner_text)
        .unwrap();
    assert_eq!(trader.villager.as_deref(), Some(&expected));
    for entity in replay
        .entities
        .values()
        .filter(|entity| entity.uuid != owner_text)
    {
        assert_eq!(
            entity.state.entity_life.leash,
            Some(mc_core::leash::Holder::Entity(owner))
        );
    }
}

#[test]
fn lazy_warning_decay_exports_the_replayed_clock_without_native_anchors() {
    use mc_core::journal::{JournalWriter, player_from_tag};

    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("native-source");
    let exported = temporary.path().join("export");
    let config = crate::test_support::config(source.clone(), 42);
    let registry = Arc::new(crate::test_support::registry().unwrap());
    let store = crate::test_support::fixture_store(&config).unwrap();
    let mut recovered = bridge::recover_with_reader(
        &config,
        Arc::clone(&registry),
        store.checkpoint_reader().unwrap(),
    )
    .unwrap();
    let uuid = uuid::Uuid::from_u128(0x5741524e);
    let mut player = player_from_tag(&Tag::Compound(Vec::new()), &registry.items).unwrap();
    player.warden_warning = serde_json::from_value(serde_json::json!({
        "ticks_since_last_warning": 0,
        "warning_level": 3,
        "cooldown_ticks": 200,
        "base_tick": 10,
        "active": true
    }))
    .unwrap();
    player.tick = 6010;
    for replay in recovered.dimensions.values_mut() {
        replay.tick = 6010;
    }
    recovered
        .dimensions
        .get_mut(&Dimension::Overworld)
        .unwrap()
        .players
        .insert(uuid.to_string(), player);
    let image = bridge::encode_checkpoint(&recovered).unwrap();
    drop(recovered);
    store
        .commit_checkpoint(
            NativeCheckpoint {
                tick: 6010,
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
    let mut writer = JournalWriter::open(&source, 1).unwrap();
    writer.append(6011, 12010, &[]).unwrap();
    writer.append(12011, 12011, &[]).unwrap();
    writer.flush().unwrap();
    drop(writer);
    crate::export_anvil(
        &config,
        &source,
        &exported,
        &temporary.path().join("export.json"),
    )
    .unwrap();
    let tag = crate::read_nbt(&exported.join(format!("players/data/{uuid}.dat"))).unwrap();
    let Tag::Compound(fields) = &tag else {
        panic!("exported player compound")
    };
    assert!(!fields.iter().any(|(key, _)| key == "SolarisWardenWarning"));
    let player = player_from_tag(&tag, &registry.items).unwrap();
    let warning = player.warden_warning;
    assert_eq!(
        (
            warning.ticks_since_last_warning(),
            warning.warning_level(),
            warning.cooldown_ticks()
        ),
        (0, 2, 0),
    );
}

#[test]
fn villager_wal_claim_exports_native_profession_and_unique_job_memory() {
    use mc_core::boundary::BlockPos;
    use mc_core::journal::JournalWriter;
    use mc_core::villager::VillagerProfession;

    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("native-source");
    let anvil = temporary.path().join("anvil");
    let imported = temporary.path().join("native-imported");
    let config = crate::test_support::config(source.clone(), 42);
    let registry = Arc::new(crate::test_support::registry().unwrap());
    let store = crate::test_support::fixture_store(&config).unwrap();
    let mut recovered = bridge::recover_with_reader(
        &config,
        Arc::clone(&registry),
        store.checkpoint_reader().unwrap(),
    )
    .unwrap();
    for replay in recovered.dimensions.values_mut() {
        replay.tick = 1;
    }
    let uuid = uuid::Uuid::from_u128(0x706f69);
    let position = [3.322834405812458, 201.0, 3.057086014531166];
    let entity = mc_data::entity_nbt::VanillaEntity {
        id: "minecraft:villager".into(),
        uuid,
        wire_id: 1,
        pos: position,
        health: 20.0,
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
    let image = bridge::encode_checkpoint(&recovered).unwrap();
    let mut entity = recovered
        .dimensions
        .get_mut(&Dimension::Overworld)
        .unwrap()
        .entities
        .remove(&1)
        .unwrap();
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
    let workstation = BlockPos { x: 4, y: 201, z: 4 };
    let villager = entity.villager.as_mut().unwrap();
    villager.job_site = Some(workstation);
    villager.job_site_dimension = "minecraft:overworld".into();
    villager.data.profession = VillagerProfession::Farmer;
    entity.tick = 2;
    entity.pos = [4.0, 201.0, 4.0];
    let mut writer = JournalWriter::open(&source, 1).unwrap();
    writer.append(2, 2, &[entity_entry(1, entity)]).unwrap();
    writer.flush().unwrap();
    drop(writer);
    crate::export_anvil(
        &config,
        &source,
        &anvil,
        &temporary.path().join("export.json"),
    )
    .unwrap();
    let regions = mc_world::anvil::read_entity_regions(
        &Dimension::Overworld.directory(&anvil).join("entities"),
        &registry.items,
    )
    .unwrap();
    let mut entities = regions.values().flat_map(|chunk| &chunk.entities);
    let entity = entities.next().unwrap();
    assert!(entities.next().is_none());
    assert_eq!(entity.uuid, uuid);
    assert_eq!(
        entity.pos, position,
        "ordinary motion stays checkpoint-durable"
    );
    let fields = &entity.extra_fields;
    let Some(Tag::Compound(data)) = mc_data::item_nbt::field(fields, "VillagerData") else {
        panic!("VillagerData must be a native compound");
    };
    assert!(data.contains(&("profession".into(), Tag::String("minecraft:farmer".into()))));
    let Some(Tag::Compound(brain)) = mc_data::item_nbt::field(fields, "Brain") else {
        panic!("Brain must be a native compound");
    };
    let Some(Tag::Compound(memories)) = mc_data::item_nbt::field(brain, "memories") else {
        panic!("Brain memories must be a native compound");
    };
    assert_eq!(
        memories
            .iter()
            .filter(|(key, _)| key == "minecraft:job_site")
            .count(),
        1
    );
    crate::import_anvil(
        &config,
        &anvil,
        &imported,
        &temporary.path().join("import.json"),
    )
    .unwrap();
    let recovered = bridge::recover_offline(&crate::test_support::config(imported, 42)).unwrap();
    let replay = &recovered.dimensions[&Dimension::Overworld];
    assert_eq!(replay.entities.len(), 1);
    let entity = replay.entities.values().next().unwrap();
    assert_eq!(entity.uuid, uuid.to_string());
    let villager = entity.villager.as_ref().unwrap();
    assert_eq!(villager.data.profession, VillagerProfession::Farmer);
    assert_eq!(villager.job_site, Some(workstation));
    assert_eq!(villager.job_site_dimension, "minecraft:overworld");
}

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
