use super::*;

#[test]
fn container_wire_nonempty_chest_must_not_disclose_items() {
    use mc_protocol::frame::{Compression, encode_frame, try_decode_frame};
    use mc_protocol::packets::play::LevelChunkWithLight;
    use mc_world::chunk::{BlockPos, ChestBlockEntity, FurnaceSlot};

    let (biomes, blocks) = reports();
    let items = items();
    let types = be_types();
    let name = Identifier::parse("minecraft:chest").unwrap();
    let chest_state = blocks.block(&name).unwrap().default;
    let air = blocks
        .block(&Identifier::parse("minecraft:air").unwrap())
        .unwrap()
        .default;
    let mut chunk = Chunk::empty(
        ChunkPos { x: 0, z: 0 },
        air,
        Identifier::parse("minecraft:plains").unwrap(),
    );
    let pos = BlockPos { x: 2, y: 64, z: 3 };
    chunk.set_block(2, 64, 3, chest_state).unwrap();
    let mut chest = ChestBlockEntity::default();
    chest.slots[0] = FurnaceSlot {
        item_id: 1,
        count: 2,
        damage: None,
        enchantments: Vec::new(),
        custom_name: None,
        item_model: None,
        stew_effects: Vec::new(),
        components: None,
    };
    chunk.chests.insert(pos, chest);
    let saved = mc_world::anvil::chunk_nbt::block_entity_to_tag(&chunk, pos, &blocks, &items)
        .unwrap()
        .unwrap();
    let mut saved_bytes = Vec::new();
    mc_nbt::write_network(&mut saved_bytes, &saved).unwrap();
    assert!(
        saved_bytes
            .windows(b"minecraft:stone".len())
            .any(|w| w == b"minecraft:stone")
    );

    let packet = encode_chunk_packet(
        chunk.pos,
        &chunk,
        EncodeInputs {
            biomes: &biomes,
            blocks: &blocks,
            block_entity_types: &types,
        },
    )
    .unwrap();
    let mut body = Vec::new();
    packet.encode(&mut body).unwrap();
    let framed = encode_frame(LevelChunkWithLight::ID, &body, Compression::Threshold(256)).unwrap();
    let mut framed = bytes::BytesMut::from(&framed[..]);
    let frame = try_decode_frame(&mut framed, Compression::Threshold(256))
        .unwrap()
        .unwrap();
    assert_eq!(frame.id, LevelChunkWithLight::ID);
    let decoded = LevelChunkWithLight::decode(&mut frame.body.clone()).unwrap();
    let record = decoded
        .block_entities
        .iter()
        .find(|record| record.packed_xz == 0x23 && record.y == 64)
        .expect("chest block entity must remain in the packet");
    assert_eq!(record.type_id, types.id_of(&name).unwrap() as i32);
    assert_eq!(
        record.nbt,
        mc_nbt::Tag::Compound(Vec::new()),
        "chest update tag must not expose stored Items"
    );
}

