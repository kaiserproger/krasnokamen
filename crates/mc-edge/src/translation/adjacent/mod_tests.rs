//! Session phases and frame-level translation over synthetic packet tables.

use bytes::{Bytes, BytesMut};
use mc_protocol::{Compression, RawFrame};

use super::tables::{Packets, Reports};
use super::*;

const CANONICAL: &str = r#"{
    "login": {
        "clientbound": {"minecraft:login_disconnect": {"protocol_id": 0}, "minecraft:login_finished": {"protocol_id": 2}},
        "serverbound": {"minecraft:hello": {"protocol_id": 0}, "minecraft:login_acknowledged": {"protocol_id": 3}}
    },
    "configuration": {
        "clientbound": {"minecraft:finish_configuration": {"protocol_id": 3}, "minecraft:post_effects": {"protocol_id": 10}, "minecraft:store_cookie": {"protocol_id": 11}, "minecraft:registry_data": {"protocol_id": 12}},
        "serverbound": {"minecraft:finish_configuration": {"protocol_id": 3}}
    },
    "play": {
        "clientbound": {"minecraft:block_update": {"protocol_id": 9}, "minecraft:add_transient_block": {"protocol_id": 37}, "minecraft:keep_alive": {"protocol_id": 45}, "minecraft:set_player_inventory": {"protocol_id": 100}},
        "serverbound": {"minecraft:keep_alive": {"protocol_id": 28}, "minecraft:punch": {"protocol_id": 46}, "minecraft:set_creative_mode_slot": {"protocol_id": 57}}
    }
}"#;

const LEGACY: &str = r#"{
    "login": {
        "clientbound": {"minecraft:login_disconnect": {"protocol_id": 0}, "minecraft:login_finished": {"protocol_id": 2}},
        "serverbound": {"minecraft:hello": {"protocol_id": 0}, "minecraft:login_acknowledged": {"protocol_id": 3}}
    },
    "configuration": {
        "clientbound": {"minecraft:finish_configuration": {"protocol_id": 3}, "minecraft:store_cookie": {"protocol_id": 10}},
        "serverbound": {"minecraft:finish_configuration": {"protocol_id": 3}}
    },
    "play": {
        "clientbound": {"minecraft:block_update": {"protocol_id": 9}, "minecraft:keep_alive": {"protocol_id": 44}, "minecraft:set_player_inventory": {"protocol_id": 99}},
        "serverbound": {"minecraft:keep_alive": {"protocol_id": 27}, "minecraft:swing": {"protocol_id": 63}, "minecraft:set_creative_mode_slot": {"protocol_id": 98}}
    }
}"#;

fn session() -> Session {
    let canonical = Reports::parse(CANONICAL, "{}", "{}").unwrap();
    let legacy = Reports::parse(LEGACY, "{}", "{}").unwrap();
    test_session(Tables {
        packets: Packets::new(&canonical, &legacy),
        ..Tables::default()
    })
}

fn frame(id: i32, body: &[u8]) -> RawFrame {
    RawFrame {
        id,
        body: Bytes::copy_from_slice(body),
    }
}

#[test]
fn legacy_creative_echo_keeps_unknown_item_but_real_changes_pass() {
    use super::tables::IdMap;
    use mc_protocol::codec::WriteMc;

    let mut writer = session();
    Arc::get_mut(&mut writer.profile).unwrap().tables.items =
        IdMap::by_name([("stone", 10), ("new_item", 11)], [("stone", 1)], |name| {
            (name == "new_item").then(|| "stone".to_owned())
        });
    writer.phase = Phase::Play;
    let mut reader = writer.split();
    let mut sent = Vec::new();
    sent.write_varint(0);
    for value in [1, 11, 0, 0] {
        sent.write_varint(value);
    }
    let (_, legacy) = writer.clientbound(100, &sent).unwrap().unwrap();
    let mut echo = Vec::new();
    echo.write_i16(36);
    echo.extend_from_slice(&legacy[1..]);
    assert!(
        reader.serverbound(frame(98, &echo)).unwrap().is_none(),
        "stand-in echo must not replace the real item"
    );
    let mut changed = Vec::new();
    changed.write_i16(37);
    for value in [1, 1, 0, 0] {
        changed.write_varint(value);
    }
    let translated = reader.serverbound(frame(98, &changed)).unwrap().unwrap();
    assert_eq!(&translated.body[..], &[0, 37, 1, 10, 0, 0]);
}

