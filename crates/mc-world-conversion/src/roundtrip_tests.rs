use super::*;
use mc_core::dimensions::Dimension;
use mc_core::journal::CheckpointManifest;
use mc_core::native_checkpoint as bridge;
use mc_nbt::Tag;
use mc_world::native::{NativeCheckpoint, NativeStore};
use std::collections::BTreeMap;

/// Driven by tools/harness/vanilla_world_roundtrip.py: real Java creation, offline
/// native import/mutation/export, Java save, then fresh native import/re-export.
#[test]
#[ignore = "requires the real vanilla 26.1.2 headless round-trip driver"]
fn vanilla_headless_world_roundtrip() {
    use mc_core::boundary::{ConnectionId, PlayerChange, PlayerInput, TickOutput};
    use std::sync::Arc;

    let root = std::path::PathBuf::from(
        std::env::var_os("KRASNOKAMEN_VANILLA_ROUNDTRIP_WORLD").expect("round-trip world path"),
    );
    let phase = std::env::var("KRASNOKAMEN_VANILLA_ROUNDTRIP_PHASE").expect("round-trip phase");
    assert!(matches!(phase.as_str(), "create" | "verify"));
    let verified = phase == "verify";
    let anvil = std::path::PathBuf::from(
        std::env::var_os("KRASNOKAMEN_VANILLA_ROUNDTRIP_ANVIL").expect("Anvil conversion path"),
    );
    let receipt = std::path::PathBuf::from(
        std::env::var_os("KRASNOKAMEN_VANILLA_ROUNDTRIP_RECEIPT").expect("conversion receipt path"),
    );
    let export = std::path::PathBuf::from(
        std::env::var_os("KRASNOKAMEN_VANILLA_ROUNDTRIP_EXPORT").expect("Anvil export destination"),
    );
    let export_receipt = std::path::PathBuf::from(
        std::env::var_os("KRASNOKAMEN_VANILLA_ROUNDTRIP_EXPORT_RECEIPT")
            .expect("export receipt path"),
    );
    let mut config = crate::test_support::config(root.clone(), 42);
    config.view_distance = 1;
    config.simulation_distance = 0;
    config.checkpoint_every_ticks = u64::MAX;
    config.journal_fsync_every_batches = 1;
    assert!(
        !root.exists(),
        "each phase requires a fresh native destination"
    );
    crate::import_anvil(&config, &anvil, &root, &receipt).expect("offline Java Anvil import");
    let uuid = uuid::Uuid::parse_str("4d75256b-f3ff-3c0a-84fe-74c1a31dbd90").unwrap();
    let registry = Arc::new(crate::test_support::registry().unwrap());
    let core = mc_core::Core::open(config.clone()).expect("open native round-trip world");
    let mut receiver = core.subscribe();
    let advance = |ticks: usize, receiver: &mut tokio::sync::broadcast::Receiver<TickOutput>| {
        let mut last = None;
        for _ in 0..ticks {
            last = Some(
                receiver
                    .blocking_recv()
                    .expect("live round-trip tick thread"),
            );
            for (_, changes) in core.try_drain_outbox() {
                for change in changes {
                    if matches!(
                        change,
                        PlayerChange::Chat { .. } | PlayerChange::StyledChat { .. }
                    ) {
                        println!("{change:?}");
                    }
                }
            }
        }
        last.unwrap()
    };
    if phase == "create" {
        core.submit(PlayerInput::Join {
            connection: ConnectionId(261),
            uuid,
            name: "LayoutPlayer".into(),
            permission_level: 0,
        });
        advance(20, &mut receiver);
        let run = |command: &str, receiver: &mut tokio::sync::broadcast::Receiver<TickOutput>| {
            core.submit(PlayerInput::Command {
                connection: ConnectionId(261),
                command: command.into(),
                permission_level: 2,
            });
            advance(5, receiver);
        };
        for command in [
            "gamemode creative LayoutPlayer",
            "gamerule advance_time false",
            "gamerule advance_weather false",
            "gamerule spawn_mobs false",
            "time set 18000",
            "setworldspawn 8 80 8",
            "scoreboard objectives add layout_gate dummy",
            "scoreboard players set LayoutPlayer layout_gate 37",
            "give LayoutPlayer minecraft:diamond 3",
        ] {
            run(command, &mut receiver);
        }
        for dimension in Dimension::ALL {
            run(
                &format!("execute in {} run tp LayoutPlayer 8 82 8", dimension.name()),
                &mut receiver,
            );
            advance(20, &mut receiver);
            run("setblock 4 80 4 minecraft:diamond_block", &mut receiver);
            run("setblock 8 79 8 minecraft:stone", &mut receiver);
        }
        run(
            "execute in overworld run tp LayoutPlayer 8 80 8",
            &mut receiver,
        );
        advance(20, &mut receiver);
        run("spawnpoint LayoutPlayer 8 80 8", &mut receiver);
        run(
            "summon minecraft:cow 8 80 8 {CustomName:\"LayoutMarker\",NoAI:1b,Invulnerable:1b}",
            &mut receiver,
        );
        core.submit(PlayerInput::Leave {
            connection: ConnectionId(261),
        });
        advance(5, &mut receiver);
        assert!(core.checkpoint(), "v2 create checkpoint");
    } else {
        assert_eq!(phase, "verify");
        let output = advance(5, &mut receiver);
        assert_eq!(output.snapshot.day_time, 19000, "vanilla-changed day clock");
        assert!(
            output.snapshot.entities.iter().any(|entity| {
                entity
                    .appearance
                    .custom_name
                    .as_deref()
                    .is_some_and(|name| name.contains("LayoutMarkerVanilla"))
                    && (entity.position[0] - 8.0).abs() < 0.01
                    && (entity.position[2] - 9.0).abs() < 0.01
            }),
            "vanilla-saved named entity must be visible in v2"
        );
    }
    core.shutdown();
    drop(core);
    let marker = registry
        .blocks
        .block(
            &mc_data::Identifier::parse(if verified {
                "minecraft:emerald_block"
            } else {
                "minecraft:diamond_block"
            })
            .unwrap(),
        )
        .unwrap()
        .default;
    let store = crate::test_support::fixture_store(&config).unwrap();
    let recovered = mc_core::native_checkpoint::recover_with_reader(
        &config,
        Arc::clone(&registry),
        store.checkpoint_reader().unwrap(),
    )
    .unwrap();
    let mut imported_boundary_light = false;
    for dimension in Dimension::ALL {
        let mut world = mc_core::world::ChunkWorld::new_native(
            config.seed,
            Arc::clone(&store),
            mc_core::native_checkpoint::dimension_id(dimension),
            mc_core::native_checkpoint::generator(&config, &registry, dimension).unwrap(),
            &registry,
            mc_world::BlockStateId(registry.states.air),
            [8, 80, 8],
            dimension.geometry(),
        );
        let want = [(mc_world::ChunkPos { x: 0, z: 0 }, 0)]
            .into_iter()
            .collect();
        world.ensure_blocking(&want);
        assert_eq!(
            world.block_at(4, 80, 4),
            Some(marker.0),
            "{dimension:?} marker"
        );
        if verified {
            let chunk = world.chunk(mc_world::ChunkPos { x: 0, z: 0 }).unwrap();
            assert_eq!(chunk.geometry(), dimension.geometry());
            // Java adds boundary light to adjacent generated chunks, not
            // necessarily the marker chunk that v2 originally checkpointed.
            let want = (0..=2)
                .flat_map(|x| (0..=2).map(move |z| (mc_world::ChunkPos { x, z }, 0)))
                .collect();
            world.ensure_blocking(&want);
            for x in 0..=2 {
                for z in 0..=2 {
                    let pos = mc_world::ChunkPos { x, z };
                    let chunk = world.chunk(pos).unwrap_or_else(|| {
                        panic!(
                            "{dimension:?} chunk {pos:?} failed admission: {:?}",
                            world.load_errors().collect::<Vec<_>>()
                        )
                    });
                    assert_eq!(chunk.geometry(), dimension.geometry());
                    imported_boundary_light |= chunk
                        .boundary_section_lights
                        .iter()
                        .any(|light| light.block.is_some() || light.sky.is_some());
                }
            }
        }
    }
    assert!(
        !verified || imported_boundary_light,
        "must exercise real Java boundary-light import"
    );
    let manifest = &recovered.manifest;
    assert_eq!(
        manifest.world_spawn,
        Some(if verified { [9, 81, 9] } else { [8, 80, 8] })
    );
    assert_eq!(
        manifest.climate.as_ref().unwrap().day_time,
        if verified { 19000 } else { 18000 }
    );
    assert_eq!(manifest.gamerules["advance_time"], "false");
    let board = recovered.dimensions[&Dimension::Overworld]
        .scoreboard
        .as_ref()
        .unwrap();
    assert!(board.objectives.contains_key("layout_gate"));
    assert_eq!(
        board.scores["LayoutPlayer"]["layout_gate"].value,
        if verified { 41 } else { 37 }
    );
    let player = &recovered.dimensions[&Dimension::Overworld].players[&uuid.to_string()];
    assert_eq!(player.respawn, Some([8, 80, 8]));
    assert_eq!(player.respawn_dimension, Dimension::Overworld);
    let diamond = registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:diamond").unwrap())
        .unwrap();
    assert_eq!(
        player
            .inventory
            .iter()
            .filter(|stack| stack.item_id == diamond)
            .map(|stack| stack.count)
            .sum::<i32>(),
        3,
        "offline player item custody survives Java save and v2 reopen"
    );
    drop(recovered);
    drop(store);
    crate::export_anvil(&config, &root, &export, &export_receipt)
        .expect("offline native Anvil export");
    println!("KRASNOKAMEN_VANILLA_ROUNDTRIP_{}_OK", phase.to_uppercase());
}

