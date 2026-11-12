//! Tests for [`crate::configuration`]: pack advertisement, known-packs
//! negotiation, registry/tag/brand/finish ordering, and ignore tolerance.

use mc_protocol::TARGET_RELEASE;
use mc_protocol::packets::Packet as _;
use mc_protocol::packets::configuration::{
    AcknowledgeFinishConfiguration, ClientboundKnownPacks, FinishConfiguration,
    ServerboundClientInformation, ServerboundCustomPayload, ServerboundKnownPacks,
};
use mc_protocol::packets::{ChatVisibility, ClientInformation, MainHand, ParticleStatus};
use tokio::net::TcpStream;

use crate::connection::Connection;

async fn loopback_pair() -> (Connection, Connection) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let client = TcpStream::connect(addr).await.unwrap();
    let (server, _) = listener.accept().await.unwrap();
    (Connection::new(client), Connection::new(server))
}

fn information(view_distance: i8) -> ClientInformation {
    information_locale("en_us", view_distance)
}

fn information_locale(language: &str, view_distance: i8) -> ClientInformation {
    ClientInformation {
        language: language.to_owned(),
        view_distance,
        chat_visibility: ChatVisibility::Full,
        chat_colors: true,
        model_customisation: 0,
        main_hand: MainHand::Right,
        text_filtering_enabled: false,
        allows_listing: true,
        particle_status: ParticleStatus::All,
    }
}

fn test_data() -> mc_data::VanillaData {
    mc_data::testing::stub()
}

fn test_tags() -> mc_data::tags::TagsData {
    mc_data::tags::TagsData::default()
}

#[tokio::test]
async fn vanilla_configuration_completes_with_only_native_optional_bundle() {
    let (mut client, mut server) = loopback_pair().await;
    let data = test_data();
    let registry_count = data.registry_count();
    let native_optional = crate::server::loader_manifest(&[]).unwrap();
    let server_task = tokio::spawn(async move {
        let tags = test_tags();
        super::handle_configuration(
            &mut server,
            &data,
            &tags,
            native_optional.as_ref(),
            false,
            &Default::default(),
        )
        .await
        .unwrap()
    });

    // UpdateEnabledFeatures.
    let _ = client
        .read_frame_in(mc_protocol::State::Configuration)
        .await
        .unwrap();
    read_channel_register(&mut client).await;
    // The optional-only manifest is probed before known-pack negotiation.
    let probe = client
        .read_frame_in(mc_protocol::State::Configuration)
        .await
        .unwrap();
    assert_eq!(
        probe.id,
        mc_protocol::packets::configuration::ClientboundCustomPayload::ID
    );
    // ClientboundKnownPacks: must advertise minecraft:core:<release>.
    let frame = client
        .read_frame_in(mc_protocol::State::Configuration)
        .await
        .unwrap();
    assert_eq!(frame.id, ClientboundKnownPacks::ID);
    let mut body = frame.body.clone();
    let packs = ClientboundKnownPacks::decode(&mut body).expect("known packs decode");
    assert!(
        packs.packs.iter().any(|p| {
            p.namespace == "minecraft" && p.id == "core" && p.version == TARGET_RELEASE
        })
    );

    // Tolerated packet before KnownPacks: client information.
    client
        .send_packet(&ServerboundClientInformation {
            information: information(6),
        })
        .await
        .unwrap();
    client
        .send_packet(&ServerboundKnownPacks {
            packs: packs.packs.clone(),
        })
        .await
        .unwrap();

    // Drain RegistryData + UpdateTags + Brand + FinishConfiguration.
    let mut saw_finish = false;
    for _ in 0..(registry_count + 8) {
        let frame = client
            .read_frame_in(mc_protocol::State::Configuration)
            .await
            .unwrap();
        if frame.id == FinishConfiguration::ID {
            saw_finish = true;
            break;
        }
    }
    assert!(saw_finish, "server must send FinishConfiguration");

    // Tolerated packet while waiting for ack, then ack.
    client
        .send_packet(&ServerboundClientInformation {
            information: ClientInformation {
                chat_visibility: ChatVisibility::Hidden,
                ..information_locale("ru_ru", 9)
            },
        })
        .await
        .unwrap();
    client
        .send_packet(&AcknowledgeFinishConfiguration)
        .await
        .unwrap();

    let outcome = server_task.await.unwrap();
    assert_eq!(outcome.client_view_distance, 9);
    // The latest `Client Information` also carries the locale for dialogs.
    assert_eq!(outcome.client_language, "ru_ru");
    assert_eq!(
        outcome.client_chat_visibility,
        mc_core::boundary::ChatVisibility::Hidden,
        "and the chat setting the core filters text by"
    );
}

