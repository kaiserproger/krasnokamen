use super::*;
use mc_protocol::packets::{Packet, play::*};

/// A per-test admin directory (prefix `mc-edge-admin-`) under the temp dir.
pub(crate) fn admin_temp_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("mc-edge-admin-")
        .tempdir_in(std::env::temp_dir())
        .expect("create the admin test directory")
}

/// The admin fixture whose directory removes itself, on success and on panic.
///
/// This replaces the manual `remove_dir_all`, which ran only on the success
/// path, so every failing admin test left a whole world behind (one leaked
/// directory held 229k files and pushed the shared tmpfs past half its inode
/// budget). `tempfile::TempDir` is the pattern the admin module's own tests
/// already use; directories that existed before this run are never touched.
pub(crate) fn fixture_in_temp_dir() -> (EdgeServer, tempfile::TempDir) {
    let dir = admin_temp_dir();
    let server = server_at(dir.path());
    (server, dir)
}

/// The same fixture, shaped as a path for call sites that keep using `dir`.
///
/// It still owns the `TempDir`: a panic before the caller's own cleanup finds
/// the directory already removed, never leaked.
pub(crate) struct AdminWorld(tempfile::TempDir);

impl std::ops::Deref for AdminWorld {
    type Target = std::path::Path;

    fn deref(&self) -> &Self::Target {
        self.0.path()
    }
}

impl AsRef<std::path::Path> for AdminWorld {
    fn as_ref(&self) -> &std::path::Path {
        self.0.path()
    }
}

pub(crate) fn fixture() -> (EdgeServer, AdminWorld) {
    let dir = admin_temp_dir();
    let server = server_at(dir.path());
    (server, AdminWorld(dir))
}

fn server_at(dir: &std::path::Path) -> EdgeServer {
    let mut config = mc_core::config::CoreConfig::new(dir.join("world"), 42);
    config.vanilla_data_dir =
        Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla"));
    config.checkpoint_every_ticks = u64::MAX;
    let hub = Arc::new(Core::open(config).unwrap());
    let report = mc_data::blocks::krasnokamen_required_blocks_report();
    let data = Arc::new(mc_data::krasnokamen_required_data());
    let cfg = EdgeConfig::default();
    EdgeServer {
        loader_manifest: None,
        protocols: crate::translation::Catalog::default(),
        wire: WireRegistries {
            dimension_type_ids: crate::dimensions::registry_ids(&data).unwrap(),
            biomes: data.registry("worldgen/biome").unwrap().clone(),
            blocks: mc_world::BlockRegistry::from_report(&report).unwrap(),
            items: mc_data::items::krasnokamen_required_items(),
            block_entity_types:
                mc_data::block_entity_types::krasnokamen_required_block_entity_types(),
        },
        hub,
        data,
        tags: Arc::new(Default::default()),
        dashboard: dashboard_state(&cfg),
        cfg,
        connections: Arc::new(ConnectionRegistry::new()),
        pre_play_addresses: crate::conn_limit::AddressLimiter::new(0, 8),
        tick_stats: Arc::new(TickStats::new()),
        net_counters: Arc::new(NetCounters::new()),
        admin: crate::admin::Admin::open(dir).unwrap(),
        console_state: Default::default(),
        stop: tokio::sync::watch::channel(false).0,
        suggestion_items: vec!["minecraft:stone".into()],
        suggestion_blocks: vec!["minecraft:stone".into()],
        suggestion_entities: vec!["minecraft:pig".into()],
        suggestion_structures: vec![],
        online_authentication: None,
        velocity: None,
        loader_required: false,
        lod: None,
        dh: None,
        computer_processes: Arc::new(tokio::sync::Semaphore::new(4)),
        operator_updates: tokio::sync::Mutex::new(()),
        screens: Default::default(),
        voice: Default::default(),
    }
}

pub(crate) fn register(
    server: &EdgeServer,
    name: &str,
    op: bool,
) -> (ConnectionId, mpsc::Receiver<bytes::Bytes>) {
    let id = server.connections.joining_id();
    let uuid = crate::crypto::offline_uuid(name);
    let (reliable_tx, reliable_rx) = mpsc::channel(64);
    let (world_tx, _) = world_queue(64, WORLD_QUEUE_BYTES);
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
    server.connections.set_name(id, name.into());
    server
        .admin
        .register(id, uuid, name.into(), "127.0.0.1".parse().unwrap());
    if op {
        server.admin.set_op(uuid, name, true).unwrap();
    }
    (id, reliable_rx)
}

