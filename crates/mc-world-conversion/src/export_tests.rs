use super::{ConversionReceipt, export_anvil, files, hashes, read_nbt};
use crate::test_support::{fixture_store, read_manifest, write_manifest};
use mc_core::block_ticks::{BlockTickChunkImage, BlockTickImage};
use mc_core::dimensions::Dimension;
use mc_core::journal::{JournalEntry, JournalWriter};
use mc_core::registry::CoreRegistry;
use mc_data::Identifier;
use mc_nbt::Tag;
use mc_world::{BlockStateId, Chunk, ChunkPos};
use std::path::Path;

pub(super) fn read_exported_chunk(
    root: &Path,
    dimension: Dimension,
    pos: ChunkPos,
    registry: &CoreRegistry,
) -> Chunk {
    let path = dimension.directory(root).join("region").join(format!(
        "r.{}.{}.mca",
        pos.x.div_euclid(32),
        pos.z.div_euclid(32)
    ));
    let payload =
        mc_world::anvil::read_chunk(path, pos.x.rem_euclid(32) as u8, pos.z.rem_euclid(32) as u8)
            .unwrap()
            .expect("exported chunk slot");
    let Tag::Compound(level) = read_nbt(&root.join("level.dat")).unwrap() else {
        panic!("level.dat root compound");
    };
    let Tag::Compound(data) = mc_data::item_nbt::field(&level, "Data").unwrap() else {
        panic!("level Data compound");
    };
    let Tag::Long(current_tick) = mc_data::item_nbt::field(data, "Time").unwrap() else {
        panic!("level Time long");
    };
    mc_world::anvil::chunk_from_payload_with_items_at_position_at_tick(
        &payload.uncompressed_nbt,
        pos,
        &registry.blocks,
        Some(&registry.items),
        &registry.chest_loot,
        dimension.geometry(),
        u64::try_from(*current_tick).unwrap(),
    )
    .unwrap()
}

#[test]
fn inhabited_time_child_dimension_replay_uses_empty_global_batch_clock() {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("native-world");
    let destination = temporary.path().join("anvil-world");
    let config = crate::test_support::config(source.clone(), 42);
    let registry = std::sync::Arc::new(crate::test_support::registry().unwrap());
    let store = fixture_store(&config).unwrap();
    let mut writer = JournalWriter::open(&source, 1).unwrap();
    writer
        .append(
            1,
            1,
            &[JournalEntry::Dimension {
                dimension: Dimension::Nether,
                entries: vec![JournalEntry::InhabitedTime {
                    image: mc_core::journal::InhabitedTimeImage {
                        chunk: [0, 0],
                        value: 1,
                        tick: 1,
                        active: true,
                    },
                }],
            }],
        )
        .unwrap();
    writer.append(2, 9, &[]).unwrap();
    writer.flush().unwrap();
    let recovered = mc_core::native_checkpoint::recover_with_reader(
        &config,
        std::sync::Arc::clone(&registry),
        store.checkpoint_reader().unwrap(),
    )
    .unwrap();
    let nether = &recovered.dimensions[&Dimension::Nether];
    assert_eq!(nether.tick, 9);
    let mut chunk = Chunk::empty_with_geometry(
        ChunkPos { x: 0, z: 0 },
        BlockStateId(registry.states.air),
        Identifier::parse("minecraft:plains").unwrap(),
        Dimension::Nether.geometry(),
    );
    nether.inhabited_time[&[0, 0]].apply(&mut chunk, nether.tick);
    assert_eq!(chunk.inhabited_time(), 9);
    assert!(
        recovered.dimensions[&Dimension::Overworld]
            .inhabited_time
            .is_empty()
    );
    drop(recovered);
    drop(writer);
    drop(store);
    export_anvil(
        &config,
        &source,
        &destination,
        &temporary.path().join("export.json"),
    )
    .unwrap();
    assert_eq!(
        read_exported_chunk(
            &destination,
            Dimension::Nether,
            ChunkPos { x: 0, z: 0 },
            &registry
        )
        .inhabited_time(),
        9,
    );
}