#[test]
fn client_preferences_default_to_english_and_unknown_packets_change_nothing() {
    let mut prefs = super::ClientPrefs::default();
    assert_eq!(prefs.language, "");
    assert_eq!(prefs.view_distance, 8);
    assert!(
        super::note_ignored(&mut prefs, 0x7fff, bytes::Bytes::new()).unwrap(),
        "unknown packets are tolerated"
    );
    assert_eq!(prefs.language, "");
}

#[tokio::test]
async fn missing_pack_without_sidecar_payloads_fails() {
    let (mut client, mut server) = loopback_pair().await;
    // `testing::stub()` has no sidecar payloads, so a client that reports
    // no packs must be rejected.
    let data = test_data();
    assert!(!data.has_full_registry_payloads());
    let server_task = tokio::spawn(async move {
        let tags = test_tags();
        super::handle_configuration(&mut server, &data, &tags, None, false, &Default::default())
            .await
    });
    // Skip features + known packs advertisements.
    for _ in 0..2 {
        let _ = client
            .read_frame_in(mc_protocol::State::Configuration)
            .await
            .unwrap();
    }
    client
        .send_packet(&ServerboundKnownPacks { packs: Vec::new() })
        .await
        .unwrap();
    let result = server_task.await.unwrap();
    assert!(result.is_err());
}

/// One verified optional/required bundle (an items-only index) as the discovery
/// path would hand it to the Loader manifest.
fn items_bundle(required: bool) -> mc_loader::LoaderManifest {
    use std::io::Write as _;
    let index = br#"{"schema":2,"items":[],"assets":[]}"#;
    let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    archive
        .start_file(
            "solaris-client.json",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    archive.write_all(index).unwrap();
    let bytes = archive.finish().unwrap().into_inner();
    let sha256 = format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(&bytes));
    let bundle = mc_script::ClientBundle::new(
        "chisel",
        "chisel-content",
        "1",
        "client/chisel-content.zip",
        sha256,
        bytes.len() as u64,
        "client/chisel-content.zip".into(),
        std::sync::Arc::from(bytes),
        vec![mc_script::ClientLoader::Native],
        vec![mc_script::ClientContentKind::Items],
        vec![mc_script::ClientPermission::RegisterItems],
    )
    .with_required(required);
    assert_eq!(bundle.required(), required);
    mc_loader::LoaderManifest::from_script_bundles(&[bundle]).unwrap()
}

/// Reads server frames until FinishConfiguration, returning every custom
/// payload channel the server sent.
async fn channels_until_finish(client: &mut Connection) -> Vec<String> {
    let mut channels = Vec::new();
    loop {
        let frame = client
            .read_frame_in(mc_protocol::State::Configuration)
            .await
            .unwrap();
        if frame.id == FinishConfiguration::ID {
            return channels;
        }
        if frame.id == mc_protocol::packets::configuration::ClientboundCustomPayload::ID {
            let mut body = frame.body.clone();
            if let mc_protocol::packets::CustomPayload::Unknown { channel, .. } =
                mc_protocol::packets::CustomPayload::decode(&mut body).unwrap()
            {
                channels.push(channel.to_string());
            }
        }
    }
}

/// Reads the `minecraft:register` frame that declares every serverbound Loader
/// channel: NeoForge clients send a custom payload only on a declared channel.
async fn read_channel_register(client: &mut Connection) {
    let register = client
        .read_frame_in(mc_protocol::State::Configuration)
        .await
        .unwrap();
    match mc_protocol::packets::CustomPayload::decode(&mut register.body.clone()).unwrap() {
        mc_protocol::packets::CustomPayload::Unknown { channel, payload } => {
            assert_eq!(channel.to_string(), "minecraft:register");
            let declared: Vec<&[u8]> = payload.split(|byte| *byte == 0).collect();
            for serverbound in [
                "solaris:loader/hello",
                "solaris:loader/request",
                "solaris:loader/ack",
                "solaris:loader/view_action",
            ] {
                assert!(
                    declared.contains(&serverbound.as_bytes()),
                    "{serverbound} is declared: {:?}",
                    String::from_utf8_lossy(&payload)
                );
            }
        }
        other => panic!("expected minecraft:register, got {other:?}"),
    }
}

