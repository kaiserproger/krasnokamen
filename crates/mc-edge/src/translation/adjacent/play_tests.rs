//! Byte layouts of the translated Play packets (26.3 -> 26.1.2 and back).

use mc_protocol::codec::{ReadMc, WriteMc};

use super::super::tables::{IdMap, Tables};
use super::super::test_session;
use super::*;

fn session() -> Session {
    test_session(Tables {
        block_states: IdMap::by_name([("a", 0), ("b", 9)], [("a", 0), ("b", 4)], |_| None),
        items: IdMap::by_name([("stick", 10)], [("stick", 3)], |_| None),
        registry_ids: [(
            "dimension_type".into(),
            IdMap::by_name([("overworld", 0)], [("overworld", 0)], |_| None),
        )]
        .into(),
        ..Tables::default()
    })
}

#[test]
fn level_event_consumes_global_and_remaps_block_state() {
    let mut s = session();
    for (event, data, expected_data) in [(2002, 9, 9), (2001, 9, 4)] {
        for global in [false, true] {
            let mut body = Vec::new();
            body.write_i32(event);
            body.write_i64(123);
            body.write_i32(data);
            body.write_bool(global);
            let out = clientbound(&mut s, "minecraft:level_event", &body)
                .unwrap()
                .unwrap();
            let mut r = out.as_slice();
            assert_eq!(r.read_i32().unwrap(), event);
            assert_eq!(r.read_i64().unwrap(), 123);
            assert_eq!(r.read_i32().unwrap(), expected_data);
            assert_eq!(r.read_bool().unwrap(), global);
            assert!(r.is_empty());
        }
    }
}

#[test]
fn settlement_text_display_maps_to_legacy_type_without_changing_its_accessors() {
    let mut s = test_session(Tables {
        entities: IdMap::by_name(
            [("minecraft:text_display", 135)],
            [("minecraft:text_display", 131)],
            |_| None,
        ),
        ..Tables::default()
    });
    let mut body = Vec::new();
    body.write_varint(-1);
    body.extend([0; 16]); // UUID
    body.write_varint(135); // 26.3 text_display, not stray131
    for value in [8.5, 66.6, 8.5] {
        body.write_f64(value);
    }
    body.extend([0; 5]); // zero LP velocity, three rotations, spawn data
    let legacy = clientbound(&mut s, "minecraft:add_entity", &body)
        .unwrap()
        .unwrap();
    let mut reader = &legacy[..];
    assert_eq!(reader.read_varint().unwrap(), -1);
    reader.read_uuid().unwrap();
    assert_eq!(reader.read_varint().unwrap(), 131);
    assert!(
        !s.entities.contains_key(&-1),
        "display metadata layout is unchanged"
    );

    let mut data = Vec::new();
    data.write_varint(-1);
    data.extend([17, 3]); // Display view range: FLOAT17 in both versions
    data.write_f32(1.0);
    data.extend([23, 5, 8, 0, 5]); // TextDisplay COMPONENT23, network NBT string
    data.extend(b"House");
    data.push(255);
    assert_eq!(
        clientbound(&mut s, "minecraft:set_entity_data", &data)
            .unwrap()
            .unwrap(),
        data
    );
}

#[test]
fn light_masks_expand_native_bytes_into_legacy_longs() {
    let native = [
        2, 0, 2, // sky: bit 9
        8, 0, 0, 0, 0, 0, 0, 0, 0x80, // block: bit 63
        9, 0, 0, 0, 0, 0, 0, 0, 0, 1, // empty sky: bit 64
        0, // empty block
        0, 0, // update lists
    ];
    let mut legacy = Vec::new();
    for mask in [&[1 << 9][..], &[i64::MIN][..], &[0, 1][..], &[][..]] {
        legacy.write_varint(mask.len() as i32);
        for word in mask {
            legacy.write_i64(*word);
        }
    }
    legacy.extend([0, 0]);
    let mut s = session();
    for (name, mut prefix) in [
        ("minecraft:light_update", vec![0, 0]),
        ("minecraft:level_chunk_with_light", vec![0; 11]),
    ] {
        let mut body = prefix.clone();
        body.extend(native);
        prefix.extend_from_slice(&legacy);
        assert_eq!(clientbound(&mut s, name, &body).unwrap().unwrap(), prefix);
    }
}