#[test]
fn legacy_creative_count_edit_preserves_canonical_player_inventory_item() {
    use super::tables::IdMap;
    use mc_core::boundary::{ConnectionId, PlayerInput};
    use mc_core::commands::GameMode;
    use mc_core::inventory::ItemRules;
    use mc_core::player::PlayerState;
    use mc_data::{Identifier, ItemStack};
    use mc_protocol::codec::WriteMc;

    let items = mc_data::items::solaris_required_items();
    let facts = mc_data::item_components::solaris_required_item_facts();
    let tags = mc_data::tags::TagsData::default();
    let rules = ItemRules {
        items: &items,
        facts: &facts,
        tags: &tags,
        recipes: &[],
    };
    let stone = items
        .id_of(&Identifier::parse("minecraft:stone").unwrap())
        .unwrap();
    let unknown = items
        .id_of(&Identifier::parse("minecraft:oak_log").unwrap())
        .unwrap();
    let mut writer = session();
    Arc::get_mut(&mut writer.profile).unwrap().tables.items = IdMap::by_name(
        [("stone", stone), ("new_item", unknown)],
        [("stone", 1)],
        |name| (name == "new_item").then(|| "stone".to_owned()),
    );
    writer.phase = Phase::Play;
    let mut reader = writer.split();
    let mut player = PlayerState::new(ConnectionId(1), GameMode::Creative);
    player.inventory.slots[0] = ItemStack::new(unknown, 1);
    let mut sent = Vec::new();
    sent.write_varint(0);
    for value in [1, unknown as i32, 0, 0] {
        sent.write_varint(value);
    }
    let (_, legacy) = writer.clientbound(100, &sent).unwrap().unwrap();
    let mut echo = Vec::new();
    echo.write_i16(36);
    echo.extend_from_slice(&legacy[1..]);
    let mut decode = crate::decode::DecodeState::default();
    let mut inputs = Vec::new();
    if let Some(frame) = reader.serverbound(frame(98, &echo)).unwrap() {
        crate::decode::decode_play_frame(
            ConnectionId(1),
            frame.id,
            &frame.body,
            &mut decode,
            &mut inputs,
        )
        .unwrap();
    }
    for input in inputs.drain(..) {
        if let PlayerInput::CreativeSlot { slot, stack, .. } = input {
            player.creative_slot(&rules, slot, stack);
        }
    }
    assert_eq!(player.inventory.slots[0], ItemStack::new(unknown, 1));
    let mut replacement = Vec::new();
    replacement.write_i16(36);
    for value in [2, 1, 0, 0] {
        replacement.write_varint(value);
    }
    let frame = reader
        .serverbound(frame(98, &replacement))
        .unwrap()
        .unwrap();
    crate::decode::decode_play_frame(
        ConnectionId(1),
        frame.id,
        &frame.body,
        &mut decode,
        &mut inputs,
    )
    .unwrap();
    for input in inputs {
        if let PlayerInput::CreativeSlot { slot, stack, .. } = input {
            player.creative_slot(&rules, slot, stack);
        }
    }
    assert_eq!(player.inventory.slots[0], ItemStack::new(unknown, 2));
}

#[test]
fn phases_follow_the_client_acknowledgements() {
    let mut s = session();
    // Login: the session UUID 26.3 appends to the profile is cut.
    let profile = [1u8; 20];
    let (id, body) = s.clientbound(2, &profile).unwrap().unwrap();
    assert_eq!((id, body.len()), (2, 4));
    assert!(s.serverbound(frame(3, &[])).unwrap().is_some());
    // Configuration: post_effects is dropped, later ids shift back by one.
    assert!(s.clientbound(10, &[0]).unwrap().is_none());
    assert_eq!(s.clientbound(11, &[7]).unwrap().unwrap().0, 10);
    assert_eq!(s.serverbound(frame(3, &[])).unwrap().unwrap().id, 3);
    // Play.
    assert_eq!(s.clientbound(45, &[0; 8]).unwrap().unwrap().0, 44);
    assert!(s.clientbound(37, &[0; 9]).unwrap().is_none());
    assert_eq!(s.serverbound(frame(27, &[0; 8])).unwrap().unwrap().id, 28);
    assert_eq!(
        s.serverbound(frame(63, &[0])).unwrap().unwrap().id,
        46,
        "legacy swings become the canonical empty punch"
    );
    assert!(
        s.serverbound(frame(99, &[])).is_err(),
        "unknown ids fail closed"
    );
}

