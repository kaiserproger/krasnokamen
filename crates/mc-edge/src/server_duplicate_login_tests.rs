//! Duplicate login through the real listener and connection pipeline (vanilla order):
//! the newer login kicks the session already in Play when its login finishes, a connection that
//! finishes configuration while the profile is in Play is refused with the duplicate-login
//! reason, and a full server refuses a same-profile login before any session is kicked.

use std::time::Duration;

use super::*;
use mc_protocol::packets::Packet;
use mc_protocol::packets::configuration::{
    AcknowledgeFinishConfiguration, ClientboundKnownPacks, ConfigurationDisconnect,
    FinishConfiguration, ServerboundKnownPacks,
};
use mc_protocol::packets::play::{LoginPlay, PlayDisconnect};
use mc_vanilla::wire_client::Client;

const REASON: &str = "You logged in from another location";
const WAIT: Duration = Duration::from_secs(60);

fn server(root: &std::path::Path, max_players: u32) -> Arc<EdgeServer> {
    let content = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla");
    let mut config = mc_core::CoreConfig::new(root.join("world"), 82);
    config.vanilla_data_dir = Some(content.clone());
    config.view_distance = 4;
    config.simulation_distance = 4;
    let core = Arc::new(mc_core::Core::open(config).unwrap());
    let mut cfg = crate::config::EdgeConfig {
        server_dir: root.join("server"),
        ..Default::default()
    };
    std::fs::create_dir_all(&cfg.server_dir).unwrap();
    cfg.server.view_distance = 4;
    cfg.server.simulation_distance = 4;
    cfg.server.max_players = max_players;
    cfg.data.world_dir = root.join("world");
    cfg.data.vanilla_data_dir = content;
    cfg.auth.online_mode = false;
    let data = crate::server::load_vanilla_data(&cfg).unwrap();
    let tags = crate::server::load_tags(&cfg, &data).unwrap();
    Arc::new(EdgeServer::new(core, data, tags, cfg).unwrap())
}

/// Real listener and connection pipeline plus the pump that routes the core's output.
async fn listen(server: &Arc<EdgeServer>) -> (std::net::SocketAddr, Vec<tokio::task::JoinHandle<()>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let pump = crate::server::spawn_pump(Arc::clone(server)).unwrap();
    let accept = tokio::spawn({
        let server = Arc::clone(server);
        async move {
            while let Ok((stream, _)) = listener.accept().await {
                let server = Arc::clone(&server);
                tokio::spawn(async move {
                    let _ = super::handle_connection(stream, server).await;
                });
            }
        }
    });
    (addr, vec![pump, accept])
}

/// Read until a Play disconnect arrives and return its reason bytes.
async fn play_disconnect_reason(client: &mut Client, wait: Duration) -> String {
    let mut frame = client
        .wait_for_frame_id_with_timeout(PlayDisconnect::ID, wait)
        .await
        .expect("a Play disconnect with the refusal reason");
    String::from_utf8_lossy(&PlayDisconnect::decode(&mut frame.body).unwrap().reason_nbt)
        .into_owned()
}

/// Login, finish configuration and wait for the Play login packet: the connection is in Play.
async fn enter_play(addr: std::net::SocketAddr, name: &str) -> Client {
    let mut client = Client::connect(addr).await.unwrap();
    client.drive_login(addr, name).await.unwrap();
    client.drive_configuration().await.unwrap();
    client
        .wait_for_frame_id_with_timeout(LoginPlay::ID, WAIT)
        .await
        .expect("the connection reaches Play");
    client
}

/// Login and walk configuration up to, but excluding, the acknowledgement that ends it.
async fn login_held_in_configuration(addr: std::net::SocketAddr, name: &str) -> Client {
    let mut client = Client::connect(addr).await.unwrap();
    client.drive_login(addr, name).await.unwrap();
    loop {
        let frame = client.read_frame_with_timeout(WAIT).await.unwrap();
        if frame.id == ClientboundKnownPacks::ID {
            let known = ClientboundKnownPacks::decode(&mut frame.body.clone()).unwrap();
            client
                .write_packet(&ServerboundKnownPacks { packs: known.packs })
                .await
                .unwrap();
        } else if frame.id == FinishConfiguration::ID {
            return client;
        }
    }
}

/// D2: the edge sends `ProfileLogin` when login finishes, ahead of `Join`. Without it the second
/// login would be refused while the first session is in Play.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn newer_login_of_the_same_account_kicks_the_session_in_play_and_reaches_play() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path(), 20);
    let (addr, tasks) = listen(&server).await;
    let mut first = enter_play(addr, "Dup").await;
    let _second = enter_play(addr, "Dup").await;
    assert!(play_disconnect_reason(&mut first, WAIT).await.contains(REASON));
    server.hub.shutdown();
    tasks.into_iter().for_each(|task| task.abort());
}

/// D5: the first connection is still in configuration when the second logs in and plays; on
/// finishing configuration it is refused and sees the vanilla duplicate-login reason, not a bare
/// closed connection or a join timeout. The second keeps playing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn late_configuration_finisher_receives_the_duplicate_login_reason() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path(), 20);
    let (addr, tasks) = listen(&server).await;
    let mut held = login_held_in_configuration(addr, "Dup").await;
    let mut playing = enter_play(addr, "Dup").await;
    held.write_packet(&AcknowledgeFinishConfiguration)
        .await
        .unwrap();
    // Well inside the 10 s join deadline: the refusal wakes the join wait, it is not a timeout.
    assert!(
        play_disconnect_reason(&mut held, Duration::from_secs(5))
            .await
            .contains(REASON)
    );
    // The player in Play was not kicked: no Play disconnect can be waiting for it.
    assert!(
        playing
            .wait_for_frame_id_with_timeout(PlayDisconnect::ID, Duration::from_secs(2))
            .await
            .is_err(),
        "the session in Play keeps playing"
    );
    server.hub.shutdown();
    tasks.into_iter().for_each(|task| task.abort());
}

/// D3: with the server full, a same-profile login is refused (vanilla `canPlayerLogin` runs before
/// `disconnectAllPlayersWithProfile`) and the session in Play is not kicked.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn full_server_refuses_a_same_account_login_before_kicking_the_session_in_play() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path(), 1);
    let (addr, tasks) = listen(&server).await;
    let mut playing = enter_play(addr, "Dup").await;
    let mut refused = Client::connect(addr).await.unwrap();
    refused.drive_login(addr, "Dup").await.unwrap();
    let mut frame = refused
        .wait_for_frame_id_with_timeout(ConfigurationDisconnect::ID, WAIT)
        .await
        .expect("the full server refuses the login");
    let reason = ConfigurationDisconnect::decode(&mut frame.body).unwrap().reason_nbt;
    assert!(String::from_utf8_lossy(&reason).contains("The server is full!"));
    assert!(
        playing
            .wait_for_frame_id_with_timeout(PlayDisconnect::ID, Duration::from_secs(2))
            .await
            .is_err(),
        "the refused login must not kick the session in Play"
    );
    assert_eq!(
        server
            .admin
            .sessions_for_uuid(crate::crypto::offline_uuid("Dup"))
            .len(),
        1
    );
    server.hub.shutdown();
    tasks.into_iter().for_each(|task| task.abort());
}