#[test]
fn login_drops_online_mode_and_narrows_game_modes() {
    let mut s = session();
    let mut body = Vec::new();
    body.write_i32(7);
    body.write_bool(false);
    body.write_varint(1);
    body.write_string("minecraft:overworld", 32767).unwrap();
    for v in [20, 10, 6] {
        body.write_varint(v);
    }
    for b in [false, true, false] {
        body.write_bool(b);
    }
    let spawn_start = body.len();
    body.write_varint(0); // dimension type holder
    body.write_string("minecraft:overworld", 32767).unwrap();
    body.write_i64(42);
    body.write_varint(1); // creative
    body.write_varint(0); // no previous mode
    body.write_bool(false);
    body.write_bool(false);
    body.write_bool(false); // no death location
    body.write_varint(0);
    body.write_varint(63);
    let spawn_end = body.len();
    body.write_bool(true); // onlineMode (introduced in 26.2)
    body.write_bool(false); // enforcesSecureChat
    let out = clientbound(&mut s, "minecraft:login", &body)
        .unwrap()
        .unwrap();
    let mut expected = body[..spawn_start].to_vec();
    expected.write_varint(0);
    expected.write_string("minecraft:overworld", 32767).unwrap();
    expected.write_i64(42);
    expected.write_u8(1);
    expected.write_i8(-1);
    expected.extend_from_slice(&body[spawn_end - 5..spawn_end]);
    expected.write_bool(false);
    assert_eq!(out, expected);
    let mut upper = session();
    std::sync::Arc::get_mut(&mut upper.profile)
        .unwrap()
        .tables
        .formats = super::super::tables::Formats {
        upper: 777,
        lower: 776,
    };
    let intermediate = clientbound(&mut upper, "minecraft:login", &body)
        .unwrap()
        .unwrap();
    assert_eq!(
        intermediate.len(),
        expected.len() + 1,
        "26.2 retains onlineMode"
    );
    let mut lower = session();
    std::sync::Arc::get_mut(&mut lower.profile)
        .unwrap()
        .tables
        .formats = super::super::tables::Formats {
        upper: 776,
        lower: 775,
    };
    assert_eq!(
        clientbound(&mut lower, "minecraft:login", &intermediate)
            .unwrap()
            .unwrap(),
        expected
    );
}

#[test]
fn stepped_entity_moves_collapse_into_one_delta() {
    let mut s = session();
    let mut body = Vec::new();
    body.write_varint(5);
    body.write_varint(1 | (2 << 1)); // on ground, two steps
    for (ticks, delta) in [(1, [10i16, -2, 3]), (2, [5, 0, -3])] {
        body.write_varint(ticks);
        for d in delta {
            body.write_i16(d);
        }
    }
    body.write_u8(64);
    body.write_u8(32);
    let out = clientbound(&mut s, "minecraft:move_entity_pos_rot", &body)
        .unwrap()
        .unwrap();
    let mut r = &out[..];
    assert_eq!(r.read_varint().unwrap(), 5);
    assert_eq!(
        [
            r.read_i16().unwrap(),
            r.read_i16().unwrap(),
            r.read_i16().unwrap()
        ],
        [15, -2, 0]
    );
    assert_eq!([r.read_u8().unwrap(), r.read_u8().unwrap()], [64, 32]);
    assert!(r.read_bool().unwrap());
    assert!(r.is_empty());
}

#[test]
fn block_updates_and_slots_use_legacy_ids() {
    let mut s = session();
    let mut body = Vec::new();
    body.write_i64(123);
    body.write_varint(9);
    let out = clientbound(&mut s, "minecraft:block_update", &body)
        .unwrap()
        .unwrap();
    let mut r = &out[..];
    assert_eq!(r.read_i64().unwrap(), 123);
    assert_eq!(r.read_varint().unwrap(), 4);

    let mut body = Vec::new();
    body.write_varint(0); // window
    body.write_varint(3); // state id
    body.write_i16(36);
    body.write_varint(2); // count
    body.write_varint(10); // stick
    body.write_varint(0);
    body.write_varint(0);
    let out = clientbound(&mut s, "minecraft:container_set_slot", &body)
        .unwrap()
        .unwrap();
    assert_eq!(out[out.len() - 3], 3, "stick has its 26.1.2 item id");
}

