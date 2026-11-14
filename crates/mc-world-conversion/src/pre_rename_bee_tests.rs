use super::*;
use mc_core::dimensions::Dimension;
use mc_core::native_checkpoint as bridge;
use mc_data::Identifier;
use std::sync::Arc;

// Serialized by v1.0.0-rc.5 (d3d2abdfbd8f5b6277e71a5708a000721c452ab2),
// dimensions::recovery::hydrate_entity -> serde_json::to_string(ReplayEntity).
const RC5_BEE: &str = include_str!("fixtures/rc5-bee-hive.json");

fn current_bee_image() -> String {
    let mut image: serde_json::Value = serde_json::from_str(RC5_BEE).unwrap();
    let mount = image["state"]["entity_life"]["mount"]
        .as_object_mut()
        .unwrap();
    assert_eq!(mount.remove("rider"), Some(serde_json::Value::Null));
    mount.insert("riders".into(), serde_json::json!([]));
    serde_json::to_string(&image).unwrap()
}

#[test]
fn captured_rc5_bee_image_requires_migration_and_keeps_strict_current_schema() {
    let error = serde_json::from_str::<mc_core::journal::ReplayEntity>(RC5_BEE).unwrap_err();
    assert!(error.to_string().contains("rider"), "{error}");
    let mut record = compound(vec![("solaris:HiveEntity", Tag::String(RC5_BEE.into()))]);
    PreRename::default()
        .rename(&mut record, Owner::Entity, &|| "captured RC5 bee".into())
        .unwrap();
    assert_eq!(
        record,
        compound(vec![(
            "krasnokamen:HiveEntity",
            Tag::String(current_bee_image())
        )])
    );
    let bee: mc_core::journal::ReplayEntity = serde_json::from_str(&current_bee_image()).unwrap();
    assert!(bee.state.controls.no_ai);
    assert_eq!(bee.health, 7.0);
    let mut unknown: serde_json::Value = serde_json::from_str(&current_bee_image()).unwrap();
    unknown["state"]["entity_life"]["mount"]["unknown_native_field"] = true.into();
    assert!(serde_json::from_value::<mc_core::journal::ReplayEntity>(unknown).is_err());
}

#[test]
fn hive_image_rejects_conflicting_and_invalid_legacy_riders() {
    for (rider, conflict) in [
        (serde_json::json!(7), false),
        (serde_json::Value::Null, true),
    ] {
        let mut image: serde_json::Value = serde_json::from_str(RC5_BEE).unwrap();
        image["state"]["entity_life"]["mount"]["rider"] = rider;
        if conflict {
            image["state"]["entity_life"]["mount"]["riders"] = serde_json::json!([]);
        }
        let mut record = compound(vec![(
            "solaris:HiveEntity",
            Tag::String(serde_json::to_string(&image).unwrap()),
        )]);
        let error = PreRename::default()
            .rename(&mut record, Owner::Entity, &|| "bad RC5 hive".into())
            .unwrap_err();
        assert!(error.to_string().contains("bad RC5 hive"), "{error}");
    }
}

fn check_player_item(input: Tag, expected: Tag) {
    let mut input = input;
    PreRename::default()
        .rename(&mut input, Owner::Player, &|| "player custody".into())
        .unwrap();
    assert_eq!(input, expected);
}

#[test]
fn player_equipment_hive_items_are_owned() {
    for slot in ["head", "chest", "legs", "feet", "mainhand", "offhand"] {
        check_player_item(
            compound(vec![(
                "equipment",
                compound(vec![(slot, hive_item(RC5_BEE.into(), true))]),
            )]),
            compound(vec![(
                "equipment",
                compound(vec![(slot, hive_item(current_bee_image(), false))]),
            )]),
        );
    }
}

#[test]
fn player_carried_hive_items_are_owned() {
    for key in ["solaris:Carried", "krasnokamen:Carried"] {
        check_player_item(
            compound(vec![(key, hive_item(RC5_BEE.into(), true))]),
            compound(vec![(
                "krasnokamen:Carried",
                hive_item(current_bee_image(), false),
            )]),
        );
    }
}

#[test]
fn player_crafting_hive_items_are_owned() {
    for key in ["solaris:CraftingGrid", "krasnokamen:CraftingGrid"] {
        let grid = |item| {
            Tag::List(ListTag {
                element_type: tag_type::COMPOUND,
                elements: vec![item],
            })
        };
        check_player_item(
            compound(vec![(key, grid(hive_item(RC5_BEE.into(), true)))]),
            compound(vec![(
                "krasnokamen:CraftingGrid",
                grid(hive_item(current_bee_image(), false)),
            )]),
        );
    }
}