fn save(root: &Path, path: &str, tag: &Tag) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    if path.exists() {
        std::fs::remove_file(&path).unwrap();
    }
    write_nbt(&path, tag).unwrap();
}

fn falcons_border(dimension: Dimension) -> mc_core::command_border::Border {
    let [center_x, center_z] = if dimension == Dimension::Nether {
        [31.0625, -23.0625]
    } else {
        [248.5, -184.5]
    };
    mc_core::command_border::Border {
        center_x,
        center_z,
        size: 128_000.0,
        // Import normalizes a settled border's target to its size.
        lerp_target: 128_000.0,
        ..Default::default()
    }
}

fn modern_fixture(root: &Path, config: &CoreConfig) {
    let registry = crate::test_support::registry().unwrap();
    let mut manifest = CheckpointManifest {
        tick: 79,
        domain_versions: bridge::domain_versions(),
        world_spawn: Some([12, 75, -45]),
        world_spawn_rotation: [90.0, 15.0],
        difficulty: mc_core::survival::Difficulty::Hard,
        climate: Some(mc_core::weather::ClimateImage {
            day_time: 18000,
            game_clock: Default::default(),
            weather_kind: 2,
            remaining_ticks: 1234,
            thunder_remaining_ticks: 4321,
            clear_remaining_ticks: 0,
            thundering: true,
            rain_level_bits: 1.0_f32.to_bits(),
            thunder_level_bits: 1.0_f32.to_bits(),
            advance_time: false,
            advance_weather: true,
        }),
        ..Default::default()
    };
    manifest
        .gamerules
        .insert("advance_time".into(), "false".into());
    manifest
        .gamerules
        .insert("random_tick_speed".into(), "7".into());
    manifest.border = falcons_border(Dimension::Overworld);
    for dimension in [Dimension::Nether, Dimension::End] {
        manifest.dimensions.insert(
            dimension,
            Box::new(CheckpointManifest {
                tick: manifest.tick,
                domain_versions: bridge::domain_versions(),
                border: falcons_border(dimension),
                ..Default::default()
            }),
        );
    }
    let mut metadata = mc_core::level_metadata::MetadataTags::new();
    mc_core::level_metadata::checkpoint_tags(&mut metadata, config.seed, &manifest).unwrap();
    if let Tag::Compound(root) = metadata.get_mut("level.dat").unwrap() {
        root.push(("OpaqueRoot".into(), Tag::Long(37)));
    }
    for (path, tag) in metadata {
        save(root, &path, &tag);
    }
    let mut scoreboard = mc_core::scoreboard::Scoreboard::default();
    scoreboard.objectives.insert(
        "gate".into(),
        mc_core::scoreboard::Objective {
            criteria: "dummy".into(),
            display_name: "gate".into(),
        },
    );
    scoreboard.scores.insert(
        "FixturePlayer".into(),
        BTreeMap::from([(
            "gate".into(),
            mc_core::scoreboard::Score {
                value: 37,
                locked: true,
            },
        )]),
    );
    save(root, "data/minecraft/scoreboard.dat", &scoreboard.to_nbt());
    for (index, dimension) in [Dimension::Overworld, Dimension::Nether, Dimension::End]
        .into_iter()
        .enumerate()
    {
        let directory = dimension.directory(root);
        std::fs::create_dir_all(directory.join("region")).unwrap();
        let mut chunk = mc_world::Chunk::empty_with_geometry(
            mc_world::ChunkPos { x: -1, z: 1 },
            mc_world::BlockStateId(registry.states.air),
            mc_data::Identifier::parse("minecraft:plains").unwrap(),
            dimension.geometry(),
        );
        chunk.status = "minecraft:full".into();
        chunk
            .extras
            .push(("InhabitedTime".into(), Tag::Long(1234 + index as i64)));
        chunk.extras.push((
            "OpaqueTerrain".into(),
            Tag::String(format!("dimension-{index}")),
        ));
        // Java preserves unknown transport metadata from older Krasnokamen exports.
        chunk
            .extras
            .push(("KrasnokamenJournalLsn".into(), Tag::Long(73)));
        chunk
            .scheduled_block_ticks
            .push(mc_world::ScheduledBlockTick::new(
                mc_world::BlockPos {
                    x: -8,
                    y: 64,
                    z: 24,
                },
                mc_data::Identifier::parse("minecraft:stone").unwrap(),
                87,
                0,
            ));
        chunk
            .scheduled_fluid_ticks
            .push(mc_world::ScheduledFluidTick::new(
                mc_world::BlockPos {
                    x: -7,
                    y: 64,
                    z: 24,
                },
                mc_data::Identifier::parse("minecraft:flowing_water").unwrap(),
                91,
                0,
            ));
        let payload = mc_world::anvil::chunk_to_payload_with_items_at_tick(
            &chunk,
            &registry.blocks,
            Some(&registry.items),
            0,
            79,
        )
        .unwrap();
        mc_world::anvil::write_region(directory.join("region/r.-1.0.mca"), &[payload]).unwrap();
        let entity = mc_data::entity_nbt::VanillaEntity {
            id: "minecraft:cow".into(),
            uuid: uuid::Uuid::from_u128(100 + index as u128),
            pos: [-8.0, 64.0, 24.0],
            wire_id: if index == 0 { 50 } else { 0 },
            pickup_delay: 23,
            extra_fields: vec![(
                "OpaqueEntity".into(),
                Tag::Compound(vec![("nested".into(), Tag::Long(987654321))]),
            )],
            ..Default::default()
        };
        mc_world::anvil::write_entity_regions(
            &directory.join("entities"),
            &registry.items,
            &BTreeMap::from([(
                (-1, 1),
                mc_world::anvil::EntityChunk {
                    x: -1,
                    z: 1,
                    entities: vec![entity],
                },
            )]),
            0,
        )
        .unwrap();
        save(
            root,
            &format!(
                "dimensions/minecraft/{}/data/minecraft/raids.dat",
                dimension.name().trim_start_matches("minecraft:")
            ),
            &mc_core::raids::to_nbt(&mc_core::raids::Raids::default()),
        );
        save(
            root,
            &format!(
                "dimensions/minecraft/{}/data/minecraft/chunk_tickets.dat",
                dimension.name().trim_start_matches("minecraft:")
            ),
            &Tag::Compound(vec![
                (
                    "DataVersion".into(),
                    Tag::Int(mc_world::anvil::TARGET_DATA_VERSION),
                ),
                (
                    "data".into(),
                    Tag::Compound(vec![(
                        "tickets".into(),
                        Tag::List(mc_nbt::ListTag {
                            element_type: mc_nbt::tag_type::COMPOUND,
                            elements: vec![Tag::Compound(vec![
                                ("chunk_pos".into(), Tag::IntArray(vec![0, 0])),
                                ("type".into(), Tag::String("minecraft:forced".into())),
                                ("level".into(), Tag::Int(31)),
                            ])],
                        }),
                    )]),
                ),
            ]),
        );
    }
    save(
        root,
        "dimensions/minecraft/the_end/data/minecraft/ender_dragon_fight.dat",
        &mc_core::end_fight::to_nbt(&mc_core::end_fight::Fight::default()),
    );
    let mut player = mc_data::player_nbt::player_to_fields(
        &registry.items,
        &mc_data::player_nbt::VanillaPlayer::default(),
    )
    .unwrap();
    player.push((
        "DataVersion".into(),
        Tag::Int(mc_world::anvil::TARGET_DATA_VERSION),
    ));
    player.push((
        "Dimension".into(),
        Tag::String("minecraft:the_nether".into()),
    ));
    player.push(("OpaquePlayer".into(), Tag::String("retained".into())));
    save(
        root,
        "players/data/00000000-0000-0000-0000-000000000003.dat",
        &Tag::Compound(player),
    );
    std::fs::create_dir_all(root.join("players/stats")).unwrap();
    std::fs::write(
        root.join("players/stats/00000000-0000-0000-0000-000000000003.json"),
        b"{\"opaque\":37}",
    )
    .unwrap();
}