#[test]
fn hidden_player_inventory_packet_translates_slot_and_stack() {
    let mut s = session();
    let mut body = Vec::new();
    body.write_varint(40);
    body.write_varint(2);
    body.write_varint(10);
    body.write_varint(0);
    body.write_varint(0);
    let out = clientbound(&mut s, "minecraft:set_player_inventory", &body)
        .unwrap()
        .unwrap();
    let mut r = &out[..];
    assert_eq!(r.read_varint().unwrap(), 40);
    assert_eq!(r.read_varint().unwrap(), 2);
    assert_eq!(r.read_varint().unwrap(), 3);
}

#[test]
fn animations_and_team_parameters_follow_the_old_layout() {
    let mut s = session();
    let mut body = Vec::new();
    body.write_varint(9);
    body.write_varint(1); // off hand
    body.write_varint(1);
    body.write_varint(6);
    let out = clientbound(&mut s, "minecraft:swing_animation", &body)
        .unwrap()
        .unwrap();
    assert_eq!(out, [9, 3]);
    assert_eq!(
        legacy_name("minecraft:swing_animation"),
        "minecraft:animate"
    );
    let out = clientbound(&mut s, "minecraft:animate", &[9, 1])
        .unwrap()
        .unwrap();
    assert_eq!(out, [9, 4], "critical hit is 4 in 26.1.2");

    let text = |s: &str| {
        let mut out = Vec::new();
        mc_nbt::write_network_value(&mut out, &mc_nbt::Tag::String(s.into())).unwrap();
        out
    };
    let mut body = Vec::new();
    body.write_string("red", 32767).unwrap();
    body.write_u8(2);
    body.extend_from_slice(&text("Red"));
    body.extend_from_slice(&text("[R] "));
    body.extend_from_slice(&text(""));
    body.write_varint(1);
    body.write_varint(2);
    body.write_bool(true);
    body.write_varint(12);
    body.write_u8(3);
    let out = clientbound(&mut s, "minecraft:set_player_team", &body)
        .unwrap()
        .unwrap();
    let mut expected = Vec::new();
    expected.write_string("red", 32767).unwrap();
    expected.write_u8(2);
    expected.extend_from_slice(&text("Red"));
    expected.write_u8(3);
    expected.write_varint(1);
    expected.write_varint(2);
    expected.write_varint(12);
    expected.extend_from_slice(&text("[R] "));
    expected.extend_from_slice(&text(""));
    assert_eq!(out, expected);
}

#[test]
fn serverbound_actions_signs_and_teleports_gain_26_3_fields() {
    let mut s = session();
    let mut body = Vec::new();
    body.write_varint(2); // 26.1.2 STOP_DESTROY
    body.write_i64(1);
    body.write_u8(1);
    body.write_varint(7);
    let out = serverbound(&mut s, "minecraft:player_action", &body)
        .unwrap()
        .unwrap();
    assert_eq!(out[0], 3, "CHANGE_DESTROY_DIRECTION shifts later actions");

    let mut body = Vec::new();
    body.write_i64(77);
    body.write_bool(true);
    for line in ["a", "b", "c", "d"] {
        body.write_string(line, 384).unwrap();
    }
    let out = serverbound(&mut s, "minecraft:sign_update", &body)
        .unwrap()
        .unwrap();
    let mut r = &out[..];
    assert_eq!(r.read_i64().unwrap(), 77);
    for line in ["a", "b", "c", "d"] {
        assert_eq!(r.read_string(384).unwrap(), line);
    }
    assert_eq!(r.read_varint().unwrap(), 1, "front text slot");

    let mut teleport = Vec::new();
    teleport.write_varint(4);
    for v in [1.5, 70.0, -2.5, 0.0, 0.0, 0.0] {
        teleport.write_f64(v);
    }
    teleport.write_f32(90.0);
    teleport.write_f32(10.0);
    teleport.write_i32(0);
    let mut writer = s.split();
    let echoed = clientbound(&mut writer, "minecraft:player_position", &teleport)
        .unwrap()
        .unwrap();
    assert_eq!(echoed, teleport, "player_position bytes are unchanged");
    let out = serverbound(&mut s, "minecraft:accept_teleportation", &[4])
        .unwrap()
        .unwrap();
    let mut r = &out[..];
    assert_eq!(r.read_varint().unwrap(), 4);
    assert_eq!(
        [
            r.read_f64().unwrap(),
            r.read_f64().unwrap(),
            r.read_f64().unwrap()
        ],
        [1.5, 70.0, -2.5]
    );
    assert_eq!([r.read_f32().unwrap(), r.read_f32().unwrap()], [90.0, 10.0]);
    assert!(r.is_empty());
}

