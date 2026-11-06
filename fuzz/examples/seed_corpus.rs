//! Writes the structure-aware seed corpus: valid packets encoded by the real
//! encoders (so libFuzzer starts past the length/enum checks), valid frame
//! streams and valid NBT documents.
//!
//! `cargo run --release --example seed_corpus -- corpus` from `fuzz/`.

use std::path::PathBuf;

use mc_data::item_stack::ItemStack;
use mc_nbt::{ListTag, Tag, tag_type};
use mc_protocol::codec::Identifier;
use mc_protocol::frame::{Compression, encode_frame};
use mc_protocol::packets::handshake::{Handshake, NextState};
use mc_protocol::packets::login::{EncryptionResponse, LoginPluginResponse, LoginStart};
use mc_protocol::packets::play::*;
use mc_protocol::packets::{ClientInformation, CustomPayload, Packet, configuration, status};
use solaris_fuzz::{PLAY, PRE_PLAY, selector};
use uuid::Uuid;

struct Writer {
    root: PathBuf,
    count: usize,
}

impl Writer {
    fn put(&mut self, target: &str, name: &str, bytes: &[u8]) {
        let dir = self.root.join(target);
        std::fs::create_dir_all(&dir).expect("create corpus dir");
        std::fs::write(dir.join(format!("seed_{name}")), bytes).expect("write seed");
        self.count += 1;
    }

    fn packet<P: Packet>(
        &mut self,
        target: &str,
        table: &[solaris_fuzz::Decoder],
        name: &str,
        p: &P,
    ) {
        let mut body = vec![selector(table, name)];
        p.encode(&mut body).expect("seed packet encodes");
        P::decode(&mut &body[1..]).expect("seed packet decodes: it must reach past the decoder");
        let unique = format!("{name}_{}", self.count);
        self.put(target, &unique, &body);
    }
}

fn hand() -> InteractionHand {
    InteractionHand::MainHand
}

fn id(text: &str) -> Identifier {
    Identifier::parse(text).expect("valid identifier")
}

fn compound(entries: Vec<(&str, Tag)>) -> Tag {
    Tag::Compound(
        entries
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect(),
    )
}

fn nbt_samples() -> Vec<(&'static str, Tag)> {
    vec![
        ("empty", compound(vec![])),
        (
            "mixed",
            compound(vec![
                ("byte", Tag::Byte(-3)),
                ("short", Tag::Short(300)),
                ("int", Tag::Int(-70_000)),
                ("long", Tag::Long(1 << 40)),
                ("float", Tag::Float(1.5)),
                ("double", Tag::Double(-2.25)),
                ("bytes", Tag::ByteArray(vec![1, 2, 3])),
                ("string", Tag::String("héllo \u{0}\u{1F600}".into())),
                ("ints", Tag::IntArray(vec![1, -2, 3])),
                ("longs", Tag::LongArray(vec![i64::MIN, i64::MAX])),
            ]),
        ),
        (
            "nested",
            compound(vec![(
                "list",
                Tag::List(ListTag {
                    element_type: tag_type::COMPOUND,
                    elements: vec![
                        compound(vec![("a", Tag::Int(1))]),
                        compound(vec![("b", compound(vec![("c", Tag::Byte(1))]))]),
                    ],
                }),
            )]),
        ),
        (
            "list_of_lists",
            compound(vec![(
                "l",
                Tag::List(ListTag {
                    element_type: tag_type::LIST,
                    elements: vec![
                        Tag::List(ListTag {
                            element_type: tag_type::INT,
                            elements: vec![Tag::Int(7), Tag::Int(8)],
                        }),
                        Tag::List(ListTag::empty()),
                    ],
                }),
            )]),
        ),
    ]
}

fn stacks() -> Vec<(&'static str, ItemStack)> {
    let custom = {
        let mut bytes = Vec::new();
        mc_nbt::write_named(
            &mut bytes,
            "",
            &compound(vec![("k", Tag::String("v".into())), ("n", Tag::Int(5))]),
        )
        .expect("custom data encodes");
        bytes
    };
    vec![
        ("empty", ItemStack::EMPTY),
        ("plain", ItemStack::new(896, 1)),
        ("damaged", ItemStack::new(896, 3).with_damage(12)),
        (
            "enchanted",
            ItemStack::new(896, 1).with_enchantment(id("minecraft:sharpness"), 3),
        ),
        ("named", ItemStack::new(896, 1).with_custom_name("seed")),
        (
            "model",
            ItemStack::new(896, 1).with_item_model(id("minecraft:stone")),
        ),
        (
            "custom_data",
            ItemStack::new(896, 1)
                .with_custom_data(&custom)
                .expect("custom data"),
        ),
        (
            "everything",
            ItemStack::new(896, 2)
                .with_damage(1)
                .with_enchantment(id("minecraft:unbreaking"), 2)
                .with_custom_name("all")
                .with_custom_data(&custom)
                .expect("custom data"),
        ),
    ]
}