#[test]
fn opaque_container_types_do_not_disclose_items_in_encoded_chunks() {
    use mc_nbt::{ListTag, Tag, tag_type};
    use mc_protocol::packets::play::LevelChunkWithLight;
    use mc_world::chunk::BlockPos;

    let (biomes, blocks) = reports();
    let types = be_types();
    let air = blocks
        .block(&Identifier::parse("minecraft:air").unwrap())
        .unwrap()
        .default;
    let mut chunk = Chunk::empty(
        ChunkPos { x: 0, z: 0 },
        air,
        Identifier::parse("minecraft:plains").unwrap(),
    );
    let contents = Tag::List(ListTag {
        element_type: tag_type::COMPOUND,
        elements: vec![Tag::Compound(vec![
            ("id".into(), Tag::String("minecraft:diamond".into())),
            ("count".into(), Tag::Int(2)),
        ])],
    });
    for (x, kind) in [
        (1, "chest"),
        (2, "trapped_chest"),
        (3, "barrel"),
        (4, "hopper"),
        (5, "dispenser"),
        (6, "dropper"),
    ] {
        let tag = Tag::Compound(vec![
            ("id".into(), Tag::String(format!("minecraft:{kind}"))),
            ("Items".into(), contents.clone()),
        ]);
        let mut encoded = Vec::new();
        mc_nbt::write_network(&mut encoded, &tag).unwrap();
        chunk
            .block_entities
            .insert(BlockPos { x, y: 64, z: 0 }, encoded);
    }
    let packet = encode_chunk_packet(
        chunk.pos,
        &chunk,
        EncodeInputs {
            biomes: &biomes,
            blocks: &blocks,
            block_entity_types: &types,
        },
    )
    .unwrap();
    let mut body = Vec::new();
    packet.encode(&mut body).unwrap();
    let decoded = LevelChunkWithLight::decode(&mut bytes::Bytes::from(body)).unwrap();
    for (x, kind) in [
        (1, "chest"),
        (2, "trapped_chest"),
        (3, "barrel"),
        (4, "hopper"),
        (5, "dispenser"),
        (6, "dropper"),
    ] {
        let record = decoded
            .block_entities
            .iter()
            .find(|record| record.packed_xz == x << 4 && record.y == 64)
            .expect("block entity must remain in chunk");
        assert_eq!(
            record.type_id,
            types
                .id_of(&Identifier::parse(format!("minecraft:{kind}")).unwrap())
                .unwrap() as i32
        );
        assert_eq!(record.nbt, Tag::Compound(Vec::new()), "{kind}");
    }
}

#[test]
fn furnace_chunk_packets_hide_items_and_timers_but_keep_persistence() {
    use mc_nbt::{ListTag, Tag, tag_type};
    use mc_protocol::frame::{Compression, encode_frame, try_decode_frame};
    use mc_protocol::packets::play::LevelChunkWithLight;
    use mc_world::chunk::{BlockPos, FurnaceBlockEntity, FurnaceSlot};

    let (biomes, blocks) = reports();
    let items = items();
    let types = be_types();
    let air = blocks
        .block(&Identifier::parse("minecraft:air").unwrap())
        .unwrap()
        .default;
    let mut chunk = Chunk::empty(
        ChunkPos { x: 0, z: 0 },
        air,
        Identifier::parse("minecraft:plains").unwrap(),
    );
    for (x, kind) in [(1, "furnace"), (2, "smoker"), (3, "blast_furnace")] {
        let name = Identifier::parse(format!("minecraft:{kind}")).unwrap();
        chunk
            .set_block(x as u8, 64, 0, blocks.block(&name).unwrap().default)
            .unwrap();
        let mut furnace = FurnaceBlockEntity::default();
        furnace.slots[0] = FurnaceSlot {
            item_id: 1,
            count: 2,
            damage: None,
            enchantments: Vec::new(),
            custom_name: None,
            item_model: None,
            stew_effects: Vec::new(),
            components: None,
        };
        furnace.burn_remaining = 71;
        furnace.burn_total = 100;
        furnace.cook_progress = 12;
        furnace.cook_total = 200;
        let pos = BlockPos { x, y: 64, z: 0 };
        chunk.furnaces.insert(pos, furnace);
        let saved = mc_world::anvil::chunk_nbt::block_entity_to_tag(&chunk, pos, &blocks, &items)
            .unwrap()
            .unwrap();
        let mut saved_bytes = Vec::new();
        mc_nbt::write_network(&mut saved_bytes, &saved).unwrap();
        assert!(
            saved_bytes
                .windows(b"minecraft:stone".len())
                .any(|w| w == b"minecraft:stone")
        );
    }
    for (x, kind) in [(4, "furnace"), (5, "smoker"), (6, "blast_furnace")] {
        let tag = Tag::Compound(vec![
            ("id".into(), Tag::String(format!("minecraft:{kind}"))),
            (
                "Items".into(),
                Tag::List(ListTag {
                    element_type: tag_type::COMPOUND,
                    elements: vec![Tag::Compound(vec![(
                        "id".into(),
                        Tag::String("minecraft:diamond".into()),
                    )])],
                }),
            ),
            ("lit_time_remaining".into(), Tag::Short(71)),
        ]);
        let mut bytes = Vec::new();
        mc_nbt::write_network(&mut bytes, &tag).unwrap();
        chunk
            .block_entities
            .insert(BlockPos { x, y: 64, z: 0 }, bytes);
    }
    let packet = encode_chunk_packet(
        chunk.pos,
        &chunk,
        EncodeInputs {
            biomes: &biomes,
            blocks: &blocks,
            block_entity_types: &types,
        },
    )
    .unwrap();
    let mut body = Vec::new();
    packet.encode(&mut body).unwrap();
    let frame = encode_frame(LevelChunkWithLight::ID, &body, Compression::Threshold(256)).unwrap();
    let mut frame = bytes::BytesMut::from(&frame[..]);
    let frame = try_decode_frame(&mut frame, Compression::Threshold(256))
        .unwrap()
        .unwrap();
    assert_eq!(frame.id, LevelChunkWithLight::ID);
    let decoded = LevelChunkWithLight::decode(&mut frame.body.clone()).unwrap();
    for (x, kind) in [
        (1, "furnace"),
        (2, "smoker"),
        (3, "blast_furnace"),
        (4, "furnace"),
        (5, "smoker"),
        (6, "blast_furnace"),
    ] {
        let pos = BlockPos { x, y: 64, z: 0 };
        assert_eq!(
            mc_world::wire::client_block_entity(&chunk, &blocks, pos)
                .unwrap()
                .nbt,
            Tag::Compound(Vec::new()),
            "{kind} at {x}"
        );
        let record = decoded
            .block_entities
            .iter()
            .find(|record| record.packed_xz == (x as u8) << 4 && record.y == 64)
            .unwrap();
        assert_eq!(
            record.type_id,
            types
                .id_of(&Identifier::parse(format!("minecraft:{kind}")).unwrap())
                .unwrap() as i32
        );
        let Tag::Compound(fields) = &record.nbt else {
            panic!("compound update");
        };
        assert!(fields.is_empty(), "{kind} at {x}");
    }
}