#[test]
fn swing_and_spectator_inputs_keep_working() {
    let mut s = session();
    assert_eq!(
        serverbound(&mut s, "minecraft:swing", &[1]).unwrap(),
        Some(Vec::new())
    );
    let out = serverbound(&mut s, "minecraft:spectate_entity", &[7])
        .unwrap()
        .unwrap();
    assert_eq!(out, [8], "OptionalInt is entity id + 1");
}

#[test]
fn sign_editor_translates_text_slot_to_front_flag() {
    let mut s = session();
    let mut body = Vec::new();
    body.write_i64(77);
    body.write_varint(0); // back
    let out = clientbound(&mut s, "minecraft:open_sign_editor", &body)
        .unwrap()
        .unwrap();
    assert_eq!(&out[..8], &body[..8]);
    assert_eq!(out[8], 0);
}

#[test]
fn block_entity_payload_and_respawn_tail_are_consumed() {
    let mut s = test_session(Tables {
        block_entities: IdMap::by_name([("chest", 7)], [("chest", 1)], |_| None),
        registry_ids: [(
            "dimension_type".into(),
            IdMap::by_name([("overworld", 0)], [("overworld", 0)], |_| None),
        )]
        .into(),
        ..Tables::default()
    });
    let mut body = Vec::new();
    body.write_i64(123);
    body.write_varint(7);
    body.extend([10, 0]); // empty network NBT compound
    let out = clientbound(&mut s, "minecraft:block_entity_data", &body)
        .unwrap()
        .unwrap();
    assert_eq!(out[8], 1);
    assert_eq!(&out[9..], &[10, 0]);

    let mut body = vec![0]; // dimension type
    body.write_string("minecraft:overworld", 32767).unwrap();
    body.write_i64(42);
    body.extend([1, 0, 0, 0, 0, 0, 63, 3]);
    let out = clientbound(&mut s, "minecraft:respawn", &body)
        .unwrap()
        .unwrap();
    assert_eq!(*out.last().unwrap(), 3);
}

#[test]
fn decorated_pot_26_3_update_and_chunk_downgrade_named_templates() {
    use mc_nbt::Tag;
    let native_kind =
        mc_data::block_entity_kinds::BlockEntityKind::DecoratedPot.protocol_id() as i32;
    let mut s = test_session(Tables {
        block_entities: IdMap::by_name(
            [("decorated_pot", native_kind as u32)],
            [("decorated_pot", 42)],
            |_| None,
        ),
        decorated_pot: Some(native_kind),
        ..Tables::default()
    });
    let tag = Tag::Compound(vec![
        (
            "sherds".into(),
            Tag::Compound(vec![
                (
                    "back".into(),
                    Tag::Compound(vec![
                        (
                            "id".into(),
                            Tag::String("minecraft:angler_pottery_sherd".into()),
                        ),
                        (
                            "components".into(),
                            Tag::Compound(vec![(
                                "minecraft:rarity".into(),
                                Tag::String("rare".into()),
                            )]),
                        ),
                    ]),
                ),
                (
                    "front".into(),
                    Tag::Compound(vec![(
                        "id".into(),
                        Tag::String("minecraft:archer_pottery_sherd".into()),
                    )]),
                ),
            ]),
        ),
        (
            "item".into(),
            Tag::Compound(vec![
                ("id".into(), Tag::String("minecraft:stone".into())),
                ("count".into(), Tag::Int(3)),
            ]),
        ),
    ]);
    let mut encoded = Vec::new();
    mc_nbt::write_network(&mut encoded, &tag).unwrap();
    let mut update = Vec::new();
    update.write_i64(123);
    update.write_varint(native_kind);
    update.extend_from_slice(&encoded);
    let mut chunk = vec![0; 8]; // x,z
    chunk.extend([0, 0, 1, 0x11, 0, 64]); // heightmaps, sections, one BE, xz,y
    chunk.write_varint(native_kind);
    chunk.extend_from_slice(&encoded);
    chunk.extend([0; 6]); // four light masks and two update lists
    for (name, body, header) in [
        ("minecraft:block_entity_data", update, 9),
        ("minecraft:level_chunk_with_light", chunk, 15),
    ] {
        let output = clientbound(&mut s, name, &body).unwrap().unwrap();
        let mut input = &output[header..];
        let Tag::Compound(fields) = mc_nbt::read_network(&mut input).unwrap() else {
            panic!()
        };
        let expected = Tag::List(mc_nbt::ListTag {
            element_type: mc_nbt::tag_type::STRING,
            elements: [
                "minecraft:angler_pottery_sherd",
                "minecraft:brick",
                "minecraft:brick",
                "minecraft:archer_pottery_sherd",
            ]
            .map(|name| Tag::String(name.into()))
            .to_vec(),
        });
        assert_eq!(
            mc_data::item_nbt::field(&fields, "sherds"),
            Some(&expected),
            "{name}"
        );
        let Tag::Compound(source) = &tag else {
            unreachable!()
        };
        assert_eq!(
            mc_data::item_nbt::field(&fields, "item"),
            mc_data::item_nbt::field(source, "item")
        );
        assert_eq!(input, if header == 9 { &[][..] } else { &[0; 6][..] });
    }
}

