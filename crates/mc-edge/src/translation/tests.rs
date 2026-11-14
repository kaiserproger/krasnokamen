use super::*;
use adjacent::tables::{Formats, Packets, Reports, Tables};
use bytes::Bytes;
use mc_protocol::codec::{ReadMc, WriteMc};

const PACKETS: &str = r#"{
 "login":{"clientbound":{"minecraft:login_finished":{"protocol_id":2}},"serverbound":{"minecraft:login_acknowledged":{"protocol_id":3}}},
 "configuration":{"clientbound":{"minecraft:finish_configuration":{"protocol_id":3},"minecraft:registry_data":{"protocol_id":7}},"serverbound":{"minecraft:finish_configuration":{"protocol_id":3}}},
 "play":{"clientbound":{"minecraft:login":{"protocol_id":1},"minecraft:block_update":{"protocol_id":9},"minecraft:light_update":{"protocol_id":10},"minecraft:level_particles":{"protocol_id":11}},"serverbound":{"minecraft:keep_alive":{"protocol_id":5},"minecraft:set_creative_mode_slot":{"protocol_id":57},"minecraft:accept_teleportation":{"protocol_id":45}}}
}"#;

fn measured_step(upper: i32, lower: i32, from_state: u32, to_state: u32) -> Step {
    let reports = Reports::parse(PACKETS, "{}", "{}").unwrap();
    let tables = Tables {
        formats: Formats { upper, lower },
        packets: Packets::new(&reports, &reports),
        block_states: adjacent::tables::IdMap::by_name(
            [("stone", from_state)],
            [("stone", to_state)],
            |_| None,
        ),
        ..Tables::default()
    };
    Step {
        upper,
        lower,
        factory: Box::new(adjacent::test_profile(tables)),
    }
}

fn session(steps: Vec<Step>, canonical: i32, peer: i32) -> Session {
    let route = route(&steps, canonical, peer).unwrap();
    let steps: Vec<_> = steps.into_iter().map(Arc::new).collect();
    Session::new(Arc::new(Profile {
        version: mc_data::protocol_versions::by_protocol(peer).unwrap(),
        data: mc_data::VanillaData::from_registries("", Vec::new()),
        tags: mc_data::tags::TagsData::default(),
        route: route
            .into_iter()
            .map(|(index, direction)| (Arc::clone(&steps[index]), direction))
            .collect(),
        tables: Tables::default(),
    }))
}

fn frame(id: i32, body: &[u8]) -> RawFrame {
    RawFrame {
        id,
        body: Bytes::copy_from_slice(body),
    }
}

fn enter_play(session: &mut Session) {
    assert_eq!(session.serverbound(frame(3, &[])).unwrap().unwrap().id, 3);
    assert_eq!(session.serverbound(frame(3, &[])).unwrap().unwrap().id, 3);
}

fn geyser_step(upper: i32, lower: i32) -> Step {
    use adjacent::tables::IdMap;
    let reports = Reports::parse(PACKETS, "{}", "{}").unwrap();
    let mut names = vec![String::new(); 81];
    for (name, id) in [
        ("geyser", 7),
        ("geyser_base", 8),
        ("geyser_poof", 9),
        ("geyser_plume", 10),
        ("splash", if upper == 777 { 80 } else { 77 }),
        ("poof", if upper == 777 { 69 } else { 66 }),
    ] {
        names[id] = format!("minecraft:{name}");
    }
    let source = if upper == 777 {
        vec![
            ("geyser", 7),
            ("geyser_base", 8),
            ("geyser_poof", 9),
            ("geyser_plume", 10),
            ("splash", 80),
            ("poof", 69),
        ]
    } else {
        vec![
            ("geyser", 7),
            ("geyser_base", 8),
            ("geyser_poof", 9),
            ("geyser_plume", 10),
            ("splash", 77),
            ("poof", 66),
        ]
    };
    let target = if lower == 776 {
        vec![
            ("geyser", 7),
            ("geyser_base", 8),
            ("geyser_poof", 9),
            ("geyser_plume", 10),
            ("splash", 77),
            ("poof", 66),
        ]
    } else {
        vec![("splash", 70), ("poof", 59)]
    };
    Step {
        upper,
        lower,
        factory: Box::new(adjacent::test_profile(Tables {
            formats: Formats { upper, lower },
            packets: Packets::new(&reports, &reports),
            particle_names: names,
            particles: IdMap::by_name(source, target, |_| None),
            block_states: IdMap::by_name(
                [("stone", 90), ("stone2", 40)],
                [("stone", 40), ("stone2", 7)],
                |_| None,
            ),
            ..Tables::default()
        })),
    }
}

