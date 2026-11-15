//! Checkpoint container selection and projection, through the actual resident capture path.
use super::*;
use crate::registry::CoreRegistry;

struct EmptyGenerator(BlockStateId);

impl ChunkGenerator for EmptyGenerator {
    fn generate(&self, pos: ChunkPos) -> mc_world::Chunk {
        mc_world::Chunk::empty(
            pos,
            self.0,
            mc_data::Identifier::parse("minecraft:plains").unwrap(),
        )
    }
}

fn fixture() -> (tempfile::TempDir, Arc<CoreRegistry>, ChunkWorld) {
    let registry = Arc::new(CoreRegistry::load(None).unwrap());
    let directory = tempfile::tempdir().unwrap();
    let store = world_tests::native_store(directory.path(), &registry);
    let air = BlockStateId(registry.states.air);
    let world = ChunkWorld::new_native(
        42,
        store,
        0,
        Arc::new(EmptyGenerator(air)),
        &registry,
        air,
        [0, 64, 0],
        mc_world::OVERWORLD_GEOMETRY,
    );
    (directory, registry, world)
}

#[cfg(debug_assertions)]
#[test]
fn healthy_container_capture_visits_keys_once_not_once_per_dirty_chunk() {
    let (_directory, registry, mut world) = fixture();
    for start in (0..2000).step_by(GENERATED_PENDING_MAX) {
        let positions = (start..(start + GENERATED_PENDING_MAX as i32).min(2000))
            .map(|x| (ChunkPos { x, z: 0 }, 0))
            .collect();
        world.ensure_blocking(&positions);
        world.take_generated(0).unwrap().fill_generated().unwrap();
    }
    let written = world.capture_dirty(&Default::default()).written();
    world.mark_flushed(&written);
    for x in 0..2000 {
        world.set_block(x * 16, 100, 0, registry.states.stone);
    }
    let image = crate::journal::ReplayBlockEntity {
        slots: vec![mc_data::ItemStack::EMPTY; 27],
        furnace: None,
        tick: 0,
    };
    let records = (0..10000)
        .map(|i| ((32_016 + i, 64, 0), image.clone()))
        .collect();
    let capture = world.capture_dirty(&records);
    assert!(capture.refused().is_empty(), "healthy-resident path only");
    assert_eq!(capture.written().len(), 2000);
    assert_eq!(
        capture.container_work.key_visits, 10000,
        "{:#?}",
        capture.container_work
    );
    assert_eq!(capture.container_work.chunk_lookups, 2000);
    assert_eq!(capture.container_work.projected, 0);
}