async fn receive(rx: &mut mpsc::Receiver<bytes::Bytes>) -> mc_protocol::frame::RawFrame {
    // Real compressed frame delivery over a loopback socket, decoded by the
    // same connection codec used in Play (no synthetic packet-body shortcut).
    use tokio::io::AsyncWriteExt;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut writer = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (reader, _) = listener.accept().await.unwrap();
    writer.write_all(&rx.recv().await.unwrap()).await.unwrap();
    let mut connection = crate::connection::Connection::new(reader);
    connection.set_compression(mc_protocol::frame::Compression::Threshold(256));
    connection.read_frame_play().await.unwrap()
}

#[tokio::test]
async fn operator_updates_wait_for_capacity_before_mutating() {
    use std::future::Future;
    let (server, _dir) = fixture_in_temp_dir();
    let (operator, _operator_rx) = register(&server, "Operator", true);
    let (_alex, _alex_rx) = register(&server, "Alex", false);
    let uuid = crate::crypto::offline_uuid("Alex");
    let held = server.hub.reserve_inputs(4096).await.unwrap();
    let dropped = server.hub.dropped_inputs();
    let command = server.handle_admin_command(operator, "op Alex");
    tokio::pin!(command);
    let deop = server.handle_admin_command(operator, "deop Alex");
    tokio::pin!(deop);
    std::future::poll_fn(|cx| {
        assert!(command.as_mut().poll(cx).is_pending());
        assert!(deop.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    assert_eq!(
        server.hub.dropped_inputs(),
        dropped,
        "a full queue must park the update, not poll submit and shed copies"
    );
    assert_eq!(server.admin.level(uuid), 0);
    drop(held);
    assert!(command.await.unwrap());
    assert_eq!(server.admin.level(uuid), 4);
    assert!(deop.await.unwrap());
    assert_eq!(server.admin.level(uuid), 0);
    assert_eq!(server.hub.dropped_inputs(), dropped);
    server.hub.shutdown();
    // The fixture's TempDir removes this directory when it is dropped.
}

#[tokio::test]
async fn queued_op_is_rejected_after_initiator_is_deopped() {
    use std::future::Future;
    let (server, _dir) = fixture_in_temp_dir();
    let (operator, _operator_rx) = register(&server, "Operator", true);
    let (victim, _victim_rx) = register(&server, "Victim", true);
    let uuid = crate::crypto::offline_uuid("Victim");
    let held = server.hub.reserve_inputs(4096).await.unwrap();
    let deop = server.handle_admin_command(operator, "deop Victim");
    let regain = server.handle_admin_command(victim, "op Victim");
    tokio::pin!(deop, regain);
    std::future::poll_fn(|cx| {
        assert!(deop.as_mut().poll(cx).is_pending());
        assert!(regain.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(held);
    assert!(deop.await.unwrap());
    assert!(regain.await.unwrap());
    assert_eq!(
        server.admin.level(uuid),
        0,
        "demoted operator must not regain rights"
    );
    server.hub.shutdown();
    // The fixture's TempDir removes this directory when it is dropped.
}

#[tokio::test]
async fn deop_reaches_rejoining_native_session() {
    let (server, _dir) = fixture_in_temp_dir();
    let (operator, _operator_rx) = register(&server, "Operator", true);
    let (old, _old_rx) = register(&server, "Alex", true);
    let uuid = crate::crypto::offline_uuid("Alex");
    let new = server.connections.joining_id();
    let mut ticks = server.hub.subscribe();
    let session = connection::admit_native_session(
        &server,
        new,
        uuid,
        "Alex".into(),
        "127.0.0.1".parse().unwrap(),
        mc_core::boundary::ClientSettings {
            view_distance: 2,
            locale: "en_us".into(),
            chat_visibility: mc_core::boundary::ChatVisibility::Full,
        },
    )
    .await
    .unwrap();
    assert_eq!(server.admin.sessions_for_uuid(uuid).len(), 2);
    assert!(
        server
            .handle_admin_command(operator, "deop Alex")
            .await
            .unwrap()
    );
    assert_eq!(server.admin.session_level(old), 0);
    assert_eq!(server.admin.session_level(new), 0);
    assert_native_deopped(&server, new, &mut ticks).await;
    drop(session);
    server.hub.shutdown();
    // The fixture's TempDir removes this directory when it is dropped.
}

#[tokio::test]
async fn joining_session_and_deop_keep_native_permission_order() {
    use std::future::Future;
    let (server, _dir) = fixture_in_temp_dir();
    let (operator, _operator_rx) = register(&server, "Operator", true);
    let uuid = crate::crypto::offline_uuid("Alex");
    server.admin.set_op(uuid, "Alex", true).unwrap();
    assert_eq!(server.admin.level(uuid), 4);
    let id = server.connections.joining_id();
    let mut ticks = server.hub.subscribe();
    let guard = server.operator_updates.lock().await;
    let joining = connection::admit_native_session(
        &server,
        id,
        uuid,
        "Alex".into(),
        "127.0.0.1".parse().unwrap(),
        mc_core::boundary::ClientSettings {
            view_distance: 2,
            locale: "en_us".into(),
            chat_visibility: mc_core::boundary::ChatVisibility::Full,
        },
    );
    tokio::pin!(joining);
    let deop = server.handle_admin_command(operator, "deop Alex");
    tokio::pin!(deop);
    std::future::poll_fn(|cx| {
        assert!(
            joining.as_mut().poll(cx).is_pending(),
            "Join must wait on operator fence"
        );
        assert!(
            deop.as_mut().poll(cx).is_pending(),
            "deop must wait on operator fence"
        );
        std::task::Poll::Ready(())
    })
    .await;
    drop(guard);
    let session = joining.await.unwrap();
    assert!(deop.await.unwrap());
    assert_eq!(server.admin.session_level(id), 0);
    assert_native_deopped(&server, id, &mut ticks).await;
    drop(session);
    server.hub.shutdown();
    // The fixture's TempDir removes this directory when it is dropped.
}

async fn assert_native_deopped(
    server: &EdgeServer,
    id: ConnectionId,
    ticks: &mut tokio::sync::broadcast::Receiver<mc_core::boundary::TickOutput>,
) {
    for _ in 0..100 {
        assert!(server.hub.submit(mc_core::boundary::PlayerInput::Chat {
            connection: id,
            message: "test".into()
        }));
    }
    let disconnected = tokio::time::timeout(std::time::Duration::from_secs(60), async {
        for _ in 0..20 {
            ticks.recv().await.unwrap();
            if server.hub.try_drain_outbox().iter().any(|(connection, changes)| {
                *connection == id && changes.iter().any(|change| matches!(change,
                    mc_core::boundary::PlayerChange::Disconnect { reason } if reason == "disconnect.spam"))
            }) {
                return true;
            }
        }
        false
    }).await.expect("native tick publications stalled");
    assert!(
        disconnected,
        "deopped native player must hit the chat-spam gate"
    );
}

#[tokio::test]
async fn deop_between_snapshot_and_registration_reaches_native_join() {
    let (server, _dir) = fixture_in_temp_dir();
    let (operator, _operator_rx) = register(&server, "Operator", true);
    let uuid = crate::crypto::offline_uuid("Alex");
    server.admin.set_op(uuid, "Alex", true).unwrap();
    let stale = server.admin.level(uuid);
    assert_eq!(stale, 4);
    let id = server.connections.joining_id();
    let mut ticks = server.hub.subscribe();
    assert!(
        server
            .handle_admin_command(operator, "deop Alex")
            .await
            .unwrap()
    );
    let session = connection::admit_native_session(
        &server,
        id,
        uuid,
        "Alex".into(),
        "127.0.0.1".parse().unwrap(),
        mc_core::boundary::ClientSettings {
            view_distance: 2,
            locale: "en_us".into(),
            chat_visibility: mc_core::boundary::ChatVisibility::Full,
        },
    )
    .await
    .unwrap();
    assert_eq!(server.admin.session_level(id), 0);
    assert_native_deopped(&server, id, &mut ticks).await;
    drop(session);
    server.hub.shutdown();
    // The fixture's TempDir removes this directory when it is dropped.
}

#[tokio::test]
async fn join_burst_permission_change_refreshes_command_tree() {
    let (server, _dir) = fixture_in_temp_dir();
    let (alex, mut rx) = register(&server, "Alex", true);
    let uuid = crate::crypto::offline_uuid("Alex");
    server.admin.set_op(uuid, "Alex", false).unwrap();
    connection::refresh_join_permission_tree(&server, alex, 4).unwrap();
    assert_eq!(
        rx.len(),
        1,
        "a changed permission must queue one command tree"
    );
    let frame = receive(&mut rx).await;
    let tree = ClientboundCommands::decode(&mut &frame.body[..]).unwrap();
    assert!(
        !tree
            .nodes
            .iter()
            .any(|node| matches!(&node.kind, CommandNodeKind::Literal(name) if name == "op"))
    );
    connection::refresh_join_permission_tree(&server, alex, 0).unwrap();
    assert!(
        rx.try_recv().is_err(),
        "unchanged permission needs no refresh"
    );
    server.hub.shutdown();
    // The fixture's TempDir removes this directory when it is dropped.
}

#[tokio::test]
async fn suggestion_round_trip_and_permission_tree_refresh() {
    let (server, _dir) = fixture_in_temp_dir();
    let (operator, mut operator_rx) = register(&server, "Operator", true);
    let (alex, mut alex_rx) = register(&server, "Alex", false);
    let mut body = Vec::new();
    ServerboundCommandSuggestion {
        id: 17,
        command: "/msg Al".into(),
    }
    .encode(&mut body)
    .unwrap();
    assert!(
        server
            .handle_edge_frame(alex, ServerboundCommandSuggestion::ID, &body)
            .await
            .unwrap()
    );
    let frame = receive(&mut alex_rx).await;
    assert_eq!(frame.id, ClientboundCommandSuggestions::ID);
    let response = ClientboundCommandSuggestions::decode(&mut &frame.body[..]).unwrap();
    assert_eq!(response.id, 17);
    assert!(
        response
            .suggestions
            .iter()
            .any(|entry| entry.text == "Alex")
    );
    assert!(
        server
            .handle_admin_command(operator, "op Alex")
            .await
            .unwrap()
    );
    let frame = receive(&mut alex_rx).await;
    assert_eq!(frame.id, ClientboundCommands::ID);
    let tree = ClientboundCommands::decode(&mut &frame.body[..]).unwrap();
    assert!(
        tree.nodes
            .iter()
            .any(|node| matches!(&node.kind, CommandNodeKind::Literal(name) if name == "op"))
    );
    operator_rx.recv().await.unwrap();
    assert!(
        server
            .handle_admin_command(operator, "deop Alex")
            .await
            .unwrap()
    );
    let frame = receive(&mut alex_rx).await;
    let tree = ClientboundCommands::decode(&mut &frame.body[..]).unwrap();
    assert!(
        !tree
            .nodes
            .iter()
            .any(|node| matches!(&node.kind, CommandNodeKind::Literal(name) if name == "op"))
    );
    server.hub.shutdown();
    // The fixture's TempDir removes this directory when it is dropped.
}

fn native_checkpoint_identity() -> [u8; 32] {
    let mut identity = [0; 32];
    identity[..16].copy_from_slice(b"KrasnoV2Native\0\0");
    identity[16..24].copy_from_slice(&42_u64.to_be_bytes());
    identity[24..].copy_from_slice(&(mc_worldgen::WORLDGEN_REVISION as u64).to_be_bytes());
    identity
}

#[tokio::test]
async fn save_all_publishes_checkpoint_before_success() {
    let (server, _dir) = fixture_in_temp_dir();
    let (id, mut rx) = register(&server, "Operator", true);
    assert!(
        server
            .handle_admin_command(id, "save-all flush")
            .await
            .unwrap()
    );
    let start = receive(&mut rx).await;
    let done = receive(&mut rx).await;
    assert_eq!(start.id, ClientboundSystemChat::ID);
    assert_eq!(done.id, ClientboundSystemChat::ID);
    let response = ClientboundSystemChat::decode(&mut &done.body[..]).unwrap();
    let content = mc_nbt::read_network(&mut response.content_nbt.as_slice()).unwrap();
    assert_eq!(
        content,
        mc_nbt::Tag::Compound(vec![(
            "text".into(),
            mc_nbt::Tag::String("Saved the game".into()),
        )]),
        "save-all must report success, not Failed to save the game"
    );
    server.hub.shutdown();
    // The fixture's TempDir removes this directory when it is dropped.
}

#[tokio::test]
async fn back_to_back_save_all_commands_both_report_success() {
    let (server, _dir) = fixture_in_temp_dir();
    let (id, mut rx) = register(&server, "Operator", true);
    let (first, second) = tokio::join!(
        server.handle_admin_command(id, "save-all flush"),
        server.handle_admin_command(id, "save-all flush"),
    );
    assert!(first.unwrap() && second.unwrap());
    let mut succeeded = 0;
    for _ in 0..4 {
        let frame = receive(&mut rx).await;
        assert_eq!(frame.id, ClientboundSystemChat::ID);
        let response = ClientboundSystemChat::decode(&mut &frame.body[..]).unwrap();
        let content = mc_nbt::read_network(&mut response.content_nbt.as_slice()).unwrap();
        let mc_nbt::Tag::Compound(fields) = content else {
            panic!("expected command feedback compound");
        };
        let text = mc_data::item_nbt::field(&fields, "text").unwrap();
        match text {
            mc_nbt::Tag::String(text) if text == "Saved the game" => succeeded += 1,
            mc_nbt::Tag::String(text) if text == "Saving the game (this may take a moment!)" => {}
            _ => panic!("unexpected save feedback: {text:?}"),
        }
    }
    assert_eq!(succeeded, 2);
    server.hub.shutdown();
    // The fixture's TempDir removes this directory when it is dropped.
}

#[tokio::test]
async fn stop_drains_shutdown_disconnect_and_checkpoints() {
    let (server, _dir) = fixture_in_temp_dir();
    let (id, mut rx) = register(&server, "Operator", true);
    let captured_tick = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        server.hub.subscribe().recv(),
    )
    .await
    .unwrap()
    .unwrap()
    .tick;
    assert!(server.handle_admin_command(id, "stop").await.unwrap());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), serve(listener, server))
        .await
        .unwrap()
        .unwrap();
    let feedback = receive(&mut rx).await;
    assert_eq!(feedback.id, ClientboundSystemChat::ID);
    let kick = receive(&mut rx).await;
    assert_eq!(kick.id, PlayDisconnect::ID);
    let stopped = mc_world::native::NativeReadOnlyStore::open_with_cache(
        &_dir.path().join("world/native/state.redb"),
        native_checkpoint_identity(),
        16 * 1024 * 1024,
    )
    .unwrap()
    .read_checkpoint()
    .unwrap();
    assert!(stopped.tick >= captured_tick);
    assert!(!_dir.path().join("world/CHECKPOINT-MANIFEST.json").exists());
    // The fixture's TempDir removes this directory when it is dropped.
}

fn feedback_text(frame: &mc_protocol::frame::RawFrame) -> String {
    assert_eq!(frame.id, ClientboundSystemChat::ID);
    let response = ClientboundSystemChat::decode(&mut &frame.body[..]).unwrap();
    let content = mc_nbt::read_network(&mut response.content_nbt.as_slice()).unwrap();
    let mc_nbt::Tag::Compound(fields) = content else {
        panic!("system chat is not a compound");
    };
    fields
        .into_iter()
        .find_map(|(name, value)| match (name.as_str(), value) {
            ("text", mc_nbt::Tag::String(text)) => Some(text),
            _ => None,
        })
        .expect("system chat carries plain text")
}

#[tokio::test]
async fn whitelist_and_banlist_commands_report_and_enforce_vanilla_state() {
    let (server, _dir) = fixture_in_temp_dir();
    let (operator, mut operator_rx) = register(&server, "Operator", true);
    let (alex, mut alex_rx) = register(&server, "Alex", false);
    assert!(
        server
            .handle_admin_command(operator, "whitelist on")
            .await
            .unwrap()
    );
    assert_eq!(
        feedback_text(&receive(&mut operator_rx).await),
        "Whitelist is now turned on"
    );
    assert!(
        server
            .handle_admin_command(operator, "whitelist add Alex")
            .await
            .unwrap()
    );
    assert_eq!(
        feedback_text(&receive(&mut operator_rx).await),
        "Added Alex to the whitelist"
    );
    assert!(
        server
            .handle_admin_command(operator, "whitelist list")
            .await
            .unwrap()
    );
    assert_eq!(
        feedback_text(&receive(&mut operator_rx).await),
        "There are 1 whitelisted player(s): Alex"
    );
    assert_eq!(
        server
            .admin
            .admission(crate::crypto::offline_uuid("Stranger"), None, false)
            .unwrap()["translate"],
        "multiplayer.disconnect.not_whitelisted"
    );
    assert!(
        server
            .admin
            .admission(crate::crypto::offline_uuid("Alex"), None, false)
            .is_none()
    );
    // Level 3 is required: an ordinary player cannot change the list.
    assert!(
        server
            .handle_admin_command(alex, "whitelist off")
            .await
            .unwrap()
    );
    assert_eq!(
        feedback_text(&receive(&mut alex_rx).await),
        "Unknown or incomplete command"
    );
    server
        .admin
        .ban_player(uuid::Uuid::new_v4(), "Banned", "Console", "test", true)
        .unwrap();
    server
        .admin
        .ban_ip("10.0.0.1", "Console", "test", true)
        .unwrap();
    assert!(
        server
            .handle_admin_command(operator, "banlist players")
            .await
            .unwrap()
    );
    assert_eq!(
        feedback_text(&receive(&mut operator_rx).await),
        "There are 1 total banned players:"
    );
    assert_eq!(
        feedback_text(&receive(&mut operator_rx).await),
        server.admin.bans().0[0]
    );
    assert!(
        server
            .handle_admin_command(operator, "banlist ips")
            .await
            .unwrap()
    );
    assert_eq!(
        feedback_text(&receive(&mut operator_rx).await),
        "There are 1 total banned IPs:"
    );
    assert_eq!(feedback_text(&receive(&mut operator_rx).await), "10.0.0.1");
    server.hub.shutdown();
    // The fixture's TempDir removes this directory when it is dropped.
}

#[tokio::test]
async fn moderation_selectors_ban_all_online_players_without_fake_profiles() {
    let (server, _dir) = fixture_in_temp_dir();
    let (alice, mut alice_rx) = register(&server, "Alice", false);
    let (bob, mut bob_rx) = register(&server, "Bob", false);
    let mut ticks = server.hub.subscribe();
    for (id, name) in [(alice, "Alice"), (bob, "Bob")] {
        assert!(server.hub.submit(mc_core::boundary::PlayerInput::Join {
            connection: id,
            uuid: crate::crypto::offline_uuid(name),
            name: name.into(),
            permission_level: 0,
        }));
    }
    for _ in 0..3 {
        ticks.recv().await.unwrap();
    }
    assert!(
        server
            .handle_admin_command(mc_core::command_console::CONNECTION, "ban @a griefing")
            .await
            .unwrap()
    );
    let bans = server.admin.bans().0;
    assert_eq!(bans.len(), 2);
    assert!(bans.iter().any(|entry| entry.starts_with("Alice (")));
    assert!(bans.iter().any(|entry| entry.starts_with("Bob (")));
    assert!(!bans.iter().any(|entry| entry.starts_with("@a (")));
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(2), receive(&mut alice_rx))
            .await
            .unwrap()
            .id,
        PlayDisconnect::ID
    );
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(2), receive(&mut bob_rx))
            .await
            .unwrap()
            .id,
        PlayDisconnect::ID
    );
    server.hub.shutdown();
    // The fixture's TempDir removes this directory when it is dropped.
}