#[tokio::test]
async fn vanilla_client_completes_configuration_with_only_optional_bundles() {
    let (mut client, mut server) = loopback_pair().await;
    let data = test_data();
    let manifest = items_bundle(false);
    let server_task = tokio::spawn(async move {
        let tags = test_tags();
        super::handle_configuration(
            &mut server,
            &data,
            &tags,
            Some(&manifest),
            false,
            &Default::default(),
        )
        .await
        .unwrap()
    });
    let _features = client
        .read_frame_in(mc_protocol::State::Configuration)
        .await
        .unwrap();
    read_channel_register(&mut client).await;
    // The probe precedes the known-pack request.
    let probe = client
        .read_frame_in(mc_protocol::State::Configuration)
        .await
        .unwrap();
    assert_eq!(
        probe.id,
        mc_protocol::packets::configuration::ClientboundCustomPayload::ID
    );
    let packs = client
        .read_frame_in(mc_protocol::State::Configuration)
        .await
        .unwrap();
    assert_eq!(packs.id, ClientboundKnownPacks::ID);
    let packs = ClientboundKnownPacks::decode(&mut packs.body.clone()).unwrap();
    // A vanilla client ignores the unknown payload and just answers known packs.
    client
        .send_packet(&ServerboundKnownPacks { packs: packs.packs })
        .await
        .unwrap();
    let channels = channels_until_finish(&mut client).await;
    assert!(
        !channels
            .iter()
            .any(|channel| channel == "solaris:loader/manifest"),
        "a client that never answered the probe is never sent the manifest: {channels:?}"
    );
    client
        .send_packet(&AcknowledgeFinishConfiguration)
        .await
        .unwrap();
    let outcome = server_task.await.unwrap();
    assert!(outcome.loader_session.is_none());
}

#[tokio::test]
async fn loader_hello_before_known_packs_starts_negotiation_of_optional_bundles() {
    let (mut client, mut server) = loopback_pair().await;
    let data = test_data();
    let manifest = items_bundle(false);
    let server_task = tokio::spawn(async move {
        let tags = test_tags();
        // The client below never acknowledges the manifest; the task is dropped.
        let _ = super::handle_configuration(
            &mut server,
            &data,
            &tags,
            Some(&manifest),
            false,
            &Default::default(),
        )
        .await;
    });
    let _features = client
        .read_frame_in(mc_protocol::State::Configuration)
        .await
        .unwrap();
    read_channel_register(&mut client).await;
    let probe = client
        .read_frame_in(mc_protocol::State::Configuration)
        .await
        .unwrap();
    let mut body = probe.body.clone();
    match mc_protocol::packets::CustomPayload::decode(&mut body).unwrap() {
        mc_protocol::packets::CustomPayload::Unknown { channel, payload } => {
            assert_eq!(channel, *mc_loader::loader_probe_channel());
            assert!(payload.is_empty());
        }
        other => panic!("expected the probe payload, got {other:?}"),
    }
    let packs = client
        .read_frame_in(mc_protocol::State::Configuration)
        .await
        .unwrap();
    let packs = ClientboundKnownPacks::decode(&mut packs.body.clone()).unwrap();
    client
        .send_packet(
            &mc_protocol::packets::configuration::ServerboundCustomPayload {
                payload: mc_protocol::packets::CustomPayload::Unknown {
                    channel: mc_loader::loader_hello_channel().clone(),
                    payload: Vec::new(),
                },
            },
        )
        .await
        .unwrap();
    client
        .send_packet(&ServerboundKnownPacks { packs: packs.packs })
        .await
        .unwrap();
    // The first custom payload after registries/tags/brand is the manifest.
    let mut manifest_seen = false;
    for _ in 0..64 {
        let frame = client
            .read_frame_in(mc_protocol::State::Configuration)
            .await
            .unwrap();
        if frame.id == mc_protocol::packets::configuration::ClientboundCustomPayload::ID
            && let mc_protocol::packets::CustomPayload::Unknown { channel, .. } =
                mc_protocol::packets::CustomPayload::decode(&mut frame.body.clone()).unwrap()
            && channel == *mc_loader::loader_manifest_channel()
        {
            manifest_seen = true;
            break;
        }
    }
    assert!(
        manifest_seen,
        "a Loader client that said hello receives the manifest"
    );
    drop(client);
    let _ = server_task.await;
}