#[test]
fn new_cube_spawns_as_slime_with_legacy_size_metadata() {
    let tables = Tables {
        entities: IdMap::by_name([("sulfur_cube", 150)], [("slime", 80)], |_| {
            Some("slime".into())
        }),
        entity_metadata: [(150, Metadata::Cube)].into(),
        ..Tables::default()
    };
    let mut s = test_session(tables);
    let mut body = vec![7];
    body.extend([0; 16]); // UUID
    body.write_varint(150);
    for value in [1.0, 65.0, -2.0] {
        body.write_f64(value);
    }
    body.extend([0, 0, 0, 0, 0]); // zero LP velocity, rotations, spawn data
    let out = clientbound(&mut s, "minecraft:add_entity", &body)
        .unwrap()
        .unwrap();
    assert_eq!(out[17], 80);
    let body = [7, 16, 8, 1, 17, 8, 0, 18, 1, 4, 255];
    let out = clientbound(&mut s, "minecraft:set_entity_data", &body)
        .unwrap()
        .unwrap();
    assert_eq!(
        out,
        [7, 16, 1, 4, 255],
        "size moves to index 16; new age fields vanish"
    );
}

#[test]
fn creative_polyfill_tracks_player_inventory_images_and_updates() {
    let mut writer = test_session(Tables {
        items: IdMap::by_name([("stone", 10), ("sulfur", 11)], [("stone", 1)], |name| {
            (name == "sulfur").then(|| "stone".to_owned())
        }),
        ..Tables::default()
    });
    let mut reader = writer.split();
    let echo = [0, 36, 1, 1, 0, 0];
    let mut content = Vec::new();
    content.write_varint(0);
    content.write_varint(1);
    content.write_varint(46);
    for slot in 0..46 {
        if slot == 36 {
            content.extend([1, 11, 0, 0]);
        } else {
            content.push(0);
        }
    }
    content.push(0);
    clientbound(&mut writer, "container_set_content", &content).unwrap();
    assert!(
        serverbound(&mut reader, "set_creative_mode_slot", &echo)
            .unwrap()
            .is_none()
    );

    let mut update = Vec::new();
    update.write_varint(0);
    update.write_varint(2);
    update.write_i16(36);
    update.extend([1, 10, 0, 0]);
    clientbound(&mut writer, "container_set_slot", &update).unwrap();
    assert!(
        serverbound(&mut reader, "set_creative_mode_slot", &echo)
            .unwrap()
            .is_some()
    );

    update[5] = 11;
    clientbound(&mut writer, "container_set_slot", &update).unwrap();
    assert!(
        serverbound(&mut reader, "set_creative_mode_slot", &echo)
            .unwrap()
            .is_none()
    );

    let empty = [0, 3, 0, 0]; // Window 0, state 3, empty list and cursor.
    clientbound(&mut writer, "container_set_content", &empty).unwrap();
    assert!(
        serverbound(&mut reader, "set_creative_mode_slot", &echo)
            .unwrap()
            .is_some()
    );
}