#[test]
fn full_chain_geyser_options_translate_and_session_remains_usable() {
    for (kind, target, options) in [
        (7, 70, vec![0, 0, 0, 3]),
        (8, 70, vec![0, 0, 0, 3, 0x3f, 0x80, 0, 0]),
        (9, 59, vec![0, 0, 0, 3, 0x3f, 0x80, 0, 0]),
        (10, 70, vec![0, 0, 0, 3]),
    ] {
        let mut session = session(vec![geyser_step(777, 776), geyser_step(776, 775)], 777, 775);
        enter_play(&mut session);
        let mut input = vec![kind];
        input.extend(options);
        input.extend([1, 0]);
        for value in [1.0, 2.0, 3.0] {
            input.write_f64(value);
        }
        for value in [0.25, 0.5, 0.75, 1.0, 1.0, 1.0] {
            input.write_f32(value);
        }
        input.extend([2, 0]);
        let (_, actual) = session.clientbound(11, &input).unwrap().unwrap();
        let mut expected = vec![1, 0];
        for value in [1.0, 2.0, 3.0] {
            expected.write_f64(value);
        }
        for value in [0.25, 0.5, 0.75, 1.0] {
            expected.write_f32(value);
        }
        expected.write_i32(2);
        expected.push(target);
        assert_eq!(actual, expected, "geyser kind {kind}");
        let mut update = Vec::new();
        update.write_i64(123);
        update.write_varint(90);
        assert!(session.clientbound(9, &update).unwrap().is_some());
    }
}

#[test]
fn each_step_owns_its_login_delta_and_the_chain_applies_it_once() {
    let mut profile = vec![4; 20];
    profile.extend([8; 16]);
    for (peer, expected) in [(776, 36), (775, 20)] {
        let mut session = session(
            vec![
                measured_step(777, 776, 90, 40),
                measured_step(776, 775, 40, 7),
            ],
            777,
            peer,
        );
        let translated = session.clientbound(2, &profile).unwrap().unwrap();
        assert_eq!(translated.1.len(), expected, "peer {peer}");
        assert_eq!(&translated.1[..20], &[4; 20]);
        enter_play(&mut session);
        let mut update = Vec::new();
        update.write_i64(123);
        update.write_varint(90);
        let translated = session.clientbound(9, &update).unwrap().unwrap();
        let mut body = translated.1.as_slice();
        assert_eq!(body.read_i64().unwrap(), 123);
        assert_eq!(
            body.read_varint().unwrap(),
            if peer == 776 { 40 } else { 7 }
        );
        assert!(body.is_empty());
    }
}

#[test]
fn lower_step_keeps_long_array_light_masks_instead_of_translating_twice() {
    let mut session = session(
        vec![
            measured_step(777, 776, 90, 40),
            measured_step(776, 775, 40, 7),
        ],
        777,
        775,
    );
    enter_play(&mut session);
    let mut body = vec![0, 0]; // chunk x/z
    for mask in [[1, 0, 0, 0, 0, 0, 0, 1], [0; 8], [0; 8], [0; 8]] {
        body.write_byte_array(&mask).unwrap();
    }
    body.extend([0, 0]);
    let (_, translated) = session.clientbound(10, &body).unwrap().unwrap();
    let mut r = translated.as_slice();
    assert_eq!((r.read_varint().unwrap(), r.read_varint().unwrap()), (0, 0));
    for expected in [0x0100_0000_0000_0001, 0, 0, 0] {
        assert_eq!(r.read_varint().unwrap(), 1);
        assert_eq!(r.read_i64().unwrap(), expected);
    }
    assert_eq!(r, &[0, 0]);
}

/// Explicitly different codecs for the two directions: this cannot pass by
/// automatically reversing a lossy registry map. Uses a synthetic older core.
#[derive(Clone)]
struct Marker(u8);
impl StepFactory for Marker {
    fn session(&self) -> Box<dyn StepSession> {
        Box::new(self.clone())
    }
}
impl StepSession for Marker {
    fn clientbound(
        &mut self,
        direction: Direction,
        mut frame: RawFrame,
    ) -> Result<Option<RawFrame>> {
        let mut body = frame.body.to_vec();
        body.push(self.0 + if direction == Direction::Up { 10 } else { 0 });
        frame.body = body.into();
        Ok(Some(frame))
    }
    fn serverbound(
        &mut self,
        direction: Direction,
        mut frame: RawFrame,
    ) -> Result<Option<RawFrame>> {
        let mut body = frame.body.to_vec();
        body.push(self.0 + if direction == Direction::Up { 20 } else { 30 });
        frame.body = body.into();
        Ok(Some(frame))
    }
    fn split(&self) -> Box<dyn StepSession> {
        Box::new(self.clone())
    }
}