#[tokio::test]
async fn moderation_selectors_missing_and_invalid_never_ban_literal() {
    let (server, _dir) = fixture_in_temp_dir();
    for command in ["ban @a reason", "ban @bogus reason", "pardon @a"] {
        assert!(
            server
                .handle_admin_command(mc_core::command_console::CONNECTION, command)
                .await
                .unwrap()
        );
        assert!(server.admin.bans().0.is_empty(), "{command}");
    }
    assert!(
        server
            .handle_admin_command(mc_core::command_console::CONNECTION, "ban Offline reason")
            .await
            .unwrap()
    );
    assert!(
        server
            .admin
            .bans()
            .0
            .iter()
            .any(|entry| entry.starts_with("Offline ("))
    );
    server.hub.shutdown();
    // The fixture's TempDir removes this directory when it is dropped.
}

#[tokio::test]
async fn moderation_selectors_kick_all_and_report_missing_without_success() {
    let (server, _dir) = fixture_in_temp_dir();
    let (alice, mut alice_rx) = register(&server, "Alice", false);
    let (bob, mut bob_rx) = register(&server, "Bob", false);
    let mut ticks = server.hub.subscribe();
    for (id, name) in [(alice, "Alice"), (bob, "Bob")] {
        assert!(server.hub.submit(mc_core::boundary::PlayerInput::Join {
            connection: id,
            uuid: crate::crypto::offline_uuid(name),
            name: name.into(),
            permission_level: 0,
        }));
    }
    for _ in 0..3 {
        ticks.recv().await.unwrap();
    }
    assert!(
        server
            .handle_admin_command(mc_core::command_console::CONNECTION, "kick @a reason")
            .await
            .unwrap()
    );
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(2), receive(&mut alice_rx))
            .await
            .unwrap()
            .id,
        PlayDisconnect::ID
    );
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(2), receive(&mut bob_rx))
            .await
            .unwrap()
            .id,
        PlayDisconnect::ID
    );
    let (operator, mut operator_rx) = register(&server, "Operator", true);
    assert!(
        server
            .handle_admin_command(operator, "kick Nobody reason")
            .await
            .unwrap()
    );
    assert_eq!(
        feedback_text(&receive(&mut operator_rx).await),
        "No player was found"
    );
    assert!(
        operator_rx.try_recv().is_err(),
        "failure must not emit a completion"
    );
    server.hub.shutdown();
    // The fixture's TempDir removes this directory when it is dropped.
}