#[test]
fn nested_paths_import_export_and_census_under_world_leases() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("anvil");
    let native = root.path().join("native");
    let output = root.path().join("export");
    let config = crate::test_support::config(native.clone(), 42);
    modern_fixture(&source, &config);
    fs::write(
        source.join(mc_world::storage::WORLD_LEASE_FILE_NAME),
        b"coordination only",
    )
    .unwrap();
    let before = hashes(&source).unwrap();
    let nested = Path::new("dimensions")
        .join("minecraft")
        .join("the_end")
        .join("data")
        .join("minecraft")
        .join("raids.dat");
    assert!(source.join(&nested).is_file());
    assert!(before.contains_key("dimensions/minecraft/the_end/data/minecraft/raids.dat"));
    assert!(before.keys().all(|path| !path.contains('\\')));
    assert!(!before.contains_key(mc_world::storage::WORLD_LEASE_FILE_NAME));
    assert_eq!(
        import_anvil(&config, &source, &native, &root.path().join("import.json"))
            .unwrap()
            .status,
        "PASS"
    );
    let native_before = hashes(&native).unwrap();
    let census = census_native(&config, &native, &root.path().join("census.json")).unwrap();
    assert_eq!(census.status, "PASS");
    assert!(census.read_only);
    assert_eq!(
        export_anvil(&config, &native, &output, &root.path().join("export.json"))
            .unwrap()
            .status,
        "PASS"
    );
    assert_eq!(
        read_nbt(&source.join(&nested)).unwrap(),
        read_nbt(&output.join(&nested)).unwrap()
    );
    assert_eq!(hashes(&source).unwrap(), before);
    assert_eq!(hashes(&native).unwrap(), native_before);
}

fn has_field(tag: &Tag, key: &str) -> bool {
    matches!(tag, Tag::Compound(fields) if fields.iter().any(|(name, _)| name == key))
}

#[tokio::test]
async fn imported_mount_cargo_menu_roundtrip_uses_native_contents() {
    use crate::test_support::Running;
    use mc_core::boundary::{ClickMode, ConnectionId, EntityId, Hand, PlayerChange, PlayerInput};
    use mc_data::{Identifier, ItemStack};
    use mc_nbt::{ListTag, tag_type};

    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("anvil");
    let native = temp.path().join("native");
    let output = temp.path().join("export");
    let again = temp.path().join("again");
    for path in [&source, &native, &output, &again] {
        std::fs::create_dir(path).unwrap();
    }
    let config = crate::test_support::config(native.clone(), 42);
    modern_fixture(&source, &config);
    let registry = crate::test_support::registry().unwrap();
    let diamond = registry
        .items
        .id_of(&Identifier::parse("minecraft:diamond").unwrap())
        .unwrap();
    let emerald = registry
        .items
        .id_of(&Identifier::parse("minecraft:emerald").unwrap())
        .unwrap();
    let mut fields =
        mc_data::item_nbt::stack_to_fields(&registry.items, &ItemStack::new(diamond, 7)).unwrap();
    fields.push(("Slot".into(), Tag::Byte(0)));
    let uuid = uuid::Uuid::from_u128(0x123456);
    let mount = mc_data::entity_nbt::VanillaEntity {
        id: "minecraft:donkey".into(),
        uuid,
        wire_id: 200,
        pos: [-8.0, 64.0, 24.0],
        health: 20.0,
        extra_fields: vec![
            ("Tame".into(), Tag::Byte(1)),
            ("ChestedHorse".into(), Tag::Byte(1)),
            (
                "Items".into(),
                Tag::List(ListTag {
                    element_type: tag_type::COMPOUND,
                    elements: vec![Tag::Compound(fields)],
                }),
            ),
        ],
        ..Default::default()
    };
    let entity_dir = Dimension::Overworld.directory(&source).join("entities");
    let mut chunks = mc_world::anvil::read_entity_regions(&entity_dir, &registry.items).unwrap();
    chunks.get_mut(&(-1, 1)).unwrap().entities.push(mount);
    mc_world::anvil::write_entity_regions(&entity_dir, &registry.items, &chunks, 79).unwrap();
    let mut floor = mc_world::Chunk::empty(
        mc_world::ChunkPos { x: -1, z: 1 },
        mc_world::BlockStateId(registry.states.air),
        Identifier::parse("minecraft:plains").unwrap(),
    );
    floor.status = "minecraft:full".into();
    for x in 0..16 {
        for z in 0..16 {
            floor
                .set_block(x, 63, z, mc_world::BlockStateId(registry.states.stone))
                .unwrap();
        }
    }
    let payload = mc_world::anvil::chunk_to_payload_with_items_at_tick(
        &floor,
        &registry.blocks,
        Some(&registry.items),
        0,
        79,
    )
    .unwrap();
    mc_world::anvil::write_region(
        Dimension::Overworld
            .directory(&source)
            .join("region/r.-1.0.mca"),
        &[payload],
    )
    .unwrap();
    let player_uuid = uuid::Uuid::from_u128(0x123457);
    let mut player = mc_data::player_nbt::player_to_fields(
        &registry.items,
        &mc_data::player_nbt::VanillaPlayer {
            pos: [-8.5, 64.0, 24.5],
            inventory: vec![(0, ItemStack::new(emerald, 5))],
            ..Default::default()
        },
    )
    .unwrap();
    player.push((
        "DataVersion".into(),
        Tag::Int(mc_world::anvil::TARGET_DATA_VERSION),
    ));
    player.push((
        "Dimension".into(),
        Tag::String("minecraft:overworld".into()),
    ));
    save(
        &source,
        &format!("players/data/{player_uuid}.dat"),
        &Tag::Compound(player),
    );
    import::convert(&config, &source, &native).unwrap();
    let recovered = bridge::recover_offline(&config).unwrap();
    let target = EntityId(
        *recovered.dimensions[&Dimension::Overworld]
            .entities
            .iter()
            .find(|(_, entity)| entity.uuid == uuid.to_string())
            .unwrap()
            .0,
    );
    drop(recovered);
    let mut live = Running::open(config.clone());
    assert!(live.core.submit(PlayerInput::Join {
        connection: ConnectionId(1),
        uuid: player_uuid,
        name: "rider".into(),
        permission_level: 0,
    }));
    live.until(|_, batches| {
        batches
            .iter()
            .filter(|(connection, _)| *connection == ConnectionId(1))
            .flat_map(|(_, changes)| changes)
            .any(|change| matches!(change, PlayerChange::Joined { .. }))
            .then_some(())
    })
    .await;
    assert!(live.core.submit(PlayerInput::InteractEntitySecondary {
        connection: ConnectionId(1),
        target,
        hand: Hand::Main,
        location: [0.0; 3],
    }));

    async fn contents(
        live: &mut Running,
        cargo: Option<ItemStack>,
    ) -> (u8, i32, Vec<Option<ItemStack>>) {
        live.until(|_, batches| {
            batches
                .iter()
                .rev()
                .filter(|(connection, _)| *connection == ConnectionId(1))
                .flat_map(|(_, changes)| changes.iter().rev())
                .find_map(|change| match change {
                    PlayerChange::MenuContents {
                        window,
                        state_id,
                        slots,
                        ..
                    } if *window != 0 && slots.get(2) == Some(&cargo) => {
                        Some((*window, *state_id, slots.clone()))
                    }
                    _ => None,
                })
        })
        .await
    }

    let (window, state_id, _) = contents(&mut live, Some(ItemStack::new(diamond, 7))).await;
    assert!(live.core.submit(PlayerInput::ContainerClick {
        connection: ConnectionId(1),
        window,
        state_id,
        slot: 2,
        button: 0,
        mode: ClickMode::QuickMove,
    }));
    let (window, state_id, slots) = contents(&mut live, None).await;
    assert_eq!(
        slots[17..]
            .iter()
            .flatten()
            .filter(|stack| stack.item_id == diamond)
            .map(|stack| stack.count)
            .sum::<i32>(),
        7
    );
    assert_eq!(slots[44], Some(ItemStack::new(emerald, 5)));
    assert!(live.core.submit(PlayerInput::ContainerClick {
        connection: ConnectionId(1),
        window,
        state_id,
        slot: 44,
        button: 0,
        mode: ClickMode::QuickMove,
    }));
    let (_, _, slots) = contents(&mut live, Some(ItemStack::new(emerald, 5))).await;
    let expected: Vec<_> = slots[2..17]
        .iter()
        .map(|stack| stack.clone().unwrap_or(ItemStack::EMPTY))
        .collect();
    assert_eq!(expected[0], ItemStack::new(emerald, 5));
    assert!(live.core.checkpoint());
    drop(live);
    export::convert(&config, &native, &output).unwrap();
    let again_config = crate::test_support::config(again.clone(), 42);
    import::convert(&again_config, &output, &again).unwrap();
    let recovered = bridge::recover_offline(&again_config).unwrap();
    let record = recovered.dimensions[&Dimension::Overworld]
        .entities
        .values()
        .find(|entity| entity.uuid == uuid.to_string())
        .unwrap();
    assert_eq!(record.state.entity_life.mount.cargo, expected);
}