#[tokio::test]
async fn required_bundles_probe_and_negotiate_with_every_client() {
    let (mut client, mut server) = loopback_pair().await;
    let data = test_data();
    let manifest = items_bundle(true);
    let server_task = tokio::spawn(async move {
        let tags = test_tags();
        let _ = super::handle_configuration(
            &mut server,
            &data,
            &tags,
            Some(&manifest),
            true,
            &Default::default(),
        )
        .await;
    });
    let _features = client
        .read_frame_in(mc_protocol::State::Configuration)
        .await
        .unwrap();
    read_channel_register(&mut client).await;
    let next = client
        .read_frame_in(mc_protocol::State::Configuration)
        .await
        .unwrap();
    assert_eq!(
        next.id,
        mc_protocol::packets::configuration::ClientboundCustomPayload::ID,
        "required content probes too: a vanilla client is told apart from a slow Loader client"
    );
    let known = client
        .read_frame_in(mc_protocol::State::Configuration)
        .await
        .unwrap();
    assert_eq!(known.id, ClientboundKnownPacks::ID);
    drop(client);
    let _ = server_task.await;
}

#[tokio::test]
async fn serverops_required_pack_is_loaded_before_finish_configuration() {
    use mc_protocol::packets::configuration::ServerboundResourcePack;
    use mc_protocol::packets::resource_pack::ResourcePackPush;
    use mc_protocol::packets::{ResourcePackAction, ResourcePackStatus};
    let (mut client, mut server) = loopback_pair().await;
    let task = tokio::spawn(async move {
        let pack = crate::config::ResourcePackSection {
            url: "https://example.org/pack.zip".into(),
            required: true,
            ..Default::default()
        };
        super::handle_configuration(&mut server, &test_data(), &test_tags(), None, false, &pack)
            .await
    });
    let _ = client
        .read_frame_in(mc_protocol::State::Configuration)
        .await
        .unwrap();
    let frame = client
        .read_frame_in(mc_protocol::State::Configuration)
        .await
        .unwrap();
    let packs = ClientboundKnownPacks::decode(&mut frame.body.clone()).unwrap();
    client
        .send_packet(&ServerboundKnownPacks { packs: packs.packs })
        .await
        .unwrap();
    let pack = loop {
        let frame = client
            .read_frame_in(mc_protocol::State::Configuration)
            .await
            .unwrap();
        assert_ne!(frame.id, FinishConfiguration::ID);
        if frame.id == ResourcePackPush::ID {
            break ResourcePackPush::decode(&mut frame.body.clone()).unwrap();
        }
    };
    assert!(pack.required);
    for action in [
        ResourcePackAction::Accepted,
        ResourcePackAction::Downloaded,
        ResourcePackAction::SuccessfullyLoaded,
    ] {
        client
            .send_packet(&ServerboundResourcePack {
                status: ResourcePackStatus {
                    id: pack.id,
                    action,
                },
            })
            .await
            .unwrap();
    }
    let frame = client
        .read_frame_in(mc_protocol::State::Configuration)
        .await
        .unwrap();
    assert_eq!(frame.id, FinishConfiguration::ID);
    client
        .send_packet(&AcknowledgeFinishConfiguration)
        .await
        .unwrap();
    assert!(task.await.unwrap().is_ok());
}

#[tokio::test]
async fn optional_pack_decline_completes_configuration() {
    use mc_protocol::packets::configuration::ServerboundResourcePack;
    use mc_protocol::packets::resource_pack::ResourcePackPush;
    use mc_protocol::packets::{ResourcePackAction, ResourcePackStatus};

    let (mut client, mut server) = loopback_pair().await;
    let task = tokio::spawn(async move {
        let pack = crate::config::ResourcePackSection {
            url: "https://example.org/pack.zip".into(),
            required: false,
            ..Default::default()
        };
        super::handle_configuration(&mut server, &test_data(), &test_tags(), None, false, &pack)
            .await
    });
    client
        .read_frame_in(mc_protocol::State::Configuration)
        .await
        .unwrap();
    let frame = client
        .read_frame_in(mc_protocol::State::Configuration)
        .await
        .unwrap();
    let packs = ClientboundKnownPacks::decode(&mut frame.body.clone()).unwrap();
    client
        .send_packet(&ServerboundKnownPacks { packs: packs.packs })
        .await
        .unwrap();
    let pack = loop {
        let frame = client
            .read_frame_in(mc_protocol::State::Configuration)
            .await
            .unwrap();
        assert_ne!(frame.id, FinishConfiguration::ID);
        if frame.id == ResourcePackPush::ID {
            break ResourcePackPush::decode(&mut frame.body.clone()).unwrap();
        }
    };
    assert!(!pack.required);
    client
        .send_packet(&ServerboundResourcePack {
            status: ResourcePackStatus {
                id: pack.id,
                action: ResourcePackAction::Declined,
            },
        })
        .await
        .unwrap();
    let frame = client
        .read_frame_in(mc_protocol::State::Configuration)
        .await
        .unwrap();
    assert_eq!(frame.id, FinishConfiguration::ID);
    client
        .send_packet(&AcknowledgeFinishConfiguration)
        .await
        .unwrap();
    assert!(task.await.unwrap().is_ok());
}