#[tokio::test]
async fn moderation_selectors_ip_and_offline_profile_rules() {
    let (server, _dir) = fixture_in_temp_dir();
    let operator = mc_core::command_console::CONNECTION;
    let (_alice, _alice_rx) = register(&server, "Alice", false);
    assert!(
        server
            .handle_admin_command(operator, "ban-ip Alice reason")
            .await
            .unwrap()
    );
    assert!(
        server
            .handle_admin_command(operator, "pardon-ip Alice")
            .await
            .unwrap()
    );
    assert_eq!(server.admin.bans().1, vec!["127.0.0.1"]);
    assert!(
        server
            .handle_admin_command(operator, "pardon-ip 127.0.0.1")
            .await
            .unwrap()
    );
    assert!(server.admin.bans().1.is_empty());
    server.admin.ban_ip("::1", "Server", "test", true).unwrap();
    assert!(
        server
            .handle_admin_command(operator, "pardon-ip ::1")
            .await
            .unwrap()
    );
    assert_eq!(server.admin.bans().1, vec!["::1"]);
    assert!(
        server
            .handle_admin_command(operator, "ban Offline reason")
            .await
            .unwrap()
    );
    assert!(
        server
            .admin
            .bans()
            .0
            .iter()
            .any(|entry| entry.starts_with("Offline ("))
    );
    assert!(
        server
            .handle_admin_command(operator, "pardon Offline")
            .await
            .unwrap()
    );
    assert!(server.admin.bans().0.is_empty());
    server.hub.shutdown();
    // The fixture's TempDir removes this directory when it is dropped.
}

