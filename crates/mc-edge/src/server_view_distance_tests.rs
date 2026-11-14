use super::*;
use mc_core::boundary::{EntityId, SnapshotChunk, WorldSnapshot};
use mc_protocol::frame::{Compression, RawFrame, try_decode_frame};
use mc_protocol::packets::{
    ChatVisibility, ClientInformation, MainHand, Packet, ParticleStatus, play::*,
};

fn server(root: &std::path::Path) -> Arc<EdgeServer> {
    let mut config = mc_core::CoreConfig::new(root.join("world"), 42);
    config.vanilla_data_dir =
        Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla"));
    let hub = Arc::new(Core::open(config).unwrap());
    let mut cfg = EdgeConfig {
        server_dir: root.join("server"),
        ..Default::default()
    };
    cfg.server.view_distance = 16;
    cfg.auth.online_mode = false;
    std::fs::create_dir_all(&cfg.server_dir).unwrap();
    Arc::new(
        EdgeServer::new(
            hub,
            Arc::new(mc_data::krasnokamen_required_data()),
            Arc::default(),
            cfg,
        )
        .unwrap(),
    )
}

fn information(distance: i8) -> Vec<u8> {
    information_language("en_us", distance)
}

fn information_language(language: &str, distance: i8) -> Vec<u8> {
    information_chat(language, distance, ChatVisibility::Full)
}

fn information_chat(language: &str, distance: i8, chat_visibility: ChatVisibility) -> Vec<u8> {
    let mut body = Vec::new();
    ServerboundClientInformation {
        information: ClientInformation {
            language: language.into(),
            view_distance: distance,
            chat_visibility,
            chat_colors: true,
            model_customisation: 0x7f,
            main_hand: MainHand::Right,
            text_filtering_enabled: false,
            allows_listing: true,
            particle_status: ParticleStatus::All,
        },
    }
    .encode(&mut body)
    .unwrap();
    body
}

fn drain(rx: &mut mpsc::Receiver<bytes::Bytes>) -> Vec<RawFrame> {
    let mut frames = Vec::new();
    while let Ok(data) = rx.try_recv() {
        frames.push(
            try_decode_frame(
                &mut bytes::BytesMut::from(data.as_ref()),
                Compression::Threshold(256),
            )
            .unwrap()
            .unwrap(),
        );
    }
    frames
}

fn radii(frames: &[RawFrame]) -> Vec<i32> {
    frames
        .iter()
        .filter(|frame| frame.id == SetChunkCacheRadius::ID)
        .map(|frame| {
            SetChunkCacheRadius::decode(&mut &frame.body[..])
                .unwrap()
                .radius
        })
        .collect()
}