#[test]
fn modern_all_domains_three_dimension_borders_roundtrip_and_unknown_entity_checkpoint_survival() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("anvil");
    let native = temp.path().join("native");
    let output = temp.path().join("export");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&native).unwrap();
    std::fs::create_dir(&output).unwrap();
    let config = crate::test_support::config(native.clone(), 42);
    modern_fixture(&source, &config);
    let original: Vec<_> = files(&source)
        .unwrap()
        .into_iter()
        .map(|path| {
            let bytes = std::fs::read(source.join(&path)).unwrap();
            (path, bytes)
        })
        .collect();
    let stats = import::convert(&config, &source, &native).unwrap();
    assert_eq!(
        (
            stats.tick,
            stats.chunks,
            stats.entities,
            stats.players,
            stats.reminted_ids
        ),
        (79, 3, 3, 1, 2)
    );
    for (path, bytes) in &original {
        assert_eq!(std::fs::read(source.join(path)).unwrap(), *bytes);
    }
    let store = NativeStore::open(
        &native.join("native/state.redb"),
        bridge::native_identity(&config),
    )
    .unwrap();
    let checkpoint = store.read_checkpoint().unwrap();
    assert_eq!(
        checkpoint
            .globals
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        [
            "bossbars",
            "checkpoint",
            "conversion_extras",
            "dimensions",
            "generation",
            "level_metadata",
            "map_sequence",
            "plugin_startup",
            "plugins",
            "scoreboard",
        ]
    );
    let manifest: CheckpointManifest =
        serde_json::from_slice(&checkpoint.globals["checkpoint"].payload).unwrap();
    assert_eq!(manifest.domain_versions, bridge::domain_versions());
    assert_eq!(manifest.dimensions.len(), 2);
    assert_eq!(manifest.world_spawn, Some([12, 75, -45]));
    assert_eq!(manifest.world_spawn_rotation, [90.0, 15.0]);
    assert_eq!(manifest.gamerules["random_tick_speed"], "7");
    assert_eq!(manifest.climate.unwrap().day_time, 18000);
    for (dimension, child) in std::iter::once((Dimension::Overworld, &manifest)).chain(
        manifest
            .dimensions
            .iter()
            .map(|(&dimension, child)| (dimension, &**child)),
    ) {
        assert_eq!(child.border, falcons_border(dimension));
        assert_eq!(child.tick, 79);
        assert_eq!(child.block_ticks.chunks[0].ticks[0].trigger_tick, 87);
        assert_eq!(child.fluid_ticks.due[0].deadline, 91);
        assert!(child.raids.is_some());
        assert_eq!(child.tickets["minecraft:forced:0:0"], ([0, 0], (79, 0)));
    }
    assert!(manifest.dimensions[&Dimension::End].end_fight.is_some());
    let ids: std::collections::BTreeSet<_> = checkpoint
        .entities
        .iter()
        .map(|entity| {
            let Tag::Compound(fields) = entity.payload.as_ref().unwrap() else {
                panic!("entity compound")
            };
            fields
                .iter()
                .find(|(name, _)| name == "krasnokamen:WireId")
                .unwrap()
                .1
                .clone()
        })
        .map(|tag| {
            let Tag::Int(id) = tag else { panic!("wire id") };
            id
        })
        .collect();
    assert_eq!(ids, [50, 51, 52].into_iter().collect());
    assert!(
        checkpoint
            .entities
            .iter()
            .all(|entity| has_field(entity.payload.as_ref().unwrap(), "OpaqueEntity"))
    );
    drop(store);
    // Use the runtime's pure hydration/capture bridge, not a converter-only echo.
    let mut recovered = bridge::recover_offline(&config).unwrap();
    for (&dimension, replay) in &mut recovered.dimensions {
        assert_eq!(replay.border, falcons_border(dimension));
        replay.tick = 80;
    }
    let captured = bridge::encode_checkpoint(&recovered).unwrap();
    assert!(
        captured
            .entities
            .iter()
            .all(|entity| has_field(entity.payload.as_ref().unwrap(), "OpaqueEntity"))
    );
    drop(recovered);
    let store = NativeStore::open(
        &native.join("native/state.redb"),
        bridge::native_identity(&config),
    )
    .unwrap();
    let registry = crate::test_support::registry().unwrap();
    store
        .commit_checkpoint(
            NativeCheckpoint {
                tick: 80,
                previous_tick: 79,
                chunks: &[],
                entities: &captured.entities,
                players: &captured.players,
                globals: &captured.globals,
            },
            &registry.blocks,
            &registry.items,
        )
        .unwrap();
    drop(store);
    export::convert(&config, &native, &output).unwrap();
    assert!(has_field(
        &read_nbt(&output.join("level.dat")).unwrap(),
        "OpaqueRoot"
    ));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(
            &std::fs::read(output.join("players/stats/00000000-0000-0000-0000-000000000003.json"))
                .unwrap(),
        )
        .unwrap(),
        serde_json::json!({"opaque":37})
    );
    for dimension in [Dimension::Overworld, Dimension::Nether, Dimension::End] {
        mc_world::anvil::visit_region(
            dimension.directory(&output).join("region/r.-1.0.mca"),
            |payload| {
                let (_, root) =
                    mc_nbt::read_named(&mut std::io::Cursor::new(&payload.uncompressed_nbt))
                        .unwrap();
                assert!(!has_field(&root, "KrasnokamenJournalLsn"));
            },
        )
        .unwrap();
        let chunks = mc_world::anvil::read_entity_regions(
            &dimension.directory(&output).join("entities"),
            &registry.items,
        )
        .unwrap();
        assert_eq!(chunks[&(-1, 1)].entities.len(), 1);
        assert!(
            chunks[&(-1, 1)].entities[0]
                .extra_fields
                .iter()
                .any(|(name, _)| name == "OpaqueEntity")
        );
    }
    let second = temp.path().join("native-again");
    std::fs::create_dir(&second).unwrap();
    let second_config = crate::test_support::config(second.clone(), 42);
    let again = import::convert(&second_config, &output, &second).unwrap();
    assert_eq!(
        (
            again.tick,
            again.chunks,
            again.entities,
            again.players,
            again.reminted_ids
        ),
        (80, 3, 3, 1, 0)
    );
    let reopened = bridge::recover_offline(&second_config).unwrap();
    for (&dimension, replay) in &reopened.dimensions {
        assert_eq!(replay.border, falcons_border(dimension));
    }
}

#[test]
fn nonzero_import_ids_are_deterministic_and_runtime_admits_empty_wal() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("anvil");
    std::fs::create_dir(&source).unwrap();
    let first_config = crate::test_support::config(temp.path().join("first"), 42);
    modern_fixture(&source, &first_config);
    let mut identities = Vec::new();
    for root in [first_config.world_dir.clone(), temp.path().join("second")] {
        std::fs::create_dir(&root).unwrap();
        let config = crate::test_support::config(root.clone(), 42);
        let stats = import::convert(&config, &source, &root).unwrap();
        assert_eq!(stats.tick, 79);
        assert!(!root.join("journal").exists());
        let store = NativeStore::open(
            &root.join("native/state.redb"),
            bridge::native_identity(&config),
        )
        .unwrap();
        let checkpoint = store.read_checkpoint().unwrap();
        identities.push(
            checkpoint
                .entities
                .iter()
                .map(|entity| (entity.identity, entity.payload.clone()))
                .collect::<Vec<_>>(),
        );
        drop(store);
        let core = mc_core::Core::open(config).unwrap();
        core.shutdown();
    }
    assert_eq!(identities[0], identities[1]);
}