#[test]
fn inhabited_time_checkpoint_tail_and_anvil_export_share_wal_tick() {
    use mc_core::journal::InhabitedTimeImage;
    use mc_core::native_checkpoint::{encode_checkpoint, recover_with_reader};
    use mc_world::native::NativeCheckpoint;
    use std::sync::Arc;

    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("native-world");
    let destination = temporary.path().join("anvil-world");
    let config = crate::test_support::config(source.clone(), 42);
    let registry = Arc::new(crate::test_support::registry().unwrap());
    let store = fixture_store(&config).unwrap();
    let pos = ChunkPos { x: 0, z: 0 };
    let chunk = Chunk::empty(
        pos,
        BlockStateId(registry.states.air),
        Identifier::parse("minecraft:plains").unwrap(),
    );
    store
        .fill_generated(0, &[chunk], &registry.blocks, &registry.items)
        .unwrap();
    let mut writer = JournalWriter::open(&source, 1).unwrap();
    writer
        .append(
            1,
            1,
            &[JournalEntry::InhabitedTime {
                image: InhabitedTimeImage {
                    chunk: [0, 0],
                    value: 1,
                    tick: 1,
                    active: true,
                },
            }],
        )
        .unwrap();
    for tick in 2..=12 {
        writer.append(tick, tick, &[]).unwrap();
    }
    writer.flush().unwrap();
    let recovered = recover_with_reader(
        &config,
        Arc::clone(&registry),
        store.checkpoint_reader().unwrap(),
    )
    .unwrap();
    assert_eq!(recovered.dimensions[&Dimension::Overworld].tick, 12);
    let image = encode_checkpoint(&recovered).unwrap();
    let manifest: mc_core::journal::CheckpointManifest =
        serde_json::from_slice(&image.globals["checkpoint"].payload).unwrap();
    assert_eq!(
        manifest.inhabited_time,
        vec![InhabitedTimeImage {
            chunk: [0, 0],
            value: 12,
            tick: 12,
            active: true,
        }]
    );
    drop(recovered);
    store
        .commit_checkpoint(
            NativeCheckpoint {
                tick: 12,
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
    for tick in 13..=24 {
        let image = match tick {
            19 => Some(InhabitedTimeImage {
                chunk: [0, 0],
                value: 18,
                tick,
                active: false,
            }),
            23 => Some(InhabitedTimeImage {
                chunk: [0, 0],
                value: 19,
                tick,
                active: true,
            }),
            _ => None,
        };
        let entries: Vec<_> = image
            .into_iter()
            .map(|image| JournalEntry::InhabitedTime { image })
            .collect();
        writer.append(tick, tick, &entries).unwrap();
    }
    writer.flush().unwrap();
    drop(writer);
    drop(store);
    let receipt = export_anvil(
        &config,
        &source,
        &destination,
        &temporary.path().join("export.json"),
    )
    .unwrap();
    assert_eq!(receipt.checkpoint_tick, Some(12));
    assert_eq!(receipt.replay_tick, Some(24));
    let exported = read_exported_chunk(&destination, Dimension::Overworld, pos, &registry);
    assert_eq!(exported.inhabited_time(), 20, "inactive ticks never accrue");
}

#[test]
fn export_replays_wal_without_mutating_native_or_journal_and_enumerates_all_dimensions() {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("native-world");
    let destination = temporary.path().join("anvil-world");
    let receipt_path = temporary.path().join("export.json");
    let config = crate::test_support::config(source.clone(), 42);
    let registry = crate::test_support::registry().unwrap();
    let store = fixture_store(&config).unwrap();
    let stored_pos = ChunkPos { x: 2, z: 2 };
    for dimension in Dimension::ALL {
        let chunk = Chunk::empty_with_geometry(
            stored_pos,
            BlockStateId(registry.states.air),
            Identifier::parse("minecraft:plains").unwrap(),
            dimension.geometry(),
        );
        store
            .fill_generated(
                mc_core::native_checkpoint::dimension_id(dimension),
                &[chunk],
                &registry.blocks,
                &registry.items,
            )
            .unwrap();
    }
    drop(store);
    let edits = Dimension::ALL
        .into_iter()
        .map(|dimension| {
            let entry = JournalEntry::Block {
                pos: [1, 80, 1],
                state: registry.states.stone,
            };
            if dimension == Dimension::Overworld {
                entry
            } else {
                JournalEntry::Dimension {
                    dimension,
                    entries: vec![entry],
                }
            }
        })
        .collect::<Vec<_>>();
    let mut writer = JournalWriter::open(&source, 1).unwrap();
    writer.append(1, 1, &edits).unwrap();
    writer.flush().unwrap();
    drop(writer);
    let before = hashes(&source).unwrap();
    let receipt = export_anvil(&config, &source, &destination, &receipt_path).unwrap();
    assert_eq!(receipt.status, "PASS");
    assert_eq!(receipt.checkpoint_tick, Some(0));
    assert_eq!(receipt.replay_tick, Some(1));
    assert_eq!(receipt.counts["chunks"], 6);
    assert_eq!(receipt.input_hashes, before);
    assert_eq!(hashes(&source).unwrap(), before);
    assert_eq!(receipt.output_hashes, hashes(&destination).unwrap());
    for dimension in Dimension::ALL {
        let changed =
            read_exported_chunk(&destination, dimension, ChunkPos { x: 0, z: 0 }, &registry);
        assert_eq!(
            changed.get_block(1, 80, 1),
            Some(BlockStateId(registry.states.stone))
        );
        assert!(matches!(
            changed.extras.iter().find(|(key, _)| key == "LastUpdate"),
            Some((_, Tag::Long(1)))
        ));
        let stored = read_exported_chunk(&destination, dimension, stored_pos, &registry);
        assert_eq!(
            stored.get_block(1, 80, 1),
            Some(BlockStateId(registry.states.air))
        );
    }
    let level = read_nbt(&destination.join("level.dat")).unwrap();
    let Tag::Compound(root) = level else {
        panic!("level.dat root compound");
    };
    let Tag::Compound(data) = mc_data::item_nbt::field(&root, "Data").unwrap() else {
        panic!("level Data compound");
    };
    assert_eq!(mc_data::item_nbt::field(data, "Time"), Some(&Tag::Long(1)));
    for path in files(&destination).unwrap() {
        assert!(!path.starts_with("native"));
        assert!(!path.starts_with("journal"));
        assert_ne!(path, Path::new("manifest.json"));
        assert_ne!(path, Path::new("world-contract.json"));
    }
    let persisted: ConversionReceipt =
        serde_json::from_slice(&std::fs::read(receipt_path).unwrap()).unwrap();
    assert_eq!(persisted.status, "PASS");
    assert_eq!(persisted.checkpoint_tick, Some(0));
    assert_eq!(persisted.replay_tick, Some(1));
}

fn assert_checkpoint_delay_overflow_refuses_publication(fluid: bool) {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("native-world");
    let destination = temporary.path().join("anvil-world");
    let receipt_path = temporary.path().join("export.json");
    let config = crate::test_support::config(source.clone(), 42);
    let registry = crate::test_support::registry().unwrap();
    let store = fixture_store(&config).unwrap();
    let pos = ChunkPos { x: 0, z: 0 };
    let chunk = Chunk::empty(
        pos,
        BlockStateId(registry.states.air),
        Identifier::parse("minecraft:plains").unwrap(),
    );
    store
        .fill_generated(0, &[chunk], &registry.blocks, &registry.items)
        .unwrap();
    let mut manifest = read_manifest(&store).unwrap();
    manifest.tick = 1;
    let deadline = manifest.tick + i32::MAX as u64 + 1;
    if fluid {
        manifest.fluid_ticks = mc_core::fluids::FluidTicksImage {
            due: vec![mc_core::fluids::FluidTickImage {
                pos: [1, 80, 1],
                fluid: Identifier::parse("minecraft:water").unwrap(),
                deadline,
                priority: 0,
                sequence: 0,
            }],
            covered: vec![[0, 0]],
            next_sequence: 1,
        };
    } else {
        manifest.block_ticks.next_sequence = 1;
        manifest.block_ticks.chunks = vec![BlockTickChunkImage {
            chunk: [0, 0],
            ticks: vec![BlockTickImage {
                pos: [1, 80, 1],
                block: Identifier::parse("minecraft:stone").unwrap(),
                trigger_tick: deadline,
                priority: 0,
                sequence: 0,
            }],
        }];
    }
    write_manifest(&config, &store, &manifest).unwrap();
    drop(store);
    let before = hashes(&source).unwrap();
    let error = export_anvil(&config, &source, &destination, &receipt_path).unwrap_err();
    assert!(
        error.to_string().contains("does not fit in Anvil int"),
        "unexpected error: {error}"
    );
    assert!(
        !destination.exists(),
        "overflow must not publish a partial world"
    );
    assert_eq!(hashes(&source).unwrap(), before);
    let receipt: ConversionReceipt =
        serde_json::from_slice(&std::fs::read(receipt_path).unwrap()).unwrap();
    assert_eq!(receipt.status, "FAIL");
    assert!(receipt.error.unwrap().contains("does not fit in Anvil int"));
    assert!(
        std::fs::read_dir(temporary.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".krasnokamen-conversion-")
        }),
        "failed staging directory must be removed"
    );
}