#[tokio::test]
async fn client_view_distance_join_grow_and_shrink() {
    let root = tempfile::tempdir().unwrap();
    let server = server(root.path());
    let id = server.connections.joining_id();
    let entity = EntityId(1);
    let spawn = [8.0, 64.0, 8.0];
    server
        .connections
        .stash_join(id, (entity, spawn, 0, Default::default()));
    wait_for_join(
        &server,
        id,
        "View",
        4,
        "en_us",
        mc_core::boundary::ChatVisibility::Full,
    )
    .await
    .unwrap();
    let (reliable_tx, mut reliable_rx) = mpsc::channel(512);
    let (world_tx, mut world_rx) = mpsc::channel(512);
    server
        .connections
        .register(
            id,
            ConnectionHandles {
                reliable_tx,
                world_tx,
            },
        )
        .unwrap();
    let mut connections = server.connections.snapshot().into_iter().collect();
    let mut viewers = HashMap::new();
    route_outbox(&server, &mut viewers, &mut connections, None);
    let chunks = (-8..=8)
        .flat_map(|x| (-8..=8).map(move |z| (x, z)))
        .map(|(x, z)| {
            let pos = mc_world::chunk::ChunkPos { x, z };
            let mut chunk = mc_world::chunk::Chunk::empty(
                pos,
                mc_world::BlockStateId(0),
                mc_data::Identifier::parse("minecraft:plains").unwrap(),
            );
            chunk.section_lights.fill(mc_world::chunk::SectionLight {
                block: Some(mc_world::chunk::LightSection::uniform(0)),
                sky: Some(mc_world::chunk::LightSection::uniform(0xff)),
            });
            SnapshotChunk {
                pos,
                revision: 1,
                chunk: Arc::new(chunk),
            }
        })
        .collect();
    let output = TickOutput {
        tick: 1,
        snapshot: Arc::new(WorldSnapshot {
            tick: 1,
            chunks: Arc::new(chunks),
            day_time_rate: 1.0,
            ..Default::default()
        }),
        changes: Arc::default(),
        times: Default::default(),
    };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let mut cache = crate::plan::ChunkPacketCache::new();
    let mut held = std::collections::HashSet::new();
    for (requested, expected) in [(4, 4), (8, 8), (2, 2), (127, 16), (-1, 2)] {
        if requested != 4 {
            assert!(
                server
                    .handle_edge_frame(
                        id,
                        ServerboundClientInformation::ID,
                        &information(requested)
                    )
                    .await
                    .unwrap(),
                "Play ClientInformation must update connection state"
            );
        }
        let mut radius_packets = Vec::new();
        // More than enough publishes for the bounded 16-chunk admission budget.
        for _ in 0..20 {
            publish_and_route(
                &server,
                &pool,
                &mut viewers,
                &output,
                &mut cache,
                &mut connections,
            )
            .await;
            radius_packets.extend(radii(&drain(&mut reliable_rx)));
            for frame in drain(&mut world_rx) {
                if frame.id == LevelChunkWithLight::ID {
                    let packet = LevelChunkWithLight::decode(&mut &frame.body[..]).unwrap();
                    assert!(
                        packet.chunk_x.abs() <= expected && packet.chunk_z.abs() <= expected,
                        "requested {requested}, got chunk ({}, {})",
                        packet.chunk_x,
                        packet.chunk_z
                    );
                    held.insert((packet.chunk_x, packet.chunk_z));
                } else if frame.id == ForgetLevelChunk::ID {
                    let packet = ForgetLevelChunk::decode(&mut &frame.body[..]).unwrap();
                    held.remove(&(packet.chunk_x, packet.chunk_z));
                }
            }
        }
        assert_eq!(
            radius_packets,
            [expected],
            "cache radius must change without moving the center"
        );
        let extent = expected.min(8);
        let wanted = (-extent..=extent)
            .flat_map(|x| (-extent..=extent).map(move |z| (x, z)))
            .collect();
        assert_eq!(
            held, wanted,
            "wire-held chunks reconcile to negotiated view"
        );
        assert_eq!(viewers[&id].loaded_chunks.len(), held.len());
    }
    // A duplicate preference must not resend radius or chunks.
    assert!(
        server
            .handle_edge_frame(id, ServerboundClientInformation::ID, &information(-1))
            .await
            .unwrap()
    );
    publish_and_route(
        &server,
        &pool,
        &mut viewers,
        &output,
        &mut cache,
        &mut connections,
    )
    .await;
    assert!(radii(&drain(&mut reliable_rx)).is_empty());
    assert!(
        drain(&mut world_rx)
            .iter()
            .all(|frame| frame.id != LevelChunkWithLight::ID)
    );
    server.connections.remove(id);
    assert!(
        !server
            .connections
            .inner
            .lock()
            .view_distances
            .contains_key(&id)
    );
    server.hub.shutdown();
}

/// The vanilla play-phase `Client Information` is where a client can change
/// language mid-session: the edge admits it as `PlayerInput::SetLocale` and
/// deduplicates an unchanged locale.
#[tokio::test]
async fn play_client_information_admits_the_client_locale() {
    let root = tempfile::tempdir().unwrap();
    let server = server(root.path());
    let id = server.connections.joining_id();
    server
        .connections
        .stash_join(id, (EntityId(1), [8.0, 64.0, 8.0], 0, Default::default()));
    wait_for_join(
        &server,
        id,
        "Locale",
        4,
        "en_us",
        mc_core::boundary::ChatVisibility::Full,
    )
    .await
    .unwrap();
    assert_eq!(locale(&server, id), Some("en_us".to_owned()));

    assert!(
        server
            .handle_edge_frame(
                id,
                ServerboundClientInformation::ID,
                &information_language("ru_ru", 4)
            )
            .await
            .unwrap()
    );
    assert_eq!(locale(&server, id), Some("ru_ru".to_owned()));

    // A repeated locale is not admitted again and does not disturb the radius.
    assert!(
        server
            .handle_edge_frame(
                id,
                ServerboundClientInformation::ID,
                &information_language("ru_ru", 4)
            )
            .await
            .unwrap()
    );
    assert_eq!(locale(&server, id), Some("ru_ru".to_owned()));

    server.connections.remove(id);
    assert_eq!(locale(&server, id), None);
    server.hub.shutdown();
}

fn locale(server: &EdgeServer, id: ConnectionId) -> Option<String> {
    server.connections.inner.lock().locales.get(&id).cloned()
}

fn chat_setting(
    server: &EdgeServer,
    id: ConnectionId,
) -> Option<mc_core::boundary::ChatVisibility> {
    server
        .connections
        .inner
        .lock()
        .chat_visibilities
        .get(&id)
        .copied()
}

/// The chat setting is admitted to the core with the configuration-phase
/// value before Join, then follows every Play `Client Information` change.
#[tokio::test]
async fn client_chat_setting_is_admitted_at_join_and_on_play_changes() {
    use mc_core::boundary::ChatVisibility as Setting;
    let root = tempfile::tempdir().unwrap();
    let server = server(root.path());
    let id = server.connections.joining_id();
    server
        .connections
        .stash_join(id, (EntityId(1), [8.0, 64.0, 8.0], 0, Default::default()));
    wait_for_join(&server, id, "Chat", 4, "en_us", Setting::System)
        .await
        .unwrap();
    assert_eq!(chat_setting(&server, id), Some(Setting::System));

    for (wire, expected) in [
        (ChatVisibility::Hidden, Setting::Hidden),
        (ChatVisibility::Full, Setting::Full),
        (ChatVisibility::System, Setting::System),
    ] {
        assert!(
            server
                .handle_edge_frame(
                    id,
                    ServerboundClientInformation::ID,
                    &information_chat("en_us", 4, wire)
                )
                .await
                .unwrap()
        );
        assert_eq!(chat_setting(&server, id), Some(expected));
    }

    server.connections.remove(id);
    assert_eq!(chat_setting(&server, id), None);
    server.hub.shutdown();
}