#[test]
fn newer_client_on_older_canonical_uses_up_codecs_and_reverse_serverbound_order() {
    let steps = || {
        vec![
            Step {
                upper: 776,
                lower: 775,
                factory: Box::new(Marker(1)),
            },
            Step {
                upper: 777,
                lower: 776,
                factory: Box::new(Marker(2)),
            },
        ]
    };
    let mut up = session(steps(), 775, 777);
    assert_eq!(up.clientbound(9, &[0]).unwrap().unwrap().1, [0, 11, 12]);
    assert_eq!(
        up.serverbound(frame(9, &[0]))
            .unwrap()
            .unwrap()
            .body
            .as_ref(),
        &[0, 22, 21]
    );
    let mut down = session(steps(), 777, 775);
    assert_eq!(down.clientbound(9, &[0]).unwrap().unwrap().1, [0, 2, 1]);
    assert_eq!(
        down.serverbound(frame(9, &[0]))
            .unwrap()
            .unwrap()
            .body
            .as_ref(),
        &[0, 31, 32]
    );
    assert_eq!(
        up.split().clientbound(9, &[0]).unwrap().unwrap().1,
        [0, 11, 12]
    );
}

#[test]
fn missing_step_has_no_nonadjacent_bypass() {
    let steps = vec![measured_step(777, 776, 9, 4)];
    assert!(
        route(&steps, 777, 775)
            .unwrap_err()
            .to_string()
            .contains("from 776 toward 775")
    );
    assert!(route(&steps, 777, 776).is_ok());
    assert!(route(&steps, 777, 777).unwrap().is_empty());
}

fn write_json(root: &Path, relative: &str, value: serde_json::Value) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}

fn endpoint(root: &Path, version: Version) {
    use serde_json::json;
    write_json(
        root,
        "version.json",
        json!({"id": version.release, "protocol_version": version.protocol}),
    );
    write_json(
        root,
        "reports/packets.json",
        serde_json::from_str(PACKETS).unwrap(),
    );
    write_json(
        root,
        "reports/blocks.json",
        json!({"minecraft:stone":{"states":[{"id":1,"default":true}]}}),
    );
    let mut registries = serde_json::Map::new();
    for name in [
        "block",
        "item",
        "entity_type",
        "block_entity_type",
        "sound_event",
        "particle_type",
        "data_component_type",
        "attribute",
        "command_argument_type",
        "slot_display",
        "recipe_display",
    ] {
        registries.insert(format!("minecraft:{name}"), json!({"entries":{"minecraft:stone":{"protocol_id":0},"minecraft:brick":{"protocol_id":1}}}));
    }
    if version.protocol == 777 {
        let entries: serde_json::Map<String, serde_json::Value> =
            mc_data::builtin_sounds_26_3::names()
                .iter()
                .enumerate()
                .map(|(id, name)| (name.clone(), json!({"protocol_id":id})))
                .collect();
        registries.insert("minecraft:sound_event".into(), json!({"entries":entries}));
    }
    write_json(root, "reports/registries.json", registries.into());
    for (registry, directory) in mc_data::KNOWN_REGISTRIES {
        write_json(
            root,
            &format!("data/minecraft/{directory}/sample.json"),
            json!({}),
        );
        let id = mc_data::Identifier::parse(format!("minecraft:{registry}")).unwrap();
        let entry = mc_data::Identifier::parse("minecraft:sample").unwrap();
        let path = mc_data::network_registry_payload_path(root, &id, &entry).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut payload = Vec::new();
        mc_nbt::write_network(&mut payload, &mc_nbt::Tag::Compound(Vec::new())).unwrap();
        std::fs::write(path, payload).unwrap();
    }
    write_json(
        root,
        "data/minecraft/tags/item/sample.json",
        json!({"values":["minecraft:brick"]}),
    );
}