#[test]
fn export_rejects_checkpoint_block_tick_delay_overflow_without_clamping() {
    assert_checkpoint_delay_overflow_refuses_publication(false);
}

#[test]
fn export_rejects_checkpoint_fluid_tick_delay_overflow_without_clamping() {
    assert_checkpoint_delay_overflow_refuses_publication(true);
}

#[tokio::test]
async fn export_import_does_not_respawn_claimed_dead_generated_villager() {
    use std::sync::Arc;
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("native-source");
    let anvil = temporary.path().join("anvil");
    let restored = temporary.path().join("native-restored");
    let config = crate::test_support::config(source.clone(), 42);
    let registry = Arc::new(crate::test_support::registry().unwrap());
    let store = fixture_store(&config).unwrap();
    let pos = ChunkPos { x: 0, z: 0 };
    let dead_claim = "village:0:0:resident:dead";
    let pending_claim = "village:0:0:resident:pending";
    let marker = |claim: &str, position| mc_world::GeneratedEntityMarker {
        claim: claim.into(),
        entity_type: "minecraft:villager".into(),
        position,
        persistent: false,
        villager_kind: "minecraft:plains".into(),
        profession: "minecraft:farmer".into(),
        level: 1,
        age: 0,
        yaw: 0.0,
        pitch: 0.0,
    };
    let mut chunk = Chunk::empty(
        pos,
        BlockStateId(registry.states.air),
        Identifier::parse("minecraft:plains").unwrap(),
    );
    for x in 0..16 {
        for z in 0..16 {
            chunk
                .set_block(x, 79, z, BlockStateId(registry.states.stone))
                .unwrap();
        }
    }
    chunk.set_generated_entities(&[
        marker(dead_claim, [1.5, 80.0, 1.5]),
        marker(pending_claim, [2.5, 80.0, 2.5]),
    ]);
    let (_, Tag::List(markers)) = chunk
        .extras
        .iter_mut()
        .find(|(key, _)| key == "KrasnokamenGeneratedEntities")
        .unwrap()
    else {
        panic!("generated marker list");
    };
    let Tag::Compound(pending) = &mut markers.elements[1] else {
        panic!("generated marker compound");
    };
    pending.push((
        "RetainedMarkerNote".into(),
        Tag::String("pending opaque field".into()),
    ));
    store
        .fill_generated(0, &[chunk], &registry.blocks, &registry.items)
        .unwrap();
    drop(store);
    // The claim survives its entity's death; no live UUID exists from which
    // import could reconstruct it. Put it in the WAL to cover replay claims.
    let mut writer = JournalWriter::open(&source, 1).unwrap();
    writer
        .append(
            1,
            1,
            &[JournalEntry::GeneratedEntityClaim {
                claim: dead_claim.into(),
            }],
        )
        .unwrap();
    writer.flush().unwrap();
    drop(writer);
    let before = hashes(&source).unwrap();
    let receipt = export_anvil(
        &config,
        &source,
        &anvil,
        &temporary.path().join("export.json"),
    )
    .unwrap();
    assert_eq!(receipt.replay_tick, Some(1));
    assert_eq!(hashes(&source).unwrap(), before);
    let exported = read_exported_chunk(&anvil, Dimension::Overworld, pos, &registry);
    let pending = exported.generated_entities();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].claim, pending_claim);
    let (_, Tag::List(markers)) = exported
        .extras
        .iter()
        .find(|(key, _)| key == "KrasnokamenGeneratedEntities")
        .unwrap()
    else {
        panic!("retained generated marker list");
    };
    let Tag::Compound(fields) = &markers.elements[0] else {
        panic!("retained marker compound");
    };
    assert_eq!(
        mc_data::item_nbt::field(fields, "RetainedMarkerNote"),
        Some(&Tag::String("pending opaque field".into()))
    );
    super::import_anvil(
        &config,
        &anvil,
        &restored,
        &temporary.path().join("import.json"),
    )
    .unwrap();
    let restored_config = crate::test_support::config(restored, config.seed);
    let recovered = mc_core::native_checkpoint::recover_offline(&restored_config).unwrap();
    assert!(
        recovered.dimensions[&Dimension::Overworld]
            .entities
            .is_empty()
    );
    assert!(
        !recovered.dimensions[&Dimension::Overworld]
            .generated_entity_claims
            .contains(dead_claim)
    );
    drop(recovered);
    async fn loaded_villager(
        runtime: &mut crate::test_support::Running,
        type_id: u32,
    ) -> uuid::Uuid {
        runtime
            .until(|output, _| {
                if !output
                    .snapshot
                    .chunks
                    .iter()
                    .any(|chunk| chunk.pos == (ChunkPos { x: 0, z: 0 }))
                {
                    return None;
                }
                let mut villagers = output
                    .snapshot
                    .entities
                    .iter()
                    .filter(|entity| entity.type_id == type_id);
                let villager = villagers.next()?;
                assert!(
                    villagers.next().is_none(),
                    "only the unclaimed control marker may spawn"
                );
                Some(villager.uuid)
            })
            .await
    }
    let villager_type = registry
        .entity_types
        .id_of(&Identifier::parse("minecraft:villager").unwrap())
        .unwrap();
    let mut runtime = crate::test_support::Running::open(restored_config.clone());
    let (command, _) = mc_core::command_console::ConsoleCommand::new("forceload add 0 0".into());
    assert!(
        runtime
            .core
            .submit(mc_core::boundary::PlayerInput::ConsoleCommand(command))
    );
    let uuid = loaded_villager(&mut runtime, villager_type).await;
    assert!(runtime.core.checkpoint());
    drop(runtime);
    let recovered = mc_core::native_checkpoint::recover_offline(&restored_config).unwrap();
    let replay = &recovered.dimensions[&Dimension::Overworld];
    assert_eq!(replay.entities.len(), 1);
    assert!(replay.generated_entity_claims.contains(pending_claim));
    assert!(
        !replay.generated_entity_claims.contains(dead_claim),
        "dead placement must not respawn"
    );
    drop(recovered);

    // Reload the same resident chunk after a real checkpoint: its remaining
    // marker must not duplicate the surviving entity.
    let mut runtime = crate::test_support::Running::open(restored_config.clone());
    assert_eq!(loaded_villager(&mut runtime, villager_type).await, uuid);
    assert!(runtime.core.checkpoint());
    drop(runtime);
    let recovered = mc_core::native_checkpoint::recover_offline(&restored_config).unwrap();
    let replay = &recovered.dimensions[&Dimension::Overworld];
    assert_eq!(
        replay.entities.len(),
        1,
        "pending marker is still once-only"
    );
    assert!(replay.generated_entity_claims.contains(pending_claim));
    assert!(!replay.generated_entity_claims.contains(dead_claim));
}

