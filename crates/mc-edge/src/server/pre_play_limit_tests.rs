//! Server-wide pre-play cap: unit semantics plus the accept-loop refusal and
//! the established-play exception.
use super::pre_play_limit::{PrePlayLimit, derived_cap};
use super::*;

fn server(root: &std::path::Path, cap: u32) -> EdgeServer {
    server_with_limits(root, cap, 0)
}

fn server_with_limits(root: &std::path::Path, cap: u32, max_per_ip: u32) -> EdgeServer {
    let mut config = mc_core::CoreConfig::new(root.join("world"), 42);
    config.vanilla_data_dir =
        Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla"));
    config.view_distance = 0;
    config.simulation_distance = 0;
    let core = Arc::new(mc_core::Core::open(config).unwrap());
    let mut cfg = crate::config::EdgeConfig {
        server_dir: root.join("server"),
        ..Default::default()
    };
    std::fs::create_dir_all(&cfg.server_dir).unwrap();
    cfg.data.world_dir = root.join("world");
    cfg.data.vanilla_data_dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla");
    cfg.auth.online_mode = false;
    cfg.network.max_pre_play_connections = cap;
    cfg.network.max_connections_per_ip = max_per_ip;
    let data = crate::server::load_vanilla_data(&cfg).unwrap();
    let tags = crate::server::load_tags(&cfg, &data).unwrap();
    EdgeServer::new(core, data, tags, cfg).unwrap()
}

#[test]
fn derived_cap_is_two_per_player_plus_headroom() {
    assert_eq!(derived_cap(0), 64);
    assert_eq!(derived_cap(20), 104);
    // Saturating arithmetic: the largest configured player count must not wrap.
    assert_eq!(derived_cap(u32::MAX), (u32::MAX as usize) * 2 + 64);
}

#[test]
fn cap_refuses_beyond_the_bound_and_releases_on_drop() {
    let limit = Arc::new(PrePlayLimit::new(2));
    let first = limit.try_acquire().expect("first slot");
    let second = limit.try_acquire().expect("second slot");
    assert_eq!(limit.live(), 2);
    assert!(limit.try_acquire().is_none(), "third connection is refused");
    // A closed pre-play connection frees its slot for the next client.
    drop(first);
    let third = limit.try_acquire().expect("released slot is reusable");
    drop((second, third));
    assert_eq!(limit.live(), 0);
}

#[test]
fn zero_config_derives_and_nonzero_overrides() {
    let mut cfg = crate::config::EdgeConfig::default();
    cfg.server.max_players = 5;
    assert_eq!(
        PrePlayLimit::resolve(&cfg).cap(),
        derived_cap(5),
        "0 derives from max_players"
    );
    cfg.network.max_pre_play_connections = 3;
    assert_eq!(PrePlayLimit::resolve(&cfg).cap(), 3, "explicit value wins");
}