#[test]
fn endpoint_loss_disables_only_that_version_but_step_loss_disables_crossing_routes() {
    let dir = tempfile::tempdir().unwrap();
    let canonical = dir.path().join("26.3");
    for version in SUPPORTED {
        endpoint(&cache_root(&canonical, "26.3", *version), *version);
    }
    for pair in SUPPORTED.windows(2) {
        snapshots::install(
            &cache_root(&canonical, "26.3", pair[1]),
            &cache_root(&canonical, "26.3", pair[0]),
            &step_root(&canonical, "26.3", pair[1], pair[0]),
            pair[1],
            pair[0],
        )
        .unwrap();
    }
    let data = mc_data::load(&canonical).unwrap();
    let catalog = Catalog::discover(&data);
    assert!(catalog.get(775).is_some() && catalog.get(776).is_some());
    std::fs::remove_dir_all(cache_root(&canonical, "26.3", SUPPORTED[1])).unwrap();
    let catalog = Catalog::discover(&data);
    assert!(
        catalog.get(775).is_some(),
        "775 needs the step snapshot, not the 776 endpoint cache"
    );
    assert!(catalog.get(776).is_none());
    endpoint(&cache_root(&canonical, "26.3", SUPPORTED[1]), SUPPORTED[1]);
    std::fs::remove_dir_all(step_root(&canonical, "26.3", SUPPORTED[1], SUPPORTED[0])).unwrap();
    let catalog = Catalog::discover(&data);
    assert!(catalog.get(775).is_none());
    assert!(catalog.get(776).is_some());
}

#[tokio::test]
async fn every_loaded_endpoint_is_selected_by_login_and_status_handshakes() {
    use crate::connection::Connection;
    use mc_protocol::packets::handshake::{Handshake, NextState};
    for version in SUPPORTED {
        for next_state in [NextState::Status, NextState::Login] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let stream = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
                .await
                .unwrap();
            let (accepted, _) = listener.accept().await.unwrap();
            let mut client = Connection::new(stream);
            let mut server = Connection::new(accepted);
            let selected = session(
                vec![
                    measured_step(777, 776, 90, 40),
                    measured_step(776, 775, 40, 7),
                ],
                777,
                version.protocol,
            );
            let catalog = Catalog {
                profiles: if version.protocol == 777 {
                    Vec::new()
                } else {
                    vec![Arc::clone(selected.profile())]
                },
            };
            client
                .send_packet(&Handshake {
                    protocol_version: version.protocol,
                    server_address: "localhost".into(),
                    server_port: 25565,
                    next_state,
                })
                .await
                .unwrap();
            let outcome = crate::handshake::read_handshake(&mut server, false, Some(&catalog))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                outcome.status_version.map(|value| value.protocol),
                (version.protocol != 777).then_some(version.protocol)
            );
            assert_eq!(
                server.profile().is_some(),
                next_state == NextState::Login && version.protocol != 777
            );
            if let Some(profile) = server.profile() {
                assert_eq!(profile.version, *version);
            }
        }
    }
}

#[test]
fn compressed_fanout_crosses_the_whole_chain_once() {
    let mut session = session(
        vec![
            measured_step(777, 776, 90, 40),
            measured_step(776, 775, 40, 7),
        ],
        777,
        775,
    );
    enter_play(&mut session);
    let compression = Compression::Threshold(1);
    let mut body = Vec::new();
    body.write_i64(123);
    body.write_varint(90);
    let batch = mc_protocol::frame::encode_frame(9, &body, compression).unwrap();
    let mut translated = session.clientbound_frames(&batch, compression).unwrap();
    let frame = mc_protocol::frame::try_decode_frame(&mut translated, compression)
        .unwrap()
        .unwrap();
    assert_eq!(frame.id, 9);
    let mut body = frame.body.as_ref();
    assert_eq!(body.read_i64().unwrap(), 123);
    assert_eq!(body.read_varint().unwrap(), 7);
    assert!(body.is_empty() && translated.is_empty());
}

#[test]
fn unchanged_bodies_and_login_uuid_trimming_retain_frame_storage_across_steps() {
    let mut session = session(
        vec![
            measured_step(777, 776, 90, 40),
            measured_step(776, 775, 40, 7),
        ],
        777,
        775,
    );
    let profile = Bytes::from_static(&[7; 36]);
    let pointer = profile.as_ptr();
    let translated = session
        .clientbound_frame(RawFrame {
            id: 2,
            body: profile,
        })
        .unwrap()
        .unwrap();
    assert_eq!(translated.body.len(), 20);
    assert_eq!(translated.body.as_ptr(), pointer);
    enter_play(&mut session);
    let keepalive = Bytes::from_static(&[8; 8]);
    let pointer = keepalive.as_ptr();
    let translated = session
        .serverbound(RawFrame {
            id: 5,
            body: keepalive,
        })
        .unwrap()
        .unwrap();
    assert_eq!(translated.body.as_ptr(), pointer);
    assert_eq!(translated.body.as_ref(), &[8; 8]);
}