#[tokio::test]
async fn moderation_selectors_idempotent_ban_and_pardon_report_failure() {
    let (server, _dir) = fixture_in_temp_dir();
    let (operator, mut rx) = register(&server, "Operator", true);
    for (command, success) in [
        ("ban Offline reason", true),
        ("ban Offline reason", false),
        ("pardon Offline", true),
        ("pardon Offline", false),
    ] {
        assert!(
            server
                .handle_admin_command(operator, command)
                .await
                .unwrap()
        );
        let text = feedback_text(&receive(&mut rx).await);
        assert_eq!(text.contains("completed"), success, "{command}: {text}");
    }
    server.hub.shutdown();
    // The fixture's TempDir removes this directory when it is dropped.
}

#[tokio::test]
async fn moderation_selectors_invalid_and_empty_report_failure() {
    let (server, _dir) = fixture_in_temp_dir();
    let (operator, mut rx) = register(&server, "Operator", true);
    for command in ["kick @a", "ban @a", "ban @bogus", "pardon @a"] {
        assert!(
            server
                .handle_admin_command(operator, command)
                .await
                .unwrap()
        );
        let text = feedback_text(&receive(&mut rx).await);
        assert!(!text.contains("completed"), "{command}: {text}");
        assert!(rx.try_recv().is_err(), "{command} emitted extra feedback");
        assert!(server.admin.bans().0.is_empty());
    }
    server.hub.shutdown();
    // The fixture's TempDir removes this directory when it is dropped.
}