#[test]
fn failed_import_partial_stage_and_unpublished_wal_are_never_runtime_worlds() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("anvil");
    let stage = temp.path().join("interrupted-stage");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&stage).unwrap();
    let config = crate::test_support::config(stage.clone(), 42);
    modern_fixture(&source, &config);
    // Terrain is staged before players. A malformed player must abort the bulk
    // import without a checkpoint that runtime could admit as a published world.
    save(
        &source,
        "players/data/00000000-0000-0000-0000-000000000003.dat",
        &Tag::Compound(vec![
            (
                "DataVersion".into(),
                Tag::Int(mc_world::anvil::TARGET_DATA_VERSION),
            ),
            ("UUID".into(), Tag::IntArray(vec![0; 3])),
        ]),
    );
    let error = import::convert(&config, &source, &stage)
        .expect_err("malformed player UUID must abort import")
        .to_string();
    assert!(error.contains("expected four-int player UUID"), "{error}");
    assert!(stage.join("native/state.redb").is_file());
    assert!(mc_core::Core::open(config.clone()).is_err());
    std::fs::create_dir_all(stage.join("journal")).unwrap();
    std::fs::write(
        stage.join("journal/batch-00000000000000000080-00000000000000000080.bin"),
        b"unpublished",
    )
    .unwrap();
    assert!(mc_core::Core::open(config).is_err());

    let destination = temp.path().join("published");
    let receipt = temp.path().join("receipt.json");
    assert!(
        import_anvil(
            &CoreConfig::new(destination.clone(), 42),
            &source,
            &destination,
            &receipt
        )
        .is_err()
    );
    assert!(!destination.exists());
    let evidence: ConversionReceipt =
        serde_json::from_slice(&std::fs::read(receipt).unwrap()).unwrap();
    assert_ne!(evidence.status, "PASS");
}

#[test]
fn unsupported_java_ticket_semantics_fail_import_before_publication() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("anvil");
    let stage = temp.path().join("stage");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&stage).unwrap();
    let config = crate::test_support::config(stage.clone(), 42);
    modern_fixture(&source, &config);
    let path = "dimensions/minecraft/overworld/data/minecraft/chunk_tickets.dat";
    let mut tag = read_nbt(&source.join(path)).unwrap();
    let Tag::Compound(root) = &mut tag else {
        panic!("saved data root")
    };
    let Tag::Compound(data) = &mut root.iter_mut().find(|(name, _)| name == "data").unwrap().1
    else {
        panic!("saved data")
    };
    let Tag::List(tickets) = &mut data
        .iter_mut()
        .find(|(name, _)| name == "tickets")
        .unwrap()
        .1
    else {
        panic!("ticket list")
    };
    let Tag::Compound(ticket) = &mut tickets.elements[0] else {
        panic!("ticket compound")
    };
    ticket
        .iter_mut()
        .find(|(name, _)| name == "level")
        .unwrap()
        .1 = Tag::Int(32);
    save(&source, path, &tag);
    assert!(import::convert(&config, &source, &stage).is_err());
    assert!(!stage.join("native/state.redb").exists());
}

#[test]
fn opaque_extras_aggregate_budget_refuses_before_creating_native_database() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("anvil");
    let stage = temp.path().join("stage");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&stage).unwrap();
    let config = crate::test_support::config(stage.clone(), 42);
    modern_fixture(&source, &config);
    // Sparse opaque backup: admission must refuse from file metadata without
    // reading or allocating the oversized body and without importing terrain.
    let backup = std::fs::File::create(source.join("level.dat_old")).unwrap();
    backup.set_len(16 * 1024 * 1024).unwrap();
    drop(backup);
    let error = import::convert(&config, &source, &stage)
        .unwrap_err()
        .to_string();
    assert!(error.contains("conversion_extras"), "{error}");
    assert!(error.contains("16 MiB"), "{error}");
    assert!(error.contains("level.dat_old"), "{error}");
    assert!(!stage.join("native").exists());
}

#[test]
fn unmodeled_saved_data_is_retained_verbatim_and_unreadable_nbt_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    let native = dir.path().join("native");
    let exported = dir.path().join("exported");
    let config = crate::test_support::config(native.clone(), 42);
    modern_fixture(&source, &config);
    let domains = [
        "custom_boss_events",
        "random_sequences",
        "scheduled_events",
        "stopwatches",
    ];
    let mut originals = BTreeMap::new();
    for domain in domains {
        let path = format!("data/minecraft/{domain}.dat");
        let tag = Tag::Compound(vec![
            ("DataVersion".into(), Tag::Int(4790)),
            (
                "data".into(),
                Tag::Compound(vec![
                    ("UnmodeledNonemptyState".into(), Tag::String(domain.into())),
                    (
                        "Nested".into(),
                        Tag::Compound(vec![("opaque".into(), Tag::Long(91))]),
                    ),
                ]),
            ),
        ]);
        save(&source, &path, &tag);
        originals.insert(path.clone(), std::fs::read(source.join(path)).unwrap());
    }
    let imported =
        import_anvil(&config, &source, &native, &dir.path().join("import.json")).unwrap();
    let exported_receipt =
        export_anvil(&config, &native, &exported, &dir.path().join("export.json")).unwrap();
    for (path, bytes) in originals {
        assert!(imported.opaque_retention.contains(&path));
        assert!(exported_receipt.opaque_retention.contains(&path));
        assert_eq!(std::fs::read(exported.join(path)).unwrap(), bytes);
    }
    std::fs::write(
        source.join("data/minecraft/scheduled_events.dat"),
        b"unreadable NBT",
    )
    .unwrap();
    let destination = dir.path().join("invalid-native");
    let receipt = dir.path().join("invalid.json");
    let before = hashes(&source).unwrap();
    assert!(import_anvil(&config, &source, &destination, &receipt).is_err());
    assert!(!destination.exists());
    assert_eq!(before, hashes(&source).unwrap());
    let receipt: ConversionReceipt =
        serde_json::from_slice(&std::fs::read(receipt).unwrap()).unwrap();
    assert_eq!(receipt.status, "FAIL");
    assert!(
        receipt
            .error
            .unwrap()
            .contains("data/minecraft/scheduled_events.dat")
    );
}