fn occupants(image: String, legacy: bool) -> Tag {
    Tag::List(ListTag {
        element_type: tag_type::COMPOUND,
        elements: vec![compound(vec![
            (
                "entity_data",
                compound(vec![
                    ("id", Tag::String("minecraft:bee".into())),
                    (
                        if legacy {
                            "solaris:HiveEntity"
                        } else {
                            "krasnokamen:HiveEntity"
                        },
                        Tag::String(image),
                    ),
                ]),
            ),
            ("ticks_in_hive", Tag::Int(0)),
            ("min_ticks_in_hive", Tag::Int(2400)),
        ])],
    })
}

fn hive_item(image: String, legacy: bool) -> Tag {
    compound(vec![
        ("id", Tag::String("minecraft:beehive".into())),
        ("count", Tag::Int(1)),
        (
            "components",
            compound(vec![
                ("minecraft:bees", occupants(image, legacy)),
                ("my_mod:bees", foreign_payload()),
            ]),
        ),
    ])
}

#[test]
fn bee_item_components_are_owned_inside_inventories_containers_and_item_entities() {
    for (owner, key) in [
        (Owner::Player, "Inventory"),
        (Owner::BlockEntity, "Items"),
        (Owner::Entity, "Item"),
    ] {
        let item = hive_item(RC5_BEE.into(), true);
        let original = item.clone();
        let mut record = compound(vec![(key, item.clone())]);
        let mut report = PreRename::default();
        report
            .rename(&mut record, owner, &|| "typed item".into())
            .unwrap();
        assert_eq!(report.keys, 2);
        assert_eq!(
            record,
            compound(vec![(key, hive_item(current_bee_image(), false))])
        );
        // The same shape under an unrelated component is not an entity owner.
        let mut opaque = compound(vec![("my_mod:inventory", original)]);
        let expected = opaque.clone();
        PreRename::default()
            .rename(&mut opaque, owner, &|| "opaque item".into())
            .unwrap();
        assert_eq!(opaque, expected);
    }
}