#[test]
fn export_hopper_deadlines_become_bounded_portable_remaining_cooldowns() {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("native-world");
    let destination = temporary.path().join("anvil-world");
    let config = crate::test_support::config(source.clone(), 42);
    let registry = crate::test_support::registry().unwrap();
    drop(fixture_store(&config).unwrap());
    let hopper = registry
        .blocks
        .by_name_and_props(
            &Identifier::parse("minecraft:hopper").unwrap(),
            &[
                ("enabled".into(), "true".into()),
                ("facing".into(), "down".into()),
            ],
        )
        .unwrap()
        .0;
    let mut entries = Vec::new();
    for (index, deadline) in [103, 90, 1_000, -1].into_iter().enumerate() {
        let x = index as i32 + 1;
        let tag = Tag::Compound(vec![
            ("id".into(), Tag::String("minecraft:hopper".into())),
            ("x".into(), Tag::Int(x)),
            ("y".into(), Tag::Int(80)),
            ("z".into(), Tag::Int(1)),
            ("TransferCooldown".into(), Tag::Int(8)),
            (
                "krasnokamen:TransferCooldownUntil".into(),
                Tag::Long(deadline),
            ),
        ]);
        let mut bytes = Vec::new();
        mc_nbt::write_network(&mut bytes, &tag).unwrap();
        entries.push(JournalEntry::Block {
            pos: [x, 80, 1],
            state: hopper,
        });
        entries.push(JournalEntry::OpaqueBlockEntity {
            pos: [x, 80, 1],
            nbt: Some(bytes),
        });
    }
    let mut writer = JournalWriter::open(&source, 1).unwrap();
    writer.append(1, 100, &entries).unwrap();
    writer.flush().unwrap();
    drop(writer);
    let before = hashes(&source).unwrap();
    let receipt = export_anvil(
        &config,
        &source,
        &destination,
        &temporary.path().join("export.json"),
    )
    .unwrap();
    assert_eq!(receipt.replay_tick, Some(100));
    assert_eq!(hashes(&source).unwrap(), before);
    let chunk = read_exported_chunk(
        &destination,
        Dimension::Overworld,
        ChunkPos { x: 0, z: 0 },
        &registry,
    );
    for (index, remaining) in [3, 0, 8, 0].into_iter().enumerate() {
        let pos = mc_world::BlockPos {
            x: index as i32 + 1,
            y: 80,
            z: 1,
        };
        assert_eq!(chunk.hoppers[&pos].transfer_cooldown, remaining);
        if let Some(bytes) = chunk.block_entities.get(&pos) {
            let Tag::Compound(fields) = mc_nbt::read_network(&mut bytes.as_slice()).unwrap() else {
                panic!("hopper compound");
            };
            assert!(
                mc_data::item_nbt::field(&fields, "krasnokamen:TransferCooldownUntil").is_none()
            );
        }
    }
}