fn loader_ack(manifest: &mc_loader::LoaderManifest) -> ServerboundCustomPayload {
    let ack = mc_loader::LoaderClientAck {
        protocol: mc_loader::LOADER_PROTOCOL_VERSION,
        platform: mc_loader::LoaderPlatform::Native,
        loader_version: "test".to_owned(),
        accepted_permissions: manifest.bundles[0].permissions.clone(),
        cached_bundles: vec![manifest.bundles[0].cache_key.clone()],
        carrier_block_state_ids: Default::default(),
    };
    ServerboundCustomPayload {
        payload: mc_protocol::packets::CustomPayload::Unknown {
            channel: mc_loader::loader_ack_channel().clone(),
            payload: serde_json::to_vec(&ack).unwrap(),
        },
    }
}

fn brand_payload() -> ServerboundCustomPayload {
    ServerboundCustomPayload {
        payload: mc_protocol::packets::CustomPayload::Unknown {
            channel: mc_data::Identifier::parse("minecraft:brand").unwrap(),
            payload: vec![0],
        },
    }
}

/// 2026-11-11 loader-live flake: a Fabric client stalled about 90 s after the manifest
/// (its render thread was busy), the edge read timed out in Configuration, and the Java
/// client closes a connection that is silent for 30 s on its own. Virtual time: the server
/// must keep the stalled Loader client's connection alive with configuration keepalives
/// (never silent for 30 s) through a 90 s stall, and finish when the ack arrives.
#[tokio::test(start_paused = true)]
async fn loader_negotiation_keeps_a_90_second_stall_alive_with_keepalives() {
    use mc_protocol::packets::configuration::{
        ConfigurationKeepAlive, ServerboundConfigurationKeepAlive,
    };
    use tokio::time::{Duration, Instant};
    let (mut client, mut server) = loopback_pair().await;
    let manifest = items_bundle(true);
    let ack = loader_ack(&manifest);
    let server_task = tokio::spawn(async move {
        let mut prefs = super::ClientPrefs::default();
        crate::plugin_loader::negotiate(&mut server, &manifest, &mut prefs, true).await
    });
    let manifest_frame = client
        .read_frame_in_untimed(mc_protocol::State::Configuration)
        .await
        .unwrap();
    assert_eq!(
        manifest_frame.id,
        mc_protocol::packets::configuration::ClientboundCustomPayload::ID
    );
    let stall_start = Instant::now();
    let stall_end = stall_start + Duration::from_secs(90);
    let mut last_byte = stall_start;
    let mut longest_silence = Duration::ZERO;
    let mut keepalives = 0;
    // The client is busy: it only reads (as the netty thread does) and echoes.
    while let Ok(frame) = tokio::time::timeout_at(
        stall_end,
        client.read_frame_in_untimed(mc_protocol::State::Configuration),
    )
    .await
    {
        let frame = frame.unwrap();
        assert_eq!(
            frame.id,
            ConfigurationKeepAlive::ID,
            "only keepalives while waiting"
        );
        let mut body = frame.body.clone();
        let keepalive = ConfigurationKeepAlive::decode(&mut body).unwrap();
        longest_silence = longest_silence.max(Instant::now() - last_byte);
        last_byte = Instant::now();
        keepalives += 1;
        client
            .send_packet(&ServerboundConfigurationKeepAlive { id: keepalive.id })
            .await
            .unwrap();
    }
    longest_silence = longest_silence.max(stall_end - last_byte);
    assert!(keepalives >= 8, "{keepalives} keepalives in 90 s");
    assert!(
        longest_silence < Duration::from_secs(30),
        "the Java client's ReadTimeoutHandler(30) would have fired: {longest_silence:?}"
    );
    client.send_packet(&ack).await.unwrap();
    let session = server_task.await.unwrap().unwrap();
    assert_eq!(session.platform(), mc_loader::LoaderPlatform::Native);
}