#[test]
fn opaque_inventory_chunk_packets_hide_saved_fields_and_preserve_persistence() {
    use mc_nbt::{ListTag, Tag, tag_type};
    use mc_protocol::packets::play::LevelChunkWithLight;
    use mc_world::chunk::BlockPos;

    let (biomes, blocks) = reports();
    let types = be_types();
    let air = blocks
        .block(&Identifier::parse("minecraft:air").unwrap())
        .unwrap()
        .default;
    let mut chunk = Chunk::empty(
        ChunkPos { x: 0, z: 0 },
        air,
        Identifier::parse("minecraft:plains").unwrap(),
    );
    let fixtures = [
        (4, "lectern"),
        (1, "brewing_stand"),
        (2, "shulker_box"),
        (3, "crafter"),
        (5, "chiseled_bookshelf"),
    ];
    let mut stored = Vec::new();
    for (x, kind) in &fixtures {
        let item = Tag::Compound(vec![
            (
                "id".into(),
                Tag::String(
                    if *kind == "lectern" {
                        "minecraft:writable_book"
                    } else {
                        "minecraft:diamond"
                    }
                    .into(),
                ),
            ),
            ("count".into(), Tag::Int(1)),
        ]);
        let secret = if *kind == "lectern" {
            ("Book".into(), item)
        } else {
            (
                "Items".into(),
                Tag::List(ListTag {
                    element_type: tag_type::COMPOUND,
                    elements: vec![item],
                }),
            )
        };
        let fields = vec![
            ("id".into(), Tag::String(format!("minecraft:{kind}"))),
            secret,
        ];
        let mut bytes = Vec::new();
        mc_nbt::write_network(&mut bytes, &Tag::Compound(fields)).unwrap();
        let pos = BlockPos { x: *x, y: 64, z: 0 };
        chunk.block_entities.insert(pos, bytes.clone());
        stored.push((pos, bytes));
    }
    let packet = encode_chunk_packet(
        chunk.pos,
        &chunk,
        EncodeInputs {
            biomes: &biomes,
            blocks: &blocks,
            block_entity_types: &types,
        },
    )
    .unwrap();
    let mut body = Vec::new();
    packet.encode(&mut body).unwrap();
    let decoded = LevelChunkWithLight::decode(&mut bytes::Bytes::from(body)).unwrap();
    for (x, kind) in fixtures {
        let record = decoded
            .block_entities
            .iter()
            .find(|record| record.packed_xz == (x as u8) << 4 && record.y == 64)
            .expect("block entity must remain in chunk");
        assert_eq!(
            record.type_id,
            types
                .id_of(&Identifier::parse(format!("minecraft:{kind}")).unwrap())
                .unwrap() as i32
        );
        assert_eq!(record.nbt, Tag::Compound(Vec::new()), "{kind}");
    }
    for (pos, bytes) in stored {
        assert_eq!(
            chunk.block_entities.get(&pos),
            Some(&bytes),
            "persistent contents changed"
        );
    }
}