#[test]
fn unchanged_776_to_775_teleports_do_not_accumulate_777_echo_state() {
    let mut session = test_session(Tables {
        formats: super::super::tables::Formats {
            upper: 776,
            lower: 775,
        },
        ..Tables::default()
    });
    let mut body = Vec::new();
    body.write_varint(4);
    for value in [1.5, 70.0, -2.5, 0.0, 0.0, 0.0] {
        body.write_f64(value);
    }
    body.write_f32(90.0);
    body.write_f32(10.0);
    body.write_i32(0);
    assert_eq!(
        clientbound(&mut session, "minecraft:player_position", &body)
            .unwrap()
            .unwrap(),
        body
    );
    assert!(session.teleports.lock().sent.is_empty());
    assert_eq!(
        serverbound(&mut session, "minecraft:accept_teleportation", &[4])
            .unwrap()
            .unwrap(),
        [4]
    );
    let mut position = Vec::new();
    for value in [1.5, 70.0, -2.5] {
        position.write_f64(value);
    }
    position.write_u8(1);
    assert_eq!(
        serverbound(&mut session, "minecraft:move_player_pos", &position)
            .unwrap()
            .unwrap(),
        position
    );
    assert!(session.teleports.lock().pose.is_none());
}

#[test]
fn spectator_entity_option_is_added_only_at_the_775_to_776_boundary() {
    let mut lower = test_session(Tables {
        formats: super::super::tables::Formats {
            upper: 776,
            lower: 775,
        },
        ..Tables::default()
    });
    let converted = serverbound(&mut lower, "minecraft:spectate_entity", &[7])
        .unwrap()
        .unwrap();
    assert_eq!(converted, [8]);
    let mut upper = test_session(Tables {
        formats: super::super::tables::Formats {
            upper: 777,
            lower: 776,
        },
        ..Tables::default()
    });
    assert_eq!(
        serverbound(&mut upper, "minecraft:spectator_action", &converted)
            .unwrap()
            .unwrap(),
        [8]
    );
    assert_eq!(
        serverbound(&mut upper, "minecraft:spectator_action", &[0])
            .unwrap()
            .unwrap(),
        [0]
    );
}

#[test]
fn unknown_block_state_is_not_a_documented_polyfill() {
    let mut body = Vec::new();
    body.write_i64(0);
    body.write_varint(123456);
    let error = clientbound(&mut session(), "minecraft:block_update", &body)
        .unwrap_err()
        .to_string();
    assert!(error.contains("unknown block-state 123456"), "{error}");
    assert!(error.contains("777 -> 775"), "{error}");
}

#[test]
fn dimension_and_painting_ids_follow_names_when_orders_differ() {
    let mut s = session();
    let t = &mut std::sync::Arc::get_mut(&mut s.profile).unwrap().tables;
    t.registry_ids.insert(
        "dimension_type".into(),
        IdMap::by_name(
            [("overworld", 0), ("nether", 1)],
            [("nether", 0), ("overworld", 1)],
            |_| None,
        ),
    );
    t.registry_ids.insert(
        "painting_variant".into(),
        IdMap::by_name(
            [("earth", 0), ("wind", 1)],
            [("wind", 0), ("earth", 1)],
            |_| None,
        ),
    );
    let mut spawn = vec![0];
    spawn.write_string("minecraft:overworld", 32767).unwrap();
    spawn.write_i64(42);
    spawn.extend([1, 0, 0, 0, 0, 0, 63]);
    let mut output = Vec::new();
    spawn_info(t, &mut spawn.as_slice(), &mut output).unwrap();
    assert_eq!(
        output[0], 1,
        "dimension_type follows overworld, not ordinal zero"
    );
    let mut output = Vec::new();
    assert!(metadata_value(t, 34, &mut &[1][..], &mut output).unwrap());
    assert_eq!(
        output,
        [2],
        "holder offset surrounds the named painting mapping"
    );
}