#[test]
fn pre_framed_batches_are_reframed_with_compression() {
    let mut s = session();
    s.phase = Phase::Play;
    let compression = Compression::Threshold(16);
    let mut batch = BytesMut::new();
    mc_protocol::frame::append_frame(&mut batch, 45, &[1; 8], compression).unwrap();
    mc_protocol::frame::append_frame(&mut batch, 37, &[2; 40], compression).unwrap();
    mc_protocol::frame::append_frame(&mut batch, 45, &[3; 64], compression).unwrap();
    let mut out = s.clientbound_frames(&batch, compression).unwrap();
    let first = mc_protocol::frame::try_decode_frame(&mut out, compression)
        .unwrap()
        .unwrap();
    assert_eq!((first.id, &first.body[..]), (44, &[1u8; 8][..]));
    let second = mc_protocol::frame::try_decode_frame(&mut out, compression)
        .unwrap()
        .unwrap();
    assert_eq!((second.id, &second.body[..]), (44, &[3u8; 64][..]));
    assert!(out.is_empty(), "the 26.3-only frame is gone");
}

#[test]
fn malformed_world_updates_fail_instead_of_being_silently_lost() {
    let mut s = session();
    s.phase = Phase::Play;
    assert!(s.clientbound(9, &[]).is_err());
}

#[test]
fn missing_target_packet_errors_unless_explicitly_presentation_only() {
    let mut session = session();
    session.phase = Phase::Configuration;
    assert!(session.clientbound(10, &[]).unwrap().is_none());
    let error = session.clientbound(12, &[]).unwrap_err().to_string();
    for detail in [
        "777 -> 775",
        "Configuration",
        "minecraft:registry_data",
        "0x0c",
    ] {
        assert!(error.contains(detail), "{error}");
    }
}