fn main() {
    let root = std::env::args_os()
        .nth(1)
        .map_or_else(|| PathBuf::from("fuzz/corpus"), PathBuf::from);
    let mut w = Writer { root, count: 0 };

    // --- packets_pre_play -------------------------------------------------
    let t = "packets_pre_play";
    w.packet(
        t,
        PRE_PLAY,
        "Handshake",
        &Handshake {
            protocol_version: 777,
            server_address: "localhost".into(),
            server_port: 25565,
            next_state: NextState::Login,
        },
    );
    w.packet(t, PRE_PLAY, "StatusRequest", &status::StatusRequest);
    w.packet(
        t,
        PRE_PLAY,
        "PingRequest",
        &status::PingRequest { payload: 42 },
    );
    w.packet(
        t,
        PRE_PLAY,
        "LoginStart",
        &LoginStart {
            name: "Steve".into(),
            player_uuid: Uuid::from_u128(7),
        },
    );
    w.packet(
        t,
        PRE_PLAY,
        "EncryptionResponse",
        &EncryptionResponse {
            encrypted_shared_secret: vec![1; 128],
            encrypted_verify_token: vec![2; 128],
        },
    );
    w.packet(
        t,
        PRE_PLAY,
        "LoginPluginResponse",
        &LoginPluginResponse {
            message_id: 1,
            successful: true,
            data: vec![9; 40],
        },
    );
    w.packet(
        t,
        PRE_PLAY,
        "ConfigKnownPacks",
        &configuration::ServerboundKnownPacks {
            packs: vec![configuration::KnownPackEntry {
                namespace: "minecraft".into(),
                id: "core".into(),
                version: "26.3".into(),
            }],
        },
    );
    let info = ClientInformation::decode(&mut &[2u8, b'e', b'n', 12, 0, 1, 0x7f, 1, 0, 1, 0][..])
        .expect("client information bytes");
    w.packet(
        t,
        PRE_PLAY,
        "ConfigClientInformation",
        &configuration::ServerboundClientInformation { information: info },
    );
    w.packet(
        t,
        PRE_PLAY,
        "ConfigCustomPayload",
        &configuration::ServerboundCustomPayload {
            payload: CustomPayload::Brand("vanilla".into()),
        },
    );
    w.packet(
        t,
        PRE_PLAY,
        "ConfigCustomPayload",
        &configuration::ServerboundCustomPayload {
            payload: CustomPayload::Unknown {
                channel: id("solaris:voice"),
                payload: vec![1, 2, 3],
            },
        },
    );

    // --- packets_play / edge_play -----------------------------------------
    fn play<P: Packet>(w: &mut Writer, name: &str, p: &P) {
        let mut body = vec![selector(PLAY, name)];
        p.encode(&mut body).expect("seed packet encodes");
        P::decode(&mut &body[1..]).expect("seed packet decodes: it must reach past the decoder");
        let unique = format!("{name}_{}", w.count);
        w.put("packets_play", &unique, &body);
        w.put("edge_play", &unique, &body);
    }
    play(
        &mut w,
        "ServerboundChat",
        &ServerboundChat {
            message: "hello".into(),
            timestamp_millis: 1,
            salt: 2,
            signature: None,
            last_seen_offset: 0,
            last_seen_acknowledged: [0; 3],
            last_seen_checksum: 0,
        },
    );
    play(
        &mut w,
        "ServerboundChatCommand",
        &ServerboundChatCommand {
            command: "gamemode creative".into(),
        },
    );
    play(
        &mut w,
        "ServerboundChatCommandSigned",
        &ServerboundChatCommandSigned {
            command: "msg a b".into(),
            timestamp_millis: 1,
            salt: 2,
            signatures: vec![("arg".into(), [3; 256])],
            last_seen_offset: 1,
            last_seen_acknowledged: [1, 2, 3],
            last_seen_checksum: 5,
        },
    );
    play(
        &mut w,
        "ServerboundChatSessionUpdate",
        &ServerboundChatSessionUpdate {
            session_id: Uuid::from_u128(1),
            expires_at_millis: 99,
            public_key: vec![1; 294],
            key_signature: vec![2; 256],
        },
    );
    play(
        &mut w,
        "ServerboundSignUpdate",
        &ServerboundSignUpdate {
            position: 1234,
            lines: vec!["a".into(), "b".into(), "c".into(), "d".into()],
            is_front_text: true,
        },
    );
    play(
        &mut w,
        "ServerboundEditBook",
        &ServerboundEditBook {
            slot: 3,
            pages: vec!["page one".into(), "page two".into()],
            title: Some("title".into()),
        },
    );
    play(
        &mut w,
        "ServerboundCommandSuggestion",
        &ServerboundCommandSuggestion {
            id: 4,
            command: "/give @s ".into(),
        },
    );
    play(
        &mut w,
        "ServerboundCustomPayload",
        &ServerboundCustomPayload {
            payload: CustomPayload::Unknown {
                channel: id("solaris:lod/request"),
                payload: vec![0, 1, 2, 3, 4],
            },
        },
    );
    play(
        &mut w,
        "ServerboundClientInformation",
        &ServerboundClientInformation {
            information: ClientInformation::decode(
                &mut &[2u8, b'e', b'n', 12, 0, 1, 0x7f, 1, 0, 1, 0][..],
            )
            .expect("client information bytes"),
        },
    );
    play(
        &mut w,
        "ServerboundUseItemOn",
        &ServerboundUseItemOn {
            hand: hand(),
            position: 0x0123_4567,
            direction: mc_protocol::packets::play::Direction::Up,
            cursor_x: 0.5,
            cursor_y: 0.5,
            cursor_z: 0.5,
            inside: false,
            world_border_hit: false,
            sequence: 9,
        },
    );
    play(
        &mut w,
        "ServerboundInteract",
        &ServerboundInteract {
            entity_id: 5,
            hand: hand(),
            location: EntityVec3 {
                x: 0.1,
                y: 0.2,
                z: 0.3,
            },
            using_secondary_action: true,
        },
    );
    play(
        &mut w,
        "ServerboundContainerClick",
        &ServerboundContainerClick {
            container_id: 1,
            state_id: 2,
            slot_num: 10,
            button_num: 0,
            container_input: ContainerInput::Pickup,
            changed_slots: vec![
                (10, HashedStack::empty()),
                (
                    11,
                    HashedStack::Actual {
                        item_id: 896,
                        count: 2,
                        components: HashedStackComponentHashes {
                            added: vec![(1, 123), (2, -5)],
                            removed: vec![7],
                        },
                    },
                ),
            ],
            carried_item: HashedStack::Actual {
                item_id: 1,
                count: 1,
                components: HashedStackComponentHashes::empty(),
            },
        },
    );
    for (_, stack) in stacks() {
        play(
            &mut w,
            "ServerboundSetCreativeModeSlot",
            &ServerboundSetCreativeModeSlot {
                slot: 36,
                item_stack: stack,
            },
        );
    }

    // --- frame_stream: valid streams for each compression kind.
    for (kind, compression) in [
        (0u8, Compression::Disabled),
        (1, Compression::Threshold(0)),
        (2, Compression::Threshold(1)),
    ] {
        let mut wire = Vec::new();
        for (frame_id, body) in [(0x1a, vec![1u8; 4]), (0x39, vec![2u8; 300]), (0x00, vec![])] {
            wire.extend_from_slice(&encode_frame(frame_id, &body, compression).expect("frame"));
        }
        // Raw-mode header: compression, threshold (u16 LE), chunk size.
        let mut seed = vec![kind, 1, 0, 7];
        seed.extend_from_slice(&wire);
        w.put("frame_stream", &format!("stream_{kind}"), &seed);
    }

    // --- loader_wire: selector 0 ack, 1 artifact request, 2 view request -----
    for (selector, name, json) in [
        (
            0u8,
            "ack",
            r#"{"protocol":3,"platform":"fabric","loader_version":"1.0","accepted_permissions":[],"cached_bundles":["a"]}"#,
        ),
        (1, "artifact", r#"{"protocol":3,"cache_key":"abc123"}"#),
        (
            2,
            "view_action",
            r#"{"protocol":3,"message":"view_action","view_instance_id":"v1","view_revision":2,"action_id":"go","action_sequence":3,"fields":[{"id":"n","number":1.5},{"id":"t","text":"x"}],"selection_token":"tok"}"#,
        ),
        (
            2,
            "cancel",
            r#"{"protocol":3,"message":"cancel_selection","selection_context_id":"c"}"#,
        ),
    ] {
        let mut seed = vec![selector];
        seed.extend_from_slice(json.as_bytes());
        match selector {
            0 => drop(mc_loader::LoaderClientAck::decode(json.as_bytes()).expect("ack seed")),
            1 => drop(
                mc_loader::LoaderArtifactRequest::decode(json.as_bytes()).expect("artifact seed"),
            ),
            _ => drop(
                mc_loader::decode_loader_view_client_request(json.as_bytes()).expect("view seed"),
            ),
        }
        w.put("loader_wire", name, &seed);
    }

    // --- nbt ---------------------------------------------------------------
    for (name, tag) in nbt_samples() {
        let mut bytes = Vec::new();
        mc_nbt::write_network(&mut bytes, &tag).expect("network nbt");
        w.put("nbt", &format!("network_{name}"), &bytes);
        let mut named = Vec::new();
        mc_nbt::write_named(&mut named, "root", &tag).expect("named nbt");
        w.put("nbt", &format!("named_{name}"), &named);
    }

    println!("wrote {} seeds under {}", w.count, w.root.display());
}