#[tokio::test]
async fn moderation_selectors_filtered_ban_and_online_pardon() {
    let (server, _dir) = fixture_in_temp_dir();
    let (alice, _alice_rx) = register(&server, "Alice", false);
    let (bob, _bob_rx) = register(&server, "Bob", false);
    let mut ticks = server.hub.subscribe();
    for (id, name) in [(alice, "Alice"), (bob, "Bob")] {
        assert!(server.hub.submit(mc_core::boundary::PlayerInput::Join {
            connection: id,
            uuid: crate::crypto::offline_uuid(name),
            name: name.into(),
            permission_level: 0,
        }));
    }
    for _ in 0..3 {
        ticks.recv().await.unwrap();
    }
    let console = mc_core::command_console::CONNECTION;
    assert!(
        server
            .handle_admin_command(console, "ban @a[name=Alice, limit=1] 1h test")
            .await
            .unwrap()
    );
    let bans = server.admin.bans().0;
    assert_eq!(bans.len(), 1);
    assert!(bans[0].starts_with("Alice ("));
    let persisted: serde_json::Value =
        serde_json::from_slice(&std::fs::read(_dir.path().join("banned-players.json")).unwrap())
            .unwrap();
    assert_ne!(persisted[0]["expires"], "forever");
    assert_eq!(persisted[0]["reason"], "test");
    server
        .admin
        .ban_player(
            crate::crypto::offline_uuid("Bob"),
            "Bob",
            "Server",
            "test",
            true,
        )
        .unwrap();
    assert!(
        server
            .handle_admin_command(console, "pardon @a")
            .await
            .unwrap()
    );
    assert!(server.admin.bans().0.is_empty());
    server.hub.shutdown();
    // The fixture's TempDir removes this directory when it is dropped.
}