#[test]
fn creative_slot_echo_of_lossy_pot_components_keeps_authoritative_stack() {
    use super::items::Components;
    use super::tables::IdMap;
    use mc_protocol::codec::WriteMc;

    for peer in [775, 776] {
        for case in ["missing_side", "nested_count", "custom_name", "combined"] {
            let mut writer = session();
            let tables = &mut Arc::get_mut(&mut writer.profile).unwrap().tables;
            tables.formats.lower = peer;
            tables.items = IdMap::by_name(
                [("decorated_pot", 10), ("brick", 11)],
                [("decorated_pot", 10), ("brick", 11)],
                |_| None,
            );
            tables.legacy_brick = 11;
            tables.legacy_pot_item = Some(10);
            let components = [
                "minecraft:pot_decorations".to_owned(),
                "minecraft:custom_name".to_owned(),
            ];
            tables.components = IdMap::by_name(
                [("pot_decorations", 0), ("custom_name", 1)],
                [("pot_decorations", 0), ("custom_name", 1)],
                |_| None,
            );
            tables.component_shapes = Components::new(&components, &components);
            writer.phase = Phase::Play;
            let mut reader = writer.split();
            let mut body = vec![0, 1, 10, 1, 0, 0]; // hotbar 0, unchanged pot id, one component
            for side in 0..4 {
                let present = !(side == 0 && matches!(case, "missing_side" | "combined"));
                body.write_bool(present);
                if present {
                    body.write_varint(11);
                    body.write_varint(
                        if (side == 0 && case == "nested_count")
                            || (side == 1 && case == "combined")
                        {
                            2
                        } else {
                            1
                        },
                    );
                    let named =
                        (side == 0 && case == "custom_name") || (side == 1 && case == "combined");
                    body.write_varint(i32::from(named));
                    body.write_varint(0);
                    if named {
                        body.write_varint(1);
                        mc_nbt::write_network_value(
                            &mut body,
                            &mc_nbt::Tag::String("Custom brick".into()),
                        )
                        .unwrap();
                    }
                }
            }
            let (_, peer_image) = writer.clientbound(100, &body).unwrap().unwrap();
            let mut echo = Vec::new();
            echo.write_i16(36);
            echo.extend_from_slice(
                &items::creative::delimit(
                    &writer.profile.tables,
                    items::To::Canonical,
                    &peer_image[1..],
                )
                .unwrap(),
            );
            assert!(
                reader.serverbound(frame(98, &echo)).unwrap().is_none(),
                "{peer} {case}: a lossy component echo must not replace the authoritative pot"
            );
            let duplicate_default = [
                0, 36, 1, 10, 2, 0, 0, 5, 4, 11, 11, 11, 11, 0, 5, 4, 11, 11, 11, 11,
            ];
            assert!(reader.serverbound(frame(98, &duplicate_default)).is_err());
            // Vanilla elides the four-brick default when it reserializes its
            // projected pot. This is still the same lossy server-owned stack.
            let normalized = [0, 36, 1, 10, 0, 0];
            assert!(
                reader
                    .serverbound(frame(98, &normalized))
                    .unwrap()
                    .is_none()
            );
            reader.serverbound(frame(98, &[0, 36, 0])).unwrap().unwrap();
            writer.clientbound(100, &[0, 0]).unwrap().unwrap();
            let restored = reader
                .serverbound(frame(98, &[0, 37, 1, 10, 0, 0]))
                .unwrap()
                .unwrap();
            let mut expected = vec![0, 37];
            expected.extend_from_slice(
                &items::creative::delimit(&writer.profile.tables, items::To::Legacy, &body[1..])
                    .unwrap(),
            );
            assert_eq!(
                restored.body.as_ref(),
                expected,
                "{peer} {case}: cursor return must restore every canonical component"
            );
            for end in ["drop", "close", "full_refresh", "cursor_refresh"] {
                writer.inventory_polyfills.lock().close();
                writer.clientbound(100, &body).unwrap().unwrap();
                reader.serverbound(frame(98, &[0, 36, 0])).unwrap().unwrap();
                match end {
                    "drop" => {
                        let dropped = reader
                            .serverbound(frame(98, &[255, 255, 1, 10, 0, 0]))
                            .unwrap()
                            .unwrap();
                        let mut expected_drop = expected.clone();
                        expected_drop[..2].copy_from_slice(&[255, 255]);
                        assert_eq!(dropped.body.as_ref(), expected_drop);
                        let again = reader
                            .serverbound(frame(98, &[255, 255, 1, 10, 0, 0]))
                            .unwrap()
                            .unwrap();
                        assert_eq!(again.body.as_ref(), &[255, 255, 1, 10, 0, 0]);
                    }
                    "close" => {
                        play::serverbound_body(&mut reader, "container_close", &[0]).unwrap();
                    }
                    "full_refresh" => {
                        let mut snapshot = vec![0, 0, 46];
                        snapshot.resize(50, 0); // 46 empty slots and empty carried stack
                        play::clientbound_body(&mut writer, "container_set_content", &snapshot)
                            .unwrap();
                    }
                    "cursor_refresh" => {
                        play::clientbound_body(&mut writer, "set_cursor_item", &[0]).unwrap();
                    }
                    _ => unreachable!(),
                }
                let after = reader
                    .serverbound(frame(98, &[0, 37, 1, 10, 0, 0]))
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    after.body.as_ref(),
                    if end == "full_refresh" {
                        expected.as_slice()
                    } else {
                        &[0, 37, 1, 10, 0, 0]
                    },
                    "{peer} {case} {end}: stale provenance"
                );
            }
            for source in ["container_set_content", "set_cursor_item"] {
                writer.inventory_polyfills.lock().close();
                let cursor = if source == "container_set_content" {
                    let mut snapshot = vec![0, 0, 46];
                    snapshot.resize(49, 0);
                    snapshot.extend_from_slice(&body[1..]);
                    snapshot
                } else {
                    body[1..].to_vec()
                };
                play::clientbound_body(&mut writer, source, &cursor).unwrap();
                let returned = reader
                    .serverbound(frame(98, &[0, 37, 1, 10, 0, 0]))
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    returned.body.as_ref(),
                    expected,
                    "{peer} {case}: {source} carried image"
                );
            }
            for count in [2, 64, 128] {
                for source in ["slot", "pickup", "pickup_full_refresh"] {
                    writer.inventory_polyfills.lock().close();
                    writer.clientbound(100, &body).unwrap().unwrap();
                    let slot: i16 = if source == "slot" {
                        36
                    } else {
                        reader.serverbound(frame(98, &[0, 36, 0])).unwrap().unwrap();
                        37
                    };
                    if source == "pickup_full_refresh" {
                        let mut snapshot = vec![0, 0, 46];
                        snapshot.resize(50, 0);
                        play::clientbound_body(&mut writer, "container_set_content", &snapshot)
                            .unwrap();
                    }
                    let mut changed = Vec::new();
                    changed.write_i16(slot);
                    changed.write_varint(count);
                    changed.extend_from_slice(&[10, 0, 0]);
                    let mut count_expected = Vec::new();
                    count_expected.write_i16(slot);
                    count_expected.write_varint(count);
                    count_expected.extend_from_slice(&expected[3..]);
                    let translated = reader.serverbound(frame(98, &changed)).unwrap().unwrap();
                    assert_eq!(
                        translated.body.as_ref(),
                        count_expected,
                        "{peer} {case}: {source} count edit keeps hidden components"
                    );
                }
            }
        }
    }
}