#[test]
fn overdue_tick_deadlines_and_reversed_priorities_survive_native_conversion() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    let native = dir.path().join("native");
    let exported = dir.path().join("exported");
    let config = crate::test_support::config(native.clone(), 42);
    modern_fixture(&source, &config);
    let registry = crate::test_support::registry().unwrap();
    let mut chunk = mc_world::Chunk::empty_with_geometry(
        mc_world::ChunkPos { x: -1, z: 1 },
        mc_world::BlockStateId(registry.states.air),
        mc_data::Identifier::parse("minecraft:plains").unwrap(),
        Dimension::Overworld.geometry(),
    );
    chunk.status = "minecraft:full".into();
    for (x, deadline, priority) in [(-8, 70, 1), (-7, 75, -1)] {
        let position = mc_world::BlockPos { x, y: 64, z: 24 };
        chunk
            .scheduled_block_ticks
            .push(mc_world::ScheduledBlockTick::new(
                position,
                mc_data::Identifier::parse("minecraft:stone").unwrap(),
                deadline,
                priority,
            ));
        chunk
            .scheduled_fluid_ticks
            .push(mc_world::ScheduledFluidTick::new(
                position,
                mc_data::Identifier::parse("minecraft:water").unwrap(),
                deadline,
                priority,
            ));
    }
    let payload = mc_world::anvil::chunk_to_payload_with_items_at_tick(
        &chunk,
        &registry.blocks,
        Some(&registry.items),
        0,
        79,
    )
    .unwrap();
    mc_world::anvil::write_region(
        Dimension::Overworld
            .directory(&source)
            .join("region/r.-1.0.mca"),
        &[payload],
    )
    .unwrap();
    let before = hashes(&source).unwrap();
    import_anvil(&config, &source, &native, &dir.path().join("import.json")).unwrap();
    let recovered = bridge::recover_offline(&config).unwrap();
    let manifest = &recovered.manifest;
    assert_eq!(
        manifest.block_ticks.chunks[0]
            .ticks
            .iter()
            .map(|tick| (tick.trigger_tick, tick.priority))
            .collect::<Vec<_>>(),
        vec![(70, 1), (75, -1)]
    );
    assert_eq!(
        manifest
            .fluid_ticks
            .due
            .iter()
            .map(|tick| (tick.deadline, tick.priority))
            .collect::<Vec<_>>(),
        vec![(70, 1), (75, -1)]
    );
    drop(recovered);
    export_anvil(&config, &native, &exported, &dir.path().join("export.json")).unwrap();
    let mut visited = 0;
    mc_world::anvil::visit_region(
        Dimension::Overworld
            .directory(&exported)
            .join("region/r.-1.0.mca"),
        |payload| {
            visited += 1;
            let (_, root) =
                mc_nbt::read_named(&mut std::io::Cursor::new(&payload.uncompressed_nbt)).unwrap();
            let Tag::Compound(fields) = root else {
                panic!("chunk compound required");
            };
            for key in ["block_ticks", "fluid_ticks"] {
                let Tag::List(ticks) = &fields.iter().find(|(name, _)| name == key).unwrap().1
                else {
                    panic!("tick list required");
                };
                let actual: Vec<_> = ticks
                    .elements
                    .iter()
                    .map(|tick| {
                        let Tag::Compound(fields) = tick else {
                            panic!("tick compound required");
                        };
                        let int =
                            |key| match &fields.iter().find(|(name, _)| name == key).unwrap().1 {
                                Tag::Int(value) => *value,
                                _ => panic!("tick int required"),
                            };
                        (int("t"), int("p"))
                    })
                    .collect();
                assert_eq!(actual, vec![(-9, 1), (-4, -1)], "{key}");
            }
        },
    )
    .unwrap();
    assert_eq!(visited, 1);
    assert_eq!(before, hashes(&source).unwrap());
}

#[test]
fn flowing_fluid_ids_survive_conversion_and_export_uses_authoritative_queue() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    let native = dir.path().join("native");
    let exported = dir.path().join("exported");
    let config = crate::test_support::config(native.clone(), 42);
    modern_fixture(&source, &config);
    let registry = crate::test_support::registry().unwrap();
    let mut chunk = mc_world::Chunk::empty_with_geometry(
        mc_world::ChunkPos { x: -1, z: 1 },
        mc_world::BlockStateId(registry.states.air),
        mc_data::Identifier::parse("minecraft:plains").unwrap(),
        Dimension::Overworld.geometry(),
    );
    chunk.status = "minecraft:full".into();
    for (x, name, block, deadline, priority) in [
        (
            -7_i32,
            "minecraft:flowing_water",
            registry.states.water_levels[1],
            91,
            -1,
        ),
        (
            -6,
            "minecraft:flowing_lava",
            registry.states.lava_levels[8],
            93,
            1,
        ),
    ] {
        chunk.set_block(x.rem_euclid(16) as u8, 64, 8, mc_world::BlockStateId(block));
        chunk
            .scheduled_fluid_ticks
            .push(mc_world::ScheduledFluidTick::new(
                mc_world::BlockPos { x, y: 64, z: 24 },
                mc_data::Identifier::parse(name).unwrap(),
                deadline,
                priority,
            ));
    }
    let payload = mc_world::anvil::chunk_to_payload_with_items_at_tick(
        &chunk,
        &registry.blocks,
        Some(&registry.items),
        0,
        79,
    )
    .unwrap();
    mc_world::anvil::write_region(
        Dimension::Overworld
            .directory(&source)
            .join("region/r.-1.0.mca"),
        &[payload],
    )
    .unwrap();
    let read_ids = |path: &Path| {
        let mut ids = Vec::new();
        mc_world::anvil::visit_region(path, |payload| {
            let (_, root) =
                mc_nbt::read_named(&mut std::io::Cursor::new(&payload.uncompressed_nbt)).unwrap();
            let Tag::Compound(fields) = root else {
                panic!("chunk compound required");
            };
            let Tag::List(ticks) = &fields
                .iter()
                .find(|(name, _)| name == "fluid_ticks")
                .unwrap()
                .1
            else {
                panic!("fluid tick list required");
            };
            for tick in &ticks.elements {
                let Tag::Compound(fields) = tick else {
                    panic!("fluid tick compound required");
                };
                let Tag::String(id) = &fields.iter().find(|(name, _)| name == "i").unwrap().1
                else {
                    panic!("fluid tick identifier required");
                };
                ids.push(id.clone());
            }
        })
        .unwrap();
        ids
    };
    let source_region = Dimension::Overworld
        .directory(&source)
        .join("region/r.-1.0.mca");
    let source_ids = read_ids(&source_region);
    assert_eq!(
        source_ids,
        ["minecraft:flowing_water", "minecraft:flowing_lava"]
    );
    let before = hashes(&source).unwrap();
    import_anvil(&config, &source, &native, &dir.path().join("import.json")).unwrap();
    let store = crate::test_support::fixture_store(&config).unwrap();
    let mut manifest = crate::test_support::read_manifest(&store).unwrap();
    assert_eq!(
        manifest
            .fluid_ticks
            .due
            .iter()
            .map(|tick| tick.fluid.as_str())
            .collect::<Vec<_>>(),
        source_ids.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    drop(store);
    export_anvil(&config, &native, &exported, &dir.path().join("export.json")).unwrap();
    assert_eq!(
        read_ids(
            &Dimension::Overworld
                .directory(&exported)
                .join("region/r.-1.0.mca")
        ),
        source_ids,
    );
    assert_eq!(hashes(&source).unwrap(), before);

    // The immutable imported chunk still holds flowing IDs; only the
    // authoritative queue changes. Export must not recover IDs from that copy.
    manifest.fluid_ticks.due[0].fluid = mc_data::Identifier::parse("minecraft:water").unwrap();
    manifest.fluid_ticks.due[1].fluid = mc_data::Identifier::parse("minecraft:lava").unwrap();
    let store = crate::test_support::fixture_store(&config).unwrap();
    crate::test_support::write_manifest(&config, &store, &manifest).unwrap();
    drop(store);
    let changed = dir.path().join("changed");
    export_anvil(
        &config,
        &native,
        &changed,
        &dir.path().join("changed-export.json"),
    )
    .unwrap();
    assert_eq!(
        read_ids(
            &Dimension::Overworld
                .directory(&changed)
                .join("region/r.-1.0.mca")
        ),
        ["minecraft:water", "minecraft:lava"],
    );
}

