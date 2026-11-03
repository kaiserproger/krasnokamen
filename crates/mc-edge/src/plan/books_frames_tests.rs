use super::*;
#[test]
fn frame_spawn_uses_attached_cell_instead_of_render_center() {
    for name in ["minecraft:item_frame", "minecraft:glow_item_frame"] {
        let mut frame = entity(7, [-1.96875, 65.5, 8.5]);
        frame.type_id = mc_data::entity_types::entity_type_contract_26_1_2_by_name(name)
            .unwrap()
            .protocol_id;
        assert_eq!(entity_spawn_position(&frame), [-2.0, 65.0, 8.0]);
    }
}
#[test]
fn signed_book_open_is_reliable_hand_packet() {
    let mut viewer = viewer_at(1, [8.0, 64.0, 8.0]);
    let packets = plan_player_changes(&[PlayerChange::BookOpened { hand: Hand::Off }], &mut viewer);
    assert_eq!(packets.len(), 1);
    assert!(packets[0].reliable);
    assert_eq!(
        packets[0].id,
        mc_protocol::packets::play::ClientboundOpenBook::ID
    );
    assert_eq!(body_of(&packets[0]).as_ref(), &[1]);
}

#[test]
fn chest_direct_packets_hide_items_and_preserve_visible_payloads() {
    use mc_data::block_entity_kinds::BlockEntityKind;
    use mc_nbt::{ListTag, Tag, tag_type};
    use mc_protocol::packets::{Packet, play::ClientboundBlockEntityData};

    let input = Tag::Compound(vec![(
        "Items".into(),
        Tag::List(ListTag {
            element_type: tag_type::COMPOUND,
            elements: vec![Tag::Compound(vec![
                ("id".into(), Tag::String("minecraft:diamond".into())),
                ("count".into(), Tag::Int(2)),
            ])],
        }),
    )]);
    for kind in [
        BlockEntityKind::Chest,
        BlockEntityKind::TrappedChest,
        BlockEntityKind::Barrel,
        BlockEntityKind::Campfire,
    ] {
        let mut viewer = viewer_at(1, [8.0, 64.0, 8.0]);
        viewer.loaded_chunks.insert((0, 0), 0);
        let packets = plan_player_changes(
            &[PlayerChange::BlockEntityData {
                position: mc_world::chunk::BlockPos { x: 1, y: 64, z: 2 },
                block_entity_type: kind.protocol_id(),
                nbt: input.clone(),
            }],
            &mut viewer,
        );
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].id, ClientboundBlockEntityData::ID);
        let packet = ClientboundBlockEntityData::decode(&mut body_of(&packets[0])).unwrap();
        assert_eq!(packet.block_entity_type, kind.protocol_id() as i32);
        let expected = if kind == BlockEntityKind::Campfire {
            input.clone()
        } else {
            Tag::Compound(Vec::new())
        };
        assert_eq!(packet.nbt, expected, "{kind:?}");
    }
}

#[test]
fn furnace_direct_packets_hide_items_and_keep_timers() {
    use mc_data::block_entity_kinds::BlockEntityKind;
    use mc_nbt::{ListTag, Tag, tag_type};
    use mc_protocol::packets::{Packet, play::ClientboundBlockEntityData};

    let items = Tag::List(ListTag {
        element_type: tag_type::COMPOUND,
        elements: vec![Tag::Compound(vec![(
            "id".into(),
            Tag::String("minecraft:diamond".into()),
        )])],
    });
    let input = Tag::Compound(vec![
        ("Items".into(), items.clone()),
        ("lit_time_remaining".into(), Tag::Short(71)),
        ("cooking_time_spent".into(), Tag::Short(12)),
    ]);
    for kind in [
        BlockEntityKind::Furnace,
        BlockEntityKind::Smoker,
        BlockEntityKind::BlastFurnace,
        BlockEntityKind::Campfire,
    ] {
        let mut viewer = viewer_at(1, [8.0, 64.0, 8.0]);
        viewer.loaded_chunks.insert((0, 0), 0);
        let packets = plan_player_changes(
            &[PlayerChange::BlockEntityData {
                position: mc_world::chunk::BlockPos { x: 1, y: 64, z: 2 },
                block_entity_type: kind.protocol_id(),
                nbt: input.clone(),
            }],
            &mut viewer,
        );
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].id, ClientboundBlockEntityData::ID);
        let packet = ClientboundBlockEntityData::decode(&mut body_of(&packets[0])).unwrap();
        assert_eq!(packet.block_entity_type, kind.protocol_id() as i32);
        let expected = if kind == BlockEntityKind::Campfire {
            input.clone()
        } else {
            Tag::Compound(vec![
                ("lit_time_remaining".into(), Tag::Short(71)),
                ("cooking_time_spent".into(), Tag::Short(12)),
            ])
        };
        assert_eq!(packet.nbt, expected, "{kind:?}");
    }
}

#[test]
fn decorated_pot_direct_packet_hides_contents_but_preserves_sherds() {
    use mc_data::block_entity_kinds::BlockEntityKind;
    use mc_nbt::Tag;
    use mc_protocol::packets::{Packet, play::ClientboundBlockEntityData};

    let sherds = Tag::Compound(vec![(
        "back".into(),
        Tag::Compound(vec![
            (
                "id".into(),
                Tag::String("minecraft:angler_pottery_sherd".into()),
            ),
            (
                "components".into(),
                Tag::Compound(vec![(
                    "minecraft:custom_data".into(),
                    Tag::Compound(vec![(
                        "item".into(),
                        Tag::String("template metadata".into()),
                    )]),
                )]),
            ),
        ]),
    )]);
    let visible = Tag::Compound(vec![
        ("sherds".into(), sherds.clone()),
        ("custom".into(), Tag::Int(7)),
    ]);
    let input = Tag::Compound(vec![
        (
            "item".into(),
            Tag::Compound(vec![("id".into(), Tag::String("minecraft:diamond".into()))]),
        ),
        (
            "LootTable".into(),
            Tag::String("minecraft:chests/test".into()),
        ),
        ("LootTableSeed".into(), Tag::Long(42)),
        ("sherds".into(), sherds),
        ("custom".into(), Tag::Int(7)),
    ]);
    let mut viewer = viewer_at(1, [8.0, 64.0, 8.0]);
    viewer.loaded_chunks.insert((0, 0), 0);
    let packets = plan_player_changes(
        &[PlayerChange::BlockEntityData {
            position: mc_world::chunk::BlockPos { x: 1, y: 64, z: 2 },
            block_entity_type: BlockEntityKind::DecoratedPot.protocol_id(),
            nbt: input,
        }],
        &mut viewer,
    );
    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].id, ClientboundBlockEntityData::ID);
    let packet = ClientboundBlockEntityData::decode(&mut body_of(&packets[0])).unwrap();
    assert_eq!(
        packet.block_entity_type,
        BlockEntityKind::DecoratedPot.protocol_id() as i32
    );
    assert_eq!(packet.nbt, visible);
}