#[test]
fn brushable_chunk_update_hides_loot_and_preserves_rendered_item() {
    use mc_nbt::Tag;
    use mc_protocol::packets::play::LevelChunkWithLight;
    use mc_world::chunk::BlockPos;

    let (biomes, blocks) = reports();
    let types = be_types();
    let air = blocks
        .block(&Identifier::parse("minecraft:air").unwrap())
        .unwrap()
        .default;
    let mut chunk = Chunk::empty(
        ChunkPos { x: 0, z: 0 },
        air,
        Identifier::parse("minecraft:plains").unwrap(),
    );
    let sand = Identifier::parse("minecraft:suspicious_sand").unwrap();
    let dusted = blocks
        .by_name_and_props(&sand, &[("dusted".into(), "1".into())])
        .unwrap();
    chunk.set_block(2, 64, 0, dusted).unwrap();
    chunk
        .set_block(3, 64, 0, blocks.block(&sand).unwrap().default)
        .unwrap();
    let item = Tag::Compound(vec![
        ("id".into(), Tag::String("minecraft:diamond".into())),
        ("count".into(), Tag::Int(1)),
        (
            "components".into(),
            Tag::Compound(vec![(
                "minecraft:custom_data".into(),
                Tag::Compound(vec![("marker".into(), Tag::String("visible".into()))]),
            )]),
        ),
    ]);
    let entries = [
        (
            1,
            vec![
                (
                    "LootTable".into(),
                    Tag::String("minecraft:archaeology/desert_well".into()),
                ),
                ("LootTableSeed".into(), Tag::Long(42)),
            ],
        ),
        (
            2,
            vec![
                ("item".into(), item.clone()),
                ("hit_direction".into(), Tag::Int(3)),
            ],
        ),
        (
            3,
            vec![
                ("item".into(), item.clone()),
                ("hit_direction".into(), Tag::Int(3)),
            ],
        ),
    ];
    for (x, fields) in entries {
        let mut fields = fields;
        fields.push(("id".into(), Tag::String("minecraft:brushable_block".into())));
        let mut data = Vec::new();
        mc_nbt::write_network(&mut data, &Tag::Compound(fields)).unwrap();
        chunk
            .block_entities
            .insert(BlockPos { x, y: 64, z: 0 }, data);
    }
    let saved = chunk.block_entities.clone();
    let packet = encode_chunk_packet(
        chunk.pos,
        &chunk,
        EncodeInputs {
            biomes: &biomes,
            blocks: &blocks,
            block_entity_types: &types,
        },
    )
    .unwrap();
    let mut body = Vec::new();
    packet.encode(&mut body).unwrap();
    let decoded = LevelChunkWithLight::decode(&mut bytes::Bytes::from(body)).unwrap();
    let entity = |xz| {
        decoded
            .block_entities
            .iter()
            .find(|record| record.packed_xz == xz && record.y == 64)
            .expect("brushable record")
    };
    assert_eq!(entity(0x10).type_id, 40, "26.3 brushable block entity id");
    assert_eq!(entity(0x20).type_id, 40, "26.3 brushable block entity id");
    assert_eq!(
        entity(0x10).nbt,
        Tag::Compound(vec![]),
        "unopened loot is private"
    );
    assert_eq!(
        entity(0x30).nbt,
        Tag::Compound(vec![]),
        "unrevealed item is private"
    );
    assert_eq!(
        entity(0x20).nbt,
        Tag::Compound(vec![
            ("item".into(), item),
            ("hit_direction".into(), Tag::Int(3))
        ]),
        "visible item components and orientation survive"
    );
    assert_eq!(
        chunk.block_entities, saved,
        "projection must not alter stored loot"
    );
}