#[test]
fn statistics_json_semantic_roundtrip_owns_legacy_and_offline_uuid_files() {
    use mc_data::statistics::{CustomStat, StatType};

    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("anvil");
    let native = directory.path().join("native");
    let output = directory.path().join("export");
    for path in [&source, &native, &output] {
        std::fs::create_dir(path).unwrap();
    }
    let config = crate::test_support::config(native.clone(), 42);
    modern_fixture(&source, &config);
    let player_uuid = uuid::Uuid::from_u128(3);
    let offline_uuid = uuid::Uuid::from_u128(900);
    let input = serde_json::json!({
        "DataVersion": 4200,
        "unknown_root": {"value": [7, null, true], "unicode": "保持"},
        "stats": {
            "minecraft:custom": {"minecraft:jump": 4, "minecraft:time_since_rest": 0, "future:stat": 99},
            "minecraft:mined": {"minecraft:stone": 8, "future:block": 9},
            "minecraft:crafted": {"minecraft:stone": 10},
            "minecraft:used": {"minecraft:stone": 11},
            "minecraft:broken": {"minecraft:stone": 12},
            "minecraft:picked_up": {"minecraft:stone": 13},
            "minecraft:dropped": {"minecraft:stone": 14},
            "minecraft:killed": {"minecraft:cow": 15},
            "minecraft:killed_by": {"minecraft:cow": 16},
            "future:category": {"future:stat": {"retained": true}}
        }
    });
    let offline_input = serde_json::json!({
        "DataVersion": 3999,
        "offline_unknown": 7,
        "stats": {"minecraft:custom": {"minecraft:play_time": 987}, "future:category": [1, 2]}
    });
    std::fs::write(
        source.join(format!("players/stats/{player_uuid}.json")),
        serde_json::to_vec(&input).unwrap(),
    )
    .unwrap();
    std::fs::create_dir(source.join("stats")).unwrap();
    std::fs::write(
        source.join(format!("stats/{offline_uuid}.json")),
        serde_json::to_vec(&offline_input).unwrap(),
    )
    .unwrap();
    let imported = import::convert(&config, &source, &native).unwrap();
    assert_eq!(imported.players, 2);
    let recovered = bridge::recover_offline(&config).unwrap();
    let players = &recovered.dimensions[&Dimension::Overworld].players;
    let player = &players[&player_uuid.to_string()];
    let offline = &players[&offline_uuid.to_string()];
    assert_eq!(
        bridge::statistics_to_json(&player.statistics, &recovered.registry).unwrap(),
        input
    );
    assert_eq!(
        bridge::statistics_to_json(&offline.statistics, &recovered.registry).unwrap(),
        offline_input
    );
    assert_eq!(offline.statistics.get_custom(CustomStat::PlayTime), 987);
    let stone = recovered
        .registry
        .blocks
        .block(&mc_data::Identifier::parse("minecraft:stone").unwrap())
        .unwrap()
        .raw_id as u32;
    assert_eq!(player.statistics.get(StatType::Mined, stone), 8);
    assert_eq!(player.statistics.wire_entries().len(), 9);
    let Tag::Compound(extras) =
        bridge::decode_tag(&recovered.checkpoint.globals["conversion_extras"], "extras").unwrap()
    else {
        panic!("extras compound")
    };
    assert!(
        extras
            .iter()
            .all(|(name, _)| !name.starts_with("stats/") && !name.starts_with("players/stats/"))
    );
    drop(recovered);
    let restarted = crate::test_support::Running::open(config.clone());
    assert!(restarted.core.checkpoint());
    drop(restarted);
    let recovered = bridge::recover_offline(&config).unwrap();
    assert_eq!(
        recovered.dimensions[&Dimension::Overworld].players[&offline_uuid.to_string()]
            .statistics
            .get_custom(CustomStat::PlayTime),
        987
    );
    drop(recovered);
    export::convert(&config, &native, &output).unwrap();
    for (uuid, expected) in [(player_uuid, input), (offline_uuid, offline_input)] {
        let actual: serde_json::Value = serde_json::from_slice(
            &std::fs::read(output.join(format!("players/stats/{uuid}.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(actual, expected);
        assert!(!output.join(format!("stats/{uuid}.json")).exists());
        let player = read_nbt(&output.join(format!("players/data/{uuid}.dat"))).unwrap();
        assert!(!has_field(&player, "KrasnokamenStatistics"));
    }
}

#[test]
fn duplicate_legacy_and_canonical_statistics_refuse_import_without_custody_loss() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("anvil");
    let native = directory.path().join("native");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&native).unwrap();
    let config = crate::test_support::config(native.clone(), 42);
    modern_fixture(&source, &config);
    std::fs::create_dir(source.join("stats")).unwrap();
    std::fs::write(
        source.join("stats/00000000-0000-0000-0000-000000000003.json"),
        b"{\"stats\":{}}",
    )
    .unwrap();
    let error = import::convert(&config, &source, &native)
        .unwrap_err()
        .to_string();
    assert!(error.contains("duplicate auxiliary destination"), "{error}");
    assert!(!native.join("native/state.redb").exists());
    assert_eq!(
        std::fs::read(source.join("stats/00000000-0000-0000-0000-000000000003.json")).unwrap(),
        b"{\"stats\":{}}"
    );
}

#[test]
fn advancements_survive_native_checkpoint_wal_and_lossless_anvil_interchange() {
    use mc_core::journal::{JournalEntry, JournalWriter};
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("anvil");
    let native = temp.path().join("native");
    let output = temp.path().join("export");
    let sealed_output = temp.path().join("checkpoint-export");
    let imported_again = temp.path().join("imported-again");
    let config = crate::test_support::config(native.clone(), 42);
    modern_fixture(&source, &config);
    let uuid = "00000000-0000-0000-0000-000000000003";
    let progress_path = format!("players/advancements/{uuid}.json");
    let legacy_progress_path = format!("advancements/{uuid}.json");
    let orphan_path = "players/advancements/00000000-0000-0000-0000-000000000004.json";
    let orphan_bytes = b"{\n \"DataVersion\": 4790, \"mod:orphan\": {\"done\":true,\"criteria\":{\"unknown\":\"2024-01-02 03:04:05 +0530\"}}\n}\n";
    let progress = serde_json::json!({
        "DataVersion": 4790,
        "minecraft:story/root": {
            "criteria": {"tick": "2026-10-14 11:12:13 +0000"},
            "done": true,
            "mod:extra": {"nested": [null, false, 9]}
        },
        "mod:unknown_advancement": {"criteria": {"future": "2020-01-02 03:04:05 -0700"}, "done": false},
        "unknown_root": [null, {"note": "untouched"}],
        "mod:large_document": "x".repeat(70_000)
    });
    std::fs::create_dir_all(source.join("advancements")).unwrap();
    std::fs::create_dir_all(source.join("players/advancements")).unwrap();
    std::fs::write(
        source.join(&legacy_progress_path),
        serde_json::to_vec_pretty(&progress).unwrap(),
    )
    .unwrap();
    std::fs::write(source.join(orphan_path), orphan_bytes).unwrap();
    let player_path = format!("players/data/{uuid}.dat");
    let mut player_tag = read_nbt(&source.join(&player_path)).unwrap();
    let Tag::Compound(fields) = &mut player_tag else {
        panic!("player compound")
    };
    let highlighted = Tag::List(mc_nbt::ListTag {
        element_type: mc_nbt::tag_type::STRING,
        elements: vec![Tag::String("minecraft:oak_planks".into())],
    });
    fields.push((
        "recipeBook".into(),
        Tag::Compound(vec![
            ("recipes".into(), highlighted.clone()),
            ("toBeDisplayed".into(), highlighted.clone()),
            ("isGuiOpen".into(), Tag::Byte(1)),
            ("mod:book_note".into(), Tag::Long(37)),
        ]),
    ));
    save(&source, &player_path, &player_tag);
    let original = hashes(&source).unwrap();
    let imported =
        import_anvil(&config, &source, &native, &temp.path().join("import.json")).unwrap();
    assert_eq!(
        imported.counts["players"], 1,
        "orphan advancements must not invent players"
    );
    assert!(!imported.opaque_retention.contains(&progress_path));
    assert!(imported.opaque_retention.contains(&orphan_path.to_owned()));
    assert_eq!(hashes(&source).unwrap(), original);
    let world = bridge::recover_offline(&config).unwrap();
    let record = &world.dimensions[&Dimension::Overworld].players[uuid];
    assert_eq!(*record.advancement_progress, progress);
    assert!(record.unlocked_recipes.contains("minecraft:oak_planks"));
    let mut updated = progress.clone();
    updated["minecraft:story/root"]["done"] = false.into();
    let recipes = [
        "minecraft:oak_planks".to_owned(),
        "minecraft:stick".to_owned(),
    ]
    .into_iter()
    .collect();
    let entry = JournalEntry::Player {
        uuid: uuid.into(),
        opaque_player_fields: record.opaque_player_fields.clone(),
        statistics: Box::new(record.statistics.clone()),
        advancement_progress: Some(std::sync::Arc::new(updated.clone())),
        unlocked_recipes: Some(std::sync::Arc::new(recipes)),
        dimension: record.dimension,
        pos: record.pos,
        yaw: record.yaw,
        pitch: record.pitch,
        health: record.health,
        food: record.food,
        saturation: record.saturation,
        exhaustion: record.exhaustion,
        air: record.air,
        experience_level: record.experience_level,
        experience_progress: record.experience_progress,
        experience_total: record.experience_total,
        game_mode: record.game_mode,
        selected: record.selected,
        inventory: record.inventory.clone(),
        carried: record.carried.clone(),
        grid: record.grid.clone(),
        respawn: record.respawn,
        respawn_dimension: record.respawn_dimension,
        respawn_forced: record.respawn_forced,
        respawn_yaw: record.respawn_yaw,
        respawn_pitch: record.respawn_pitch,
        active_effects: record.active_effects.clone(),
        enchantment_seed: record.enchantment_seed,
        enchantment_offer_seed: record.enchantment_offer_seed,
        warden_warning: record.warden_warning,
        current_impulse_impact_pos: record.current_impulse_impact_pos,
        fall_flying: record.fall_flying,
        burn_ticks: record.burn_ticks,
        portal: record.portal,
        tick: 80,
    };
    drop(world);
    let mut writer = JournalWriter::open(&native, 1).unwrap();
    writer.append(80, 80, &[entry]).unwrap();
    writer.flush().unwrap();
    drop(writer);
    let before = hashes(&native).unwrap();
    let exported =
        export_anvil(&config, &native, &output, &temp.path().join("export.json")).unwrap();
    assert_eq!(hashes(&native).unwrap(), before, "export is read-only");
    assert_eq!(exported.replay_tick, Some(80));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(
            &std::fs::read(output.join(&progress_path)).unwrap()
        )
        .unwrap(),
        updated
    );
    assert!(
        !output.join(&legacy_progress_path).exists(),
        "no dual advancement files"
    );
    assert_eq!(
        std::fs::read(output.join(orphan_path)).unwrap(),
        orphan_bytes
    );
    let world = bridge::recover_offline(&config).unwrap();
    let record = &world.dimensions[&Dimension::Overworld].players[uuid];
    assert_eq!(*record.advancement_progress, updated);
    assert!(record.unlocked_recipes.contains("minecraft:stick"));
    let mut captured = bridge::encode_checkpoint(&world).unwrap();
    drop(world);
    // A modeled record always supersedes stale opaque custody from older
    // imports; neither canonical nor legacy extras may overwrite this update.
    let mut extras =
        bridge::decode_tag(&captured.globals["conversion_extras"], "conversion_extras").unwrap();
    let Tag::Compound(fields) = &mut extras else {
        panic!("extras compound")
    };
    let stale = Tag::ByteArray(
        serde_json::to_vec(&progress)
            .unwrap()
            .into_iter()
            .map(|byte| byte as i8)
            .collect(),
    );
    fields.push((progress_path.clone(), stale.clone()));
    fields.push((legacy_progress_path.clone(), stale));
    captured.globals.insert(
        "conversion_extras".into(),
        bridge::tag_record(80, &extras).unwrap(),
    );
    let store = NativeStore::open(
        &native.join("native/state.redb"),
        bridge::native_identity(&config),
    )
    .unwrap();
    let registry = crate::test_support::registry().unwrap();
    store
        .commit_checkpoint(
            NativeCheckpoint {
                tick: 80,
                previous_tick: 79,
                chunks: &[],
                players: &captured.players,
                entities: &captured.entities,
                globals: &captured.globals,
            },
            &registry.blocks,
            &registry.items,
        )
        .unwrap();
    drop(store);
    export_anvil(
        &config,
        &native,
        &sealed_output,
        &temp.path().join("checkpoint-export.json"),
    )
    .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(
            &std::fs::read(sealed_output.join(&progress_path)).unwrap()
        )
        .unwrap(),
        updated
    );
    assert!(!sealed_output.join(&legacy_progress_path).exists());
    let tag = read_nbt(&sealed_output.join(&player_path)).unwrap();
    assert!(!has_field(&tag, "KrasnokamenAdvancements"));
    assert!(!has_field(&tag, "KrasnokamenUnlockedRecipes"));
    let Tag::Compound(fields) = &tag else {
        panic!("player compound")
    };
    let Some(Tag::Compound(book)) = mc_data::item_nbt::field(fields, "recipeBook") else {
        panic!("recipeBook compound")
    };
    assert_eq!(
        mc_data::item_nbt::field(book, "toBeDisplayed"),
        Some(&highlighted)
    );
    assert_eq!(
        mc_data::item_nbt::field(book, "isGuiOpen"),
        Some(&Tag::Byte(1))
    );
    assert_eq!(
        mc_data::item_nbt::field(book, "mod:book_note"),
        Some(&Tag::Long(37))
    );
    import_anvil(
        &config,
        &sealed_output,
        &imported_again,
        &temp.path().join("reimport.json"),
    )
    .unwrap();
    let mut config = config;
    config.world_dir = imported_again;
    let world = bridge::recover_offline(&config).unwrap();
    let record = &world.dimensions[&Dimension::Overworld].players[uuid];
    assert_eq!(*record.advancement_progress, updated);
    assert_eq!(
        *record.unlocked_recipes,
        [
            "minecraft:oak_planks".to_owned(),
            "minecraft:stick".to_owned()
        ]
        .into_iter()
        .collect()
    );
    assert_eq!(world.dimensions[&Dimension::Overworld].players.len(), 1);
}