#[test]
fn production_factory_upgrades_776_to_777_through_login_configuration_and_play() {
    use serde_json::json;
    let dir = tempfile::tempdir().unwrap();
    let upper = dir.path().join("777");
    let lower = dir.path().join("776");
    endpoint(&upper, SUPPORTED[2]);
    endpoint(&lower, SUPPORTED[1]);
    write_json(
        &lower,
        "reports/blocks.json",
        json!({"minecraft:stone":{"states":[{"id":7,"default":true}]}}),
    );
    write_json(
        &upper,
        "reports/blocks.json",
        json!({"minecraft:stone":{"states":[{"id":1,"default":true}]},"minecraft:cinnabar":{"states":[{"id":9,"default":true}]}}),
    );
    let mut registries: serde_json::Value =
        serde_json::from_slice(&std::fs::read(upper.join("reports/registries.json")).unwrap())
            .unwrap();
    registries["minecraft:item"]["entries"]["minecraft:cinnabar"] = json!({"protocol_id":2});
    write_json(&upper, "reports/registries.json", registries);
    let snapshot = dir.path().join("777-776");
    snapshots::install(&upper, &lower, &snapshot, SUPPORTED[2], SUPPORTED[1]).unwrap();
    let production = Arc::new(adjacent::Profile::load(&snapshot, 777, 776).unwrap());
    let mut session = session(
        vec![Step {
            upper: 777,
            lower: 776,
            factory: Box::new(production),
        }],
        776,
        777,
    );
    assert_eq!(
        session.clientbound(2, &[4; 36]).unwrap().unwrap().1,
        [4; 36]
    );
    session.serverbound(frame(3, &[])).unwrap().unwrap();
    assert_eq!(
        session.clientbound(7, &[1, 2, 3]).unwrap().unwrap().1,
        [1, 2, 3]
    );
    session.clientbound(3, &[]).unwrap().unwrap();
    session.serverbound(frame(3, &[])).unwrap().unwrap();

    let mut prefix = Vec::new();
    prefix.write_i32(7);
    prefix.write_bool(false);
    prefix.write_varint(0);
    for value in [20, 10, 6] {
        prefix.write_varint(value);
    }
    prefix.extend([0, 1, 0]);
    prefix.write_varint(0);
    prefix.write_string("minecraft:overworld", 32767).unwrap();
    prefix.write_i64(42);
    let mut old_login = prefix.clone();
    old_login.extend([1, 255, 0, 0, 0, 0, 63, 0, 0]);
    let mut new_login = prefix;
    new_login.extend([1, 0, 0, 0, 0, 0, 63, 0, 0]);
    assert_eq!(
        session.clientbound(1, &old_login).unwrap().unwrap().1,
        new_login
    );
    let mut update = vec![0; 8];
    update.write_varint(7);
    let translated = session.clientbound(9, &update).unwrap().unwrap().1;
    assert_eq!(translated, [0, 0, 0, 0, 0, 0, 0, 0, 1]);

    let mut light = vec![0, 0, 1];
    light.write_i64(1);
    light.extend([0, 0, 0, 0, 0]);
    assert_eq!(
        session.clientbound(10, &light).unwrap().unwrap().1,
        [0, 0, 1, 1, 0, 0, 0, 0, 0]
    );
    let mut teleport = vec![11];
    for _ in 0..3 {
        teleport.write_f64(0.0);
    }
    for _ in 0..2 {
        teleport.write_f32(0.0);
    }
    assert_eq!(
        session
            .serverbound(frame(45, &teleport))
            .unwrap()
            .unwrap()
            .body
            .as_ref(),
        [11]
    );
    let mut unavailable_item = vec![0, 36, 1, 2, 0, 0];
    let error = session
        .serverbound(frame(57, &unavailable_item))
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("unmapped item id"),
        "a 777-only cinnabar must never become the 776 stone polyfill"
    );
    unavailable_item[3] = 0;
    assert_eq!(
        session
            .serverbound(frame(57, &unavailable_item))
            .unwrap()
            .unwrap()
            .body
            .as_ref(),
        unavailable_item
    );
}