async fn command(runtime: &crate::test_support::Running, text: &str) {
    let (command, completed) = mc_core::command_console::ConsoleCommand::new(text.into());
    assert!(
        runtime
            .core
            .submit(mc_core::boundary::PlayerInput::ConsoleCommand(command))
    );
    tokio::time::timeout(std::time::Duration::from_secs(60), completed)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn rc5_hive_item_export_import_place_release_preserves_native_bee() {
    use mc_core::boundary::{BlockPos, ConnectionId, Hand, PlayerInput};
    use mc_world::native::NativeCheckpoint;
    use mc_world::{BlockStateId, Chunk, ChunkPos};
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let anvil = temp.path().join("rc5-export");
    let imported = temp.path().join("imported");
    let config = crate::test_support::config(source.clone(), 42);
    let registry = Arc::new(crate::test_support::registry().unwrap());
    let store = crate::test_support::fixture_store(&config).unwrap();
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
    store
        .fill_generated(0, &[chunk], &registry.blocks, &registry.items)
        .unwrap();
    let mut recovered = bridge::recover_with_reader(
        &config,
        Arc::clone(&registry),
        store.checkpoint_reader().unwrap(),
    )
    .unwrap();
    for replay in recovered.dimensions.values_mut() {
        replay.tick = 1;
    }
    let overworld = recovered.dimensions.get_mut(&Dimension::Overworld).unwrap();
    // The native stack carries actual RC5 serializer output, not a current
    // image disguised only by its outer NBT key. Export keeps this JSON opaque.
    let item = hive_item(RC5_BEE.into(), false);
    let Tag::Compound(item_fields) = item else {
        unreachable!()
    };
    let stack = mc_data::item_nbt::stack_from_fields(&registry.items, &item_fields)
        .unwrap()
        .unwrap();
    let fields = mc_data::player_nbt::player_to_fields(
        &registry.items,
        &mc_data::player_nbt::VanillaPlayer {
            pos: [4.5, 201.0, 2.5],
            inventory: vec![(0, stack)],
            ..Default::default()
        },
    )
    .unwrap();
    let mut player =
        mc_core::journal::player_from_tag(&Tag::Compound(fields), &registry.items).unwrap();
    player.tick = 1;
    let player_uuid = uuid::Uuid::from_u128(0x50);
    overworld.players.insert(player_uuid.to_string(), player);
    let checkpoint = bridge::encode_checkpoint(&recovered).unwrap();
    drop(recovered);
    store
        .commit_checkpoint(
            NativeCheckpoint {
                tick: 1,
                previous_tick: 0,
                chunks: &[],
                entities: &checkpoint.entities,
                players: &checkpoint.players,
                globals: &checkpoint.globals,
            },
            &registry.blocks,
            &registry.items,
        )
        .unwrap();
    drop(store);
    crate::export_anvil(&config, &source, &anvil, &temp.path().join("export.json")).unwrap();
    let player_file = format!("players/data/{player_uuid}.dat");
    let mut exported = crate::read_nbt(&anvil.join(&player_file)).unwrap();
    fn downgrade(tag: &mut Tag) -> usize {
        match tag {
            Tag::Compound(fields) => fields
                .iter_mut()
                .map(|(key, value)| {
                    let own = usize::from(key == "krasnokamen:HiveEntity");
                    if own != 0 {
                        *key = "solaris:HiveEntity".into();
                    }
                    own + downgrade(value)
                })
                .sum(),
            Tag::List(list) => list.elements.iter_mut().map(downgrade).sum(),
            _ => 0,
        }
    }
    assert_eq!(downgrade(&mut exported), 1);
    crate::roundtrip_tests::save(&anvil, &player_file, &exported);
    let imported_config = crate::test_support::config(imported.clone(), 42);
    let receipt = crate::import_anvil(
        &imported_config,
        &anvil,
        &imported,
        &temp.path().join("import.json"),
    )
    .unwrap();
    assert_eq!(receipt.status, "PASS");
    let before = bridge::recover_offline(&imported_config).unwrap();
    let saved = &before.dimensions[&Dimension::Overworld].players[&player_uuid.to_string()];
    assert_eq!(saved.inventory[0].count, 1);
    assert_eq!(
        registry
            .items
            .name_of(saved.inventory[0].item_id)
            .unwrap()
            .to_string(),
        "minecraft:beehive"
    );
    assert_eq!(saved.selected, 0);
    drop(before);
    let mut runtime = crate::test_support::Running::open(imported_config.clone());
    assert!(runtime.core.submit(PlayerInput::Join {
        connection: ConnectionId(1),
        uuid: player_uuid,
        name: "BeeKeeper".into(),
        permission_level: 4,
    }));
    command(&runtime, "forceload add 0 0").await;
    let position = runtime
        .until(|out, _| {
            out.snapshot
                .entities
                .iter()
                .find(|e| e.uuid == player_uuid)
                .map(|e| e.position)
                .filter(|_| {
                    out.snapshot
                        .chunks
                        .iter()
                        .any(|chunk| chunk.pos == (ChunkPos { x: 0, z: 0 }))
                })
        })
        .await;
    assert_eq!(position, [4.5, 201.0, 2.5], "returning beekeeper position");
    assert!(runtime.core.submit(PlayerInput::UseItemOn {
        connection: ConnectionId(1),
        hand: Hand::Main,
        position: BlockPos { x: 4, y: 200, z: 4 },
        face: 1,
        cursor: [0.5, 1.0, 0.5],
        sequence: 1,
    }));
    let hive = registry
        .blocks
        .block(&Identifier::parse("minecraft:beehive").unwrap())
        .unwrap();
    runtime
        .until(|out, batches| {
            if batches
                .iter()
                .flat_map(|(_, changes)| changes)
                .any(|change| {
                    matches!(
                        change,
                        mc_core::boundary::PlayerChange::BlockActionsAcked { sequence: 1 }
                    )
                })
            {
                let chunk = out
                    .snapshot
                    .chunks
                    .iter()
                    .find(|c| c.pos == (ChunkPos { x: 0, z: 0 }))
                    .unwrap();
                assert!(
                    chunk
                        .chunk
                        .get_block(4, 201, 4)
                        .is_some_and(|s| hive.states.contains(&s)),
                    "placement acknowledged but no hive; floor={:?}, changes={batches:?}",
                    chunk.chunk.get_block(4, 200, 4)
                );
            }
            out.snapshot
                .chunks
                .iter()
                .find(|chunk| chunk.pos == (ChunkPos { x: 0, z: 0 }))
                .and_then(|chunk| chunk.chunk.get_block(4, 201, 4))
                .filter(|state| hive.states.contains(state))
                .map(|_| ())
        })
        .await;
    command(&runtime, "setblock 4 201 4 minecraft:air destroy").await;
    let bee_type = registry
        .entity_types
        .id_of(&Identifier::parse("minecraft:bee").unwrap())
        .unwrap();
    let released_uuid = runtime
        .until(|out, _| {
            out.snapshot
                .entities
                .iter()
                .find(|e| e.type_id == bee_type)
                .map(|e| e.uuid)
        })
        .await;
    assert!(runtime.core.checkpoint());
    drop(runtime);
    let recovered = bridge::recover_offline(&imported_config).unwrap();
    let released = recovered.dimensions[&Dimension::Overworld]
        .entities
        .values()
        .find(|e| e.uuid == released_uuid.to_string())
        .unwrap();
    assert!(
        released.state.controls.no_ai,
        "native bee controls lost on item release"
    );
    assert_eq!(
        released.health, 7.0,
        "native bee health lost on item release"
    );
}