#[test]
fn duplicate_legacy_and_modern_advancement_destinations_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("anvil");
    let native = temp.path().join("native");
    let config = crate::test_support::config(native.clone(), 42);
    modern_fixture(&source, &config);
    for directory in ["advancements", "players/advancements"] {
        std::fs::create_dir_all(source.join(directory)).unwrap();
        std::fs::write(
            source
                .join(directory)
                .join("00000000-0000-0000-0000-000000000003.json"),
            b"{}",
        )
        .unwrap();
    }
    assert!(import_anvil(&config, &source, &native, &temp.path().join("import.json")).is_err());
    assert!(!native.exists());
}

#[test]
fn container_metadata_survives_native_import_export_without_restoring_inventory() {
    use mc_world::{BlockPos, BlockStateId, Chunk, ChunkPos};
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("anvil");
    let native = temp.path().join("native");
    let exported = temp.path().join("export");
    std::fs::create_dir(&source).unwrap();
    let config = crate::test_support::config(native.clone(), 42);
    modern_fixture(&source, &config);
    let registry = crate::test_support::registry().unwrap();
    let mut chunk = Chunk::empty(
        ChunkPos { x: 0, z: 0 },
        BlockStateId(registry.states.air),
        mc_data::Identifier::parse("minecraft:plains").unwrap(),
    );
    chunk.status = "minecraft:full".into();
    for (x, name) in [
        "chest",
        "barrel",
        "furnace",
        "blast_furnace",
        "smoker",
        "hopper",
        "brewing_stand",
    ]
    .into_iter()
    .enumerate()
    {
        let id = mc_data::Identifier::parse(format!("minecraft:{name}")).unwrap();
        chunk
            .set_block(x as u8, 80, 0, registry.blocks.block(&id).unwrap().default)
            .unwrap();
        let tag = Tag::Compound(vec![
            ("id".into(), Tag::String(id.to_string())),
            ("x".into(), Tag::Int(x as i32)),
            ("y".into(), Tag::Int(80)),
            ("z".into(), Tag::Int(0)),
            (
                "CustomName".into(),
                Tag::Compound(vec![("text".into(), Tag::String(format!("named-{name}")))]),
            ),
            ("lock".into(), Tag::Compound(Vec::new())),
            (
                "components".into(),
                Tag::Compound(vec![("test:payload".into(), Tag::Long(37))]),
            ),
        ]);
        let mut bytes = Vec::new();
        mc_nbt::write_network(&mut bytes, &tag).unwrap();
        chunk.block_entities.insert(
            BlockPos {
                x: x as i32,
                y: 80,
                z: 0,
            },
            bytes,
        );
    }
    let path = Dimension::Overworld
        .directory(&source)
        .join("region/r.0.0.mca");
    let payload = mc_world::anvil::chunk_to_payload_with_items(
        &chunk,
        &registry.blocks,
        Some(&registry.items),
        0,
    )
    .unwrap();
    mc_world::anvil::write_region(&path, &[payload]).unwrap();
    import_anvil(&config, &source, &native, &temp.path().join("import.json")).unwrap();
    export_anvil(
        &config,
        &native,
        &exported,
        &temp.path().join("export.json"),
    )
    .unwrap();
    let bytes = mc_world::anvil::read_chunk(
        Dimension::Overworld
            .directory(&exported)
            .join("region/r.0.0.mca"),
        0,
        0,
    )
    .unwrap()
    .unwrap();
    let (_, tag) = mc_nbt::read_named(&mut bytes.uncompressed_nbt.as_slice()).unwrap();
    let Tag::Compound(fields) = tag else {
        panic!("chunk")
    };
    let (_, Tag::List(entities)) = fields
        .iter()
        .find(|(key, _)| key == "block_entities")
        .unwrap()
    else {
        panic!("entities")
    };
    assert_eq!(entities.elements.len(), 7);
    for entity in &entities.elements {
        let Tag::Compound(fields) = entity else {
            panic!("container")
        };
        for key in ["CustomName", "lock", "components"] {
            assert!(
                fields.iter().any(|(name, _)| name == key),
                "lost {key}: {entity:?}"
            );
        }
        let Some(Tag::List(items)) = mc_data::item_nbt::field(fields, "Items") else {
            panic!("inventory")
        };
        assert!(items.elements.is_empty());
    }
}