/// Selection/projection reference from before CAF-04: deliberately keep the
/// full-map filter here, independent of the production chunk buckets.
fn project_using_original_full_map(
    chunk: &mut Chunk,
    pos: ChunkPos,
    records: &crate::core::CheckpointBlockEntities,
    registry: &BlockRegistry,
) {
    chunk.chests.retain(|block, chest| {
        !ChunkWorld::in_chunk(*block, pos)
            || (chest.loot_table.is_some() && !records.contains_key(&(block.x, block.y, block.z)))
    });
    chunk
        .furnaces
        .retain(|block, _| !ChunkWorld::in_chunk(*block, pos));
    chunk
        .hoppers
        .retain(|block, _| !ChunkWorld::in_chunk(*block, pos));
    chunk
        .brewing_stands
        .retain(|block, _| !ChunkWorld::in_chunk(*block, pos));
    for (&(x, y, z), record) in records {
        let block = mc_world::BlockPos { x, y, z };
        if !ChunkWorld::in_chunk(block, pos) {
            continue;
        }
        use mc_data::block_entity_kinds::BlockEntityKind;
        let kind = chunk
            .get_block(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
            .and_then(|id| registry.by_id(id))
            .and_then(|state| BlockEntityKind::for_block(state.block.id.path()));
        if matches!(
            kind,
            Some(
                BlockEntityKind::Furnace | BlockEntityKind::Smoker | BlockEntityKind::BlastFurnace
            )
        ) {
            chunk.furnaces.insert(
                block,
                mc_world::FurnaceBlockEntity {
                    slots: ChunkWorld::furnace_slots(&record.slots),
                    burn_remaining: record
                        .furnace
                        .as_ref()
                        .map(|image| image.lit_remaining as i16)
                        .unwrap_or(0),
                    burn_total: record
                        .furnace
                        .as_ref()
                        .map(|image| image.lit_total as i16)
                        .unwrap_or(0),
                    cook_progress: record
                        .furnace
                        .as_ref()
                        .map(|image| image.cook_progress as i16)
                        .unwrap_or(0),
                    cook_total: record
                        .furnace
                        .as_ref()
                        .map(|image| image.cook_total as i16)
                        .unwrap_or(200),
                    recipes_used: record
                        .furnace
                        .as_ref()
                        .map(|image| (*image.recipes_used).clone())
                        .unwrap_or_default(),
                },
            );
        } else if matches!(
            kind,
            Some(BlockEntityKind::Chest | BlockEntityKind::TrappedChest | BlockEntityKind::Barrel)
        ) {
            let mut slots = std::array::from_fn(|_| mc_world::FurnaceSlot::default());
            for (index, stack) in record.slots.iter().take(27).enumerate() {
                slots[index] = mc_world::FurnaceSlot {
                    count: stack.count,
                    item_id: stack.item_id,
                    damage: stack.damage,
                    enchantments: stack.enchantments.clone(),
                    custom_name: stack.custom_name.as_deref().map(str::to_owned),
                    item_model: stack.item_model.clone(),
                    stew_effects: stack.stew_effects.clone(),
                    components: stack.components.clone(),
                };
            }
            chunk.chests.insert(
                block,
                mc_world::ChestBlockEntity {
                    slots,
                    loot_table: None,
                    loot_seed: 0,
                },
            );
        }
    }
}

#[test]
fn container_buckets_preserve_full_native_images_at_signed_chunk_boundaries() {
    use mc_world::chunk::BrewingStandBlockEntity;
    use mc_world::{
        BlockPos, ChestBlockEntity, FurnaceBlockEntity, FurnaceSlot, HopperBlockEntity,
    };

    let (_directory, registry, mut world) = fixture();
    let kinds = [
        ("chest", 27),
        ("trapped_chest", 27),
        ("barrel", 27),
        ("furnace", 3),
        ("smoker", 3),
        ("blast_furnace", 3),
        ("hopper", 5),
        ("brewing_stand", 5),
        ("oak_shelf", 3),
        ("crafter", 9),
    ];
    let mut cells = Vec::new();
    for x in [-17, -16, -1, 0, 15, 16] {
        for z in [-17, -16, -1, 0, 15, 16] {
            let (kind, size) = kinds[cells.len() % kinds.len()];
            cells.push((BlockPos { x, y: 64, z }, kind, size, true));
        }
    }
    let removed_chest = BlockPos { x: 48, y: 64, z: 0 };
    let removed_furnace = BlockPos {
        x: 64,
        ..removed_chest
    };
    let retained_loot = BlockPos {
        x: 80,
        ..removed_chest
    };
    let cleared_loot = BlockPos {
        x: 96,
        ..removed_chest
    };
    let isolated_hopper = BlockPos {
        x: 112,
        ..removed_chest
    };
    let isolated_brewing = BlockPos {
        x: 128,
        ..removed_chest
    };
    cells.extend([
        (removed_chest, "chest", 27, false),
        (removed_furnace, "furnace", 3, false),
        (retained_loot, "chest", 27, false),
        (cleared_loot, "chest", 27, true),
        (isolated_hopper, "hopper", 5, false),
        (isolated_brewing, "brewing_stand", 5, false),
    ]);
    let positions = cells
        .iter()
        .map(|(pos, _, _, _)| {
            (
                ChunkPos {
                    x: pos.x.div_euclid(16),
                    z: pos.z.div_euclid(16),
                },
                0,
            )
        })
        .collect();
    world.ensure_blocking(&positions);
    let diamond = registry
        .items
        .id_of(&Identifier::parse("minecraft:diamond").unwrap())
        .unwrap();
    for &(pos, kind, _, _) in &cells {
        let state = registry
            .blocks
            .block(&Identifier::parse(format!("minecraft:{kind}")).unwrap())
            .unwrap()
            .default
            .0;
        world.set_block(pos.x, pos.y, pos.z, state);
    }
    world.fold_overlay();
    let mut records = crate::core::CheckpointBlockEntities::new();
    let stale = FurnaceSlot {
        item_id: diamond,
        count: 63,
        custom_name: Some("old native slot".into()),
        ..FurnaceSlot::EMPTY
    };
    for (index, &(pos, kind, size, live)) in cells.iter().enumerate() {
        let chunk_pos = ChunkPos {
            x: pos.x.div_euclid(16),
            z: pos.z.div_euclid(16),
        };
        let chunk = Arc::make_mut(world.chunks.get_mut(&chunk_pos).unwrap());
        match kind {
            "chest" | "trapped_chest" | "barrel" => {
                chunk.chests.insert(
                    pos,
                    ChestBlockEntity {
                        slots: std::array::from_fn(|_| stale.clone()),
                        loot_table: (pos == retained_loot || pos == cleared_loot).then(|| {
                            Identifier::parse("minecraft:chests/abandoned_mineshaft").unwrap()
                        }),
                        loot_seed: 123,
                    },
                );
            }
            "furnace" | "smoker" | "blast_furnace" => {
                chunk.furnaces.insert(
                    pos,
                    FurnaceBlockEntity {
                        slots: std::array::from_fn(|_| stale.clone()),
                        burn_remaining: 7,
                        burn_total: 8,
                        cook_progress: 9,
                        cook_total: 10,
                        recipes_used: [("minecraft:old_recipe".into(), 11)].into(),
                    },
                );
            }
            "hopper" => {
                chunk.hoppers.insert(
                    pos,
                    HopperBlockEntity {
                        slots: std::array::from_fn(|_| stale.clone()),
                        transfer_cooldown: 17,
                    },
                );
            }
            "brewing_stand" => {
                chunk.brewing_stands.insert(
                    pos,
                    BrewingStandBlockEntity {
                        slots: std::array::from_fn(|_| stale.clone()),
                        brew_time: 22,
                        fuel: 9,
                        ingredient: Some(diamond),
                    },
                );
            }
            _ => {}
        }
        let mut slots = vec![mc_data::ItemStack::EMPTY; size];
        if index % 7 != 0 && pos != cleared_loot {
            slots[0] = mc_data::ItemStack::new(diamond, (index % 17 + 1) as i32);
            slots[0].damage = Some(4);
            slots[0].custom_name = Some(format!("custody {index}").into());
        }
        if matches!(kind, "hopper" | "brewing_stand" | "oak_shelf" | "crafter") {
            let mut fields = vec![
                (
                    "id".into(),
                    mc_nbt::Tag::String(format!("minecraft:{kind}")),
                ),
                ("x".into(), mc_nbt::Tag::Int(pos.x)),
                ("y".into(), mc_nbt::Tag::Int(pos.y)),
                ("z".into(), mc_nbt::Tag::Int(pos.z)),
            ];
            mc_data::storage_blocks::encode_items(&mut fields, &slots, &registry.items);
            let mut bytes = Vec::new();
            mc_nbt::write_network(&mut bytes, &mc_nbt::Tag::Compound(fields)).unwrap();
            chunk.block_entities.insert(pos, bytes);
        }
        if live {
            records.insert(
                (pos.x, pos.y, pos.z),
                crate::journal::ReplayBlockEntity {
                    slots,
                    furnace: matches!(kind, "furnace" | "smoker" | "blast_furnace").then(|| {
                        crate::journal::FurnaceImage {
                            lit_remaining: 13,
                            lit_total: 37,
                            cook_progress: 41,
                            cook_total: 201,
                            recipes_used: Arc::new([("minecraft:iron_ingot".into(), 5)].into()),
                        }
                    }),
                    tick: 3,
                },
            );
        }
    }
    // None of the live owners is separately dirty. Ordinary block dirtiness
    // still needs all clean owners sharing each complete native chunk image.
    assert!(world.block_entity_dirty.is_empty());
    let mut expected = world
        .chunks
        .iter()
        .map(|(&pos, chunk)| (pos, (**chunk).clone()))
        .collect::<HashMap<_, _>>();
    for (&pos, chunk) in &mut expected {
        if !chunk.chests.is_empty()
            || !chunk.furnaces.is_empty()
            || records
                .keys()
                .any(|&(x, y, z)| ChunkWorld::in_chunk(BlockPos { x, y, z }, pos))
        {
            project_using_original_full_map(chunk, pos, &records, &registry.blocks);
        }
    }
    let capture = world.capture_dirty(&records);
    assert!(capture.refused().is_empty());
    assert_eq!(capture.chunks.len(), expected.len());
    for captured in &capture.chunks {
        let actual = &captured.chunk;
        let expected = &expected[&captured.pos];
        assert_eq!(actual.chests, expected.chests, "chests {:?}", captured.pos);
        assert_eq!(
            actual.furnaces, expected.furnaces,
            "furnaces {:?}",
            captured.pos
        );
        assert_eq!(
            actual.hoppers, expected.hoppers,
            "hoppers {:?}",
            captured.pos
        );
        assert_eq!(
            actual.brewing_stands, expected.brewing_stands,
            "brewing {:?}",
            captured.pos
        );
        assert_eq!(
            actual.block_entities, expected.block_entities,
            "opaque bytes {:?}",
            captured.pos
        );
    }
    let chunk_at = |block: BlockPos| {
        &expected[&ChunkPos {
            x: block.x.div_euclid(16),
            z: block.z.div_euclid(16),
        }]
    };
    assert!(!chunk_at(removed_chest).chests.contains_key(&removed_chest));
    assert!(
        !chunk_at(removed_furnace)
            .furnaces
            .contains_key(&removed_furnace)
    );
    assert!(
        chunk_at(retained_loot).chests[&retained_loot]
            .loot_table
            .is_some()
    );
    assert!(
        chunk_at(cleared_loot).chests[&cleared_loot]
            .loot_table
            .is_none()
    );
    assert!(
        chunk_at(cleared_loot).chests[&cleared_loot]
            .slots
            .iter()
            .all(FurnaceSlot::is_empty)
    );
    assert!(
        chunk_at(isolated_hopper)
            .hoppers
            .contains_key(&isolated_hopper)
    );
    assert!(
        chunk_at(isolated_brewing)
            .brewing_stands
            .contains_key(&isolated_brewing)
    );
}

#[test]
fn required_generated_capacity_refuses_admission_without_losing_the_next_baseline() {
    let (_directory, _, mut world) = fixture();
    for x in 0..GENERATED_PENDING_MAX as i32 {
        let pos = ChunkPos { x, z: 0 };
        let chunk = world.source.generator.generate(pos);
        assert!(world.insert_produced(pos, chunk, true));
    }
    let overflow = ChunkPos {
        x: GENERATED_PENDING_MAX as i32,
        z: 0,
    };
    let chunk = world.source.generator.generate(overflow);
    assert!(
        !world.insert_produced(overflow, chunk, true),
        "generated admission must refuse rather than silently drop its required baseline"
    );
    assert!(world.chunk(overflow).is_none());
    assert_eq!(
        world.take_generated(0).unwrap().chunks.len(),
        GENERATED_PENDING_MAX
    );
}

#[test]
fn required_generated_capacity_holds_checkpoint_and_unmaterialized_wal_edits() {
    let (_directory, registry, mut world) = fixture();
    for x in 0..GENERATED_PENDING_MAX as i32 {
        let pos = ChunkPos { x, z: 0 };
        let chunk = world.source.generator.generate(pos);
        assert!(world.insert_produced(pos, chunk, true));
    }
    let x = GENERATED_PENDING_MAX as i32 * 16;
    world.set_block_catch_up(x, 100, 0, registry.states.stone);
    let before = world
        .source
        .native
        .as_ref()
        .unwrap()
        .read_checkpoint()
        .unwrap()
        .tick;
    let capture = world.capture_dirty(&Default::default());
    assert!(
        capture.write().is_err(),
        "checkpoint must not commit/prune edits whose generated baseline could not enter custody"
    );
    assert_eq!(
        world
            .source
            .native
            .as_ref()
            .unwrap()
            .read_checkpoint()
            .unwrap()
            .tick,
        before
    );
    assert!(
        world.overlay.contains_key(&(x, 100, 0)),
        "retain unmaterialized replay edits"
    );
}