#[test]
fn creative_named_stack_preserves_outer_component_delimiter() {
    use super::items::Components;
    use super::tables::IdMap;

    let mut reader = session();
    let tables = &mut Arc::get_mut(&mut reader.profile).unwrap().tables;
    tables.items = IdMap::by_name([("stone", 10)], [("stone", 1)], |_| None);
    tables.components = IdMap::by_name([("custom_name", 9)], [("custom_name", 6)], |_| None);
    let canonical = (0..10)
        .map(|id| {
            if id == 9 {
                "minecraft:custom_name".to_owned()
            } else {
                String::new()
            }
        })
        .collect::<Vec<_>>();
    let legacy = (0..7)
        .map(|id| {
            if id == 6 {
                "minecraft:custom_name".to_owned()
            } else {
                String::new()
            }
        })
        .collect::<Vec<_>>();
    tables.component_shapes = Components::new(&canonical, &legacy);
    reader.phase = Phase::Play;
    // Same measured framing as the real 775 creative write captured by the
    // graphical gate: id, byte length, network-NBT string.
    let mut creative = vec![0, 36, 1, 1, 1, 0, 6, 15];
    mc_nbt::write_network_value(&mut creative, &mc_nbt::Tag::String("Protocol Pot".into()))
        .unwrap();
    let translated = reader.serverbound(frame(98, &creative)).unwrap().unwrap();
    let mut expected = vec![0, 36, 1, 10, 1, 0, 9, 15];
    mc_nbt::write_network_value(&mut expected, &mc_nbt::Tag::String("Protocol Pot".into()))
        .unwrap();
    assert_eq!(translated.body.as_ref(), expected);
    creative[7] = 14;
    assert!(reader.serverbound(frame(98, &creative)).is_err());
}

#[test]
fn creative_unknown_removed_component_names_id_and_source_protocol() {
    use super::items::Components;
    use super::tables::IdMap;
    use mc_protocol::codec::WriteMc;
    for peer in [775, 776] {
        let mut reader = session();
        let tables = &mut Arc::get_mut(&mut reader.profile).unwrap().tables;
        tables.formats.lower = peer;
        tables.items = IdMap::by_name([("stone", 10)], [("stone", 1)], |_| None);
        tables.component_shapes = Components::new(&[], &["minecraft:custom_name".into()]);
        reader.phase = Phase::Play;
        for id in [-1, 1, 9999] {
            let mut body = vec![0, 36, 1, 1, 0, 1];
            body.write_varint(id);
            let error = format!("{:#}", reader.serverbound(frame(98, &body)).unwrap_err());
            assert!(
                error.contains(&format!("unknown removed component {id}")),
                "{error}"
            );
            assert!(error.contains(&format!("protocol {peer}")), "{error}");
        }
        // The source knows this component but the target does not: an explicit
        // omitted representation is not an unknown/negative source id.
        let translated = reader
            .serverbound(frame(98, &[0, 36, 1, 1, 0, 1, 0]))
            .unwrap()
            .unwrap();
        assert_eq!(translated.body.as_ref(), &[0, 36, 1, 10, 0, 0]);
    }
}