/// Text the edge writes itself (join tip, /help, unknown command) obeys the
/// client's chat setting like the core's: Hidden gets no text line at all.
#[tokio::test]
async fn edge_sent_feedback_follows_the_clients_chat_setting() {
    use mc_core::boundary::ChatVisibility as Setting;
    let root = tempfile::tempdir().unwrap();
    let server = server(root.path());
    let id = server.connections.joining_id();
    server
        .connections
        .stash_join(id, (EntityId(1), [8.0, 64.0, 8.0], 0, Default::default()));
    wait_for_join(&server, id, "Chat", 4, "en_us", Setting::Full)
        .await
        .unwrap();
    let (reliable_tx, mut reliable_rx) = mpsc::channel(16);
    let (world_tx, _world_rx) = mpsc::channel(16);
    server
        .connections
        .register(
            id,
            ConnectionHandles {
                reliable_tx,
                world_tx,
            },
        )
        .unwrap();
    for (wire, lines) in [
        (ChatVisibility::Full, 1),
        (ChatVisibility::System, 1),
        (ChatVisibility::Hidden, 0),
        (ChatVisibility::Full, 1),
    ] {
        assert!(
            server
                .handle_edge_frame(
                    id,
                    ServerboundClientInformation::ID,
                    &information_chat("en_us", 4, wire)
                )
                .await
                .unwrap()
        );
        drain(&mut reliable_rx);
        server
            .feedback(id, "Unknown or incomplete command")
            .unwrap();
        let frames = drain(&mut reliable_rx);
        let texts = frames
            .iter()
            .filter(|frame| frame.id == ClientboundSystemChat::ID)
            .count();
        assert_eq!(texts, lines, "{wire:?}");
    }
    server.connections.remove(id);
    server.hub.shutdown();
}

/// `/list` is answered by the connection layer, but its success reply obeys
/// `send_command_feedback` like every other command's (vanilla `sendSuccess`).
#[tokio::test]
async fn list_obeys_the_send_command_feedback_rule() {
    use mc_core::boundary::ChatVisibility as Setting;
    let root = tempfile::tempdir().unwrap();
    let server = server(root.path());
    let id = server.connections.joining_id();
    server
        .connections
        .stash_join(id, (EntityId(1), [8.0, 64.0, 8.0], 0, Default::default()));
    wait_for_join(&server, id, "Lister", 4, "en_us", Setting::Full)
        .await
        .unwrap();
    let (reliable_tx, mut reliable_rx) = mpsc::channel(16);
    let (world_tx, _world_rx) = mpsc::channel(16);
    server
        .connections
        .register(
            id,
            ConnectionHandles {
                reliable_tx,
                world_tx,
            },
        )
        .unwrap();
    for (rule, lines) in [("false", 0), ("true", 1)] {
        let mut ticks = server.hub.subscribe();
        let (command, completed) = mc_core::command_console::ConsoleCommand::new(format!(
            "gamerule send_command_feedback {rule}"
        ));
        assert!(
            server
                .hub
                .submit(mc_core::boundary::PlayerInput::ConsoleCommand(command))
        );
        // The console acknowledges the submission; the command runs by the end of the next tick.
        let submitted = completed.await.unwrap();
        loop {
            match ticks.recv().await {
                Ok(output) if output.tick >= submitted => break,
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(error) => panic!("the tick thread stopped: {error}"),
            }
        }
        assert_eq!(
            server.hub.command_feedback_enabled(),
            rule == "true",
            "the core published the rule after the console command"
        );
        drain(&mut reliable_rx);
        for command in ["list", "list uuids", "help"] {
            assert!(server.handle_admin_command(id, command).await.unwrap());
            let shown = drain(&mut reliable_rx)
                .iter()
                .filter(|frame| frame.id == ClientboundSystemChat::ID)
                .count();
            assert_eq!(shown, lines, "{command} with send_command_feedback {rule}");
        }
    }
    server.connections.remove(id);
    server.hub.shutdown();
}

#[test]
fn the_wire_chat_setting_maps_to_the_core_setting() {
    use mc_core::boundary::ChatVisibility as Setting;
    for (wire, expected) in [
        (ChatVisibility::Full, Setting::Full),
        (ChatVisibility::System, Setting::System),
        (ChatVisibility::Hidden, Setting::Hidden),
    ] {
        assert_eq!(crate::configuration::chat_visibility(wire), expected);
    }
}