/// A client that drips an ignored payload faster than any per-read deadline must not
/// hold the pre-play slot past the total negotiation deadline.
#[tokio::test(start_paused = true)]
async fn loader_negotiation_closes_a_dripping_client_at_the_total_deadline() {
    use mc_protocol::packets::configuration::ConfigurationDisconnect;
    use tokio::time::{Duration, Instant};
    let (mut client, mut server) = loopback_pair().await;
    let manifest = items_bundle(true);
    let server_task = tokio::spawn(async move {
        let mut prefs = super::ClientPrefs::default();
        crate::plugin_loader::negotiate(&mut server, &manifest, &mut prefs, true).await
    });
    let start = Instant::now();
    let mut disconnected = false;
    let mut drip = tokio::time::interval(Duration::from_secs(9));
    drip.tick().await;
    while !disconnected {
        tokio::select! {
            frame = client.read_frame_in_untimed(mc_protocol::State::Configuration) => {
                disconnected = frame.unwrap().id == ConfigurationDisconnect::ID;
            }
            _ = drip.tick() => {
                // The server may already have closed; the disconnect frame is read above.
                let _ = client.send_packet(&brand_payload()).await;
            }
        }
    }
    let elapsed = Instant::now() - start;
    assert!(
        elapsed >= crate::plugin_loader::LOADER_NEGOTIATION_TOTAL_TIMEOUT
            && elapsed
                < crate::plugin_loader::LOADER_NEGOTIATION_TOTAL_TIMEOUT + Duration::from_secs(5),
        "closed after {elapsed:?}"
    );
    let error = server_task.await.unwrap().unwrap_err().to_string();
    assert!(error.contains("not finished within"), "{error}");
}

/// A Loader-required server must still tell a vanilla client what to install: with no
/// hello, request or ack the negotiation refuses it within the short hello window,
/// before the Java client's own 30 s read timeout shows its generic "Timed out".
#[tokio::test(start_paused = true)]
async fn required_loader_content_refuses_a_vanilla_client_with_the_loader_message_quickly() {
    use mc_protocol::packets::configuration::ConfigurationDisconnect;
    use tokio::time::{Duration, Instant};
    let (mut client, mut server) = loopback_pair().await;
    let data = test_data();
    let manifest = items_bundle(true);
    let server_task = tokio::spawn(async move {
        let tags = test_tags();
        super::handle_configuration(
            &mut server,
            &data,
            &tags,
            Some(&manifest),
            true,
            &Default::default(),
        )
        .await
    });
    // Features, register, probe (a vanilla client ignores both), known packs.
    for _ in 0..3 {
        client
            .read_frame_in(mc_protocol::State::Configuration)
            .await
            .unwrap();
    }
    let known = client
        .read_frame_in(mc_protocol::State::Configuration)
        .await
        .unwrap();
    assert_eq!(known.id, ClientboundKnownPacks::ID);
    let packs = ClientboundKnownPacks::decode(&mut known.body.clone()).unwrap();
    client
        .send_packet(&ServerboundKnownPacks { packs: packs.packs })
        .await
        .unwrap();
    let started = Instant::now();
    let reason = loop {
        let frame = client
            .read_frame_in_untimed(mc_protocol::State::Configuration)
            .await
            .unwrap();
        if frame.id == ConfigurationDisconnect::ID {
            break frame.body;
        }
        assert_ne!(
            frame.id,
            mc_protocol::packets::configuration::ConfigurationKeepAlive::ID,
            "no keepalive for a client that never said it is a Loader client"
        );
    };
    // Virtual time from the KnownPacks reply: the hello window (10 s) plus the server's
    // own configuration sends; far inside the Java client's 30 s ReadTimeoutHandler.
    let refused_after = Instant::now() - started;
    assert!(
        refused_after >= crate::plugin_loader::LOADER_HELLO_TIMEOUT
            && refused_after < Duration::from_secs(20),
        "refused after {refused_after:?}"
    );
    let reason = String::from_utf8_lossy(&reason).into_owned();
    assert!(
        reason.contains("requires the Solaris Loader client mod"),
        "{reason}"
    );
    assert!(server_task.await.unwrap().is_err());
}