/// A failing admin test must leave no world behind, and touch no other one.
///
/// The fixture used to delete its directory only on the success path, so every
/// panic leaked a whole world (one leaked directory held 229k files and pushed
/// the shared tmpfs past half its inode budget). The check is hermetic: it looks
/// at the directory the fixture created and at a sentinel it owns, never at the
/// whole temp dir, because neighbouring tests create and remove their own.
#[test]
fn a_failing_admin_test_leaves_no_temp_dir_and_removes_no_other() {
    let sentinel = admin_temp_dir();
    let mut created = None;
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let (_server, dir) = fixture_in_temp_dir();
        created = Some(dir.path().to_path_buf());
        assert!(
            dir.path().is_dir(),
            "the fixture directory exists during the test"
        );
        panic!("simulated assertion failure inside an admin test");
    }));
    std::panic::set_hook(previous);
    assert!(outcome.is_err(), "the simulated admin test must fail");
    let created = created.expect("the fixture ran");
    assert!(
        !created.exists(),
        "a failed admin test left {} behind",
        created.display()
    );
    assert!(
        sentinel.path().is_dir(),
        "the fixture removed the test's own sentinel directory {}",
        sentinel.path().display()
    );
}

/// The passing path of the same fixture leaves nothing either.
#[test]
fn a_passing_admin_test_leaves_no_temp_dir() {
    let sentinel = admin_temp_dir();
    let path = {
        let (server, dir) = fixture_in_temp_dir();
        let path = dir.path().to_path_buf();
        assert!(path.is_dir());
        server.hub.shutdown();
        path
    };
    assert!(
        !path.exists(),
        "a passing admin test left {} behind",
        path.display()
    );
    assert!(
        sentinel.path().is_dir(),
        "the fixture removed a directory it did not create"
    );
}

#[path = "clock_queue_tests.rs"]
mod clock_queue_tests;

#[path = "shelf_update_queue_tests.rs"]
mod shelf_update_queue_tests;

#[path = "lost_entity_frames_tests.rs"]
mod lost_entity_frames_tests;