/// With every pre-play slot held, one more accepted socket is closed before
/// the handshake, while the session in Play keeps receiving routed frames.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extra_pre_play_socket_is_refused_while_a_player_plays() {
    use tokio::io::AsyncReadExt;
    let dir = tempfile::tempdir().unwrap();
    let server = Arc::new(server(dir.path(), 2));
    let limiter = Arc::clone(&server.pre_play);
    let core = Arc::clone(&server.hub);
    // An established Play session: registered outbound queues, joined in the
    // core, exactly what the play loop owns once the handshake is done.
    let play = server.connections.joining_id();
    let (reliable_tx, mut reliable_rx) = mpsc::channel(256);
    let (world_tx, _world_rx) = world_queue(256, WORLD_QUEUE_BYTES);
    server
        .connections
        .register(
            play,
            ConnectionHandles::new(reliable_tx, world_tx, crate::plan::CHUNKS_PER_TICK),
            || server.hub.published_knockback_count(play),
        )
        .unwrap();
    assert!(core.submit(mc_core::boundary::PlayerInput::Join {
        connection: play,
        uuid: uuid::Uuid::new_v4(),
        name: "resident".to_owned(),
        permission_level: 0,
    }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let serving = tokio::spawn(serve_with_pump(listener, Arc::clone(&server), false, None));
    // The player is live once the join burst is routed.
    tokio::time::timeout(std::time::Duration::from_secs(60), reliable_rx.recv())
        .await
        .expect("join burst reaches the play session")
        .expect("reliable queue open");

    // Two pre-play sockets: connected, no handshake bytes, so they stay in
    // pre-play and hold both slots.
    let mut pre_play = Vec::new();
    for _ in 0..2 {
        let stream = TcpStream::connect(address).await.unwrap();
        pre_play.push(stream);
    }
    wait_for_live(&limiter, 2).await;

    // The third is refused: the listener closes it instead of spawning a task.
    let mut extra = TcpStream::connect(address).await.unwrap();
    let mut byte = [0_u8; 1];
    let refused = tokio::time::timeout(std::time::Duration::from_secs(30), extra.read(&mut byte))
        .await
        .expect("over-cap connection is closed, not left open");
    match refused {
        Ok(0) => {}
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
            ) => {}
        other => panic!("over-cap socket must be closed without a byte, got {other:?}"),
    }
    assert_eq!(limiter.live(), 2, "the refused socket holds no slot");

    // The established player still receives routed reliable frames.
    server
        .feedback(play, "still playing")
        .expect("player is routed");
    assert!(
        reliable_rx.try_recv().is_ok(),
        "the play session keeps receiving while pre-play is refused"
    );
    assert_eq!(
        server.connections.len(),
        1,
        "the refused socket never became a session"
    );

    serving.abort();
    core.shutdown();
}

/// Acceptance is asynchronous: wait for the limiter's own accounting, not a
/// wall-clock delay.
async fn wait_for_live(limit: &PrePlayLimit, want: usize) {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        while limit.live() != want {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("pre-play slots are accounted");
}

/// Connect from a chosen loopback source address, so one test can hold the
/// server-wide cap from one address while knocking from another.
async fn connect_from(local: &str, address: std::net::SocketAddr) -> TcpStream {
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.bind(format!("{local}:0").parse().unwrap()).unwrap();
    socket.connect(address).await.unwrap()
}

/// A refusal by the server-wide cap must not consume a per-address slot: with
/// `max_connections_per_ip > 0` one leaked slot per refusal locks the address
/// out for the rest of the process, and with the default 0 the address map grows
/// without bound — exactly the flood the cap exists to bound.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cap_refusal_keeps_the_address_admissible() {
    use tokio::io::AsyncReadExt;
    let dir = tempfile::tempdir().unwrap();
    let server = Arc::new(server_with_limits(dir.path(), 1, 1));
    let limiter = Arc::clone(&server.pre_play);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let serving = tokio::spawn(serve_with_pump(listener, Arc::clone(&server), false, None));

    // Saturate the cap from another loopback address.
    let holder = connect_from("127.0.0.2", address).await;
    wait_for_live(&limiter, 1).await;

    // Refused by the cap while this address's only per-address slot is free.
    let mut refused = connect_from("127.0.0.1", address).await;
    let mut byte = [0_u8; 1];
    let closed = tokio::time::timeout(std::time::Duration::from_secs(30), refused.read(&mut byte))
        .await
        .expect("the cap closes the over-cap socket");
    match closed {
        Ok(0) => {}
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
            ) => {}
        other => panic!("over-cap socket must be closed without a byte, got {other:?}"),
    }

    // Free the cap; the same address must be admitted again.
    drop(holder);
    wait_for_live(&limiter, 0).await;
    let admitted = connect_from("127.0.0.1", address).await;
    wait_for_live(&limiter, 1).await;
    let mut probe = [0_u8; 1];
    assert!(
        admitted.try_read(&mut probe).is_err(),
        "an admitted pre-play socket stays open instead of being refused by a leaked address slot"
    );

    serving.abort();
    server.hub.shutdown();
}
