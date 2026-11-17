use super::*;

fn server(root: &std::path::Path) -> EdgeServer {
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
    let data = crate::server::load_vanilla_data(&cfg).unwrap();
    let tags = crate::server::load_tags(&cfg, &data).unwrap();
    EdgeServer::new(core, data, tags, cfg).unwrap()
}

/// A core that halts itself (journal append failure, fail closed) queues a
/// terminal `Disconnect{reason}` per connection in its reliable outbox. The
/// pump must still route it when the tick broadcast closes: a bare socket
/// close left clients (and the log) without the cause.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn core_halt_routes_terminal_disconnect_reason() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let journal = server.cfg.data.world_dir.join("journal");
    let core = Arc::clone(&server.hub);
    let id = server.connections.joining_id();
    let (reliable_tx, mut reliable_rx) = mpsc::channel(256);
    let (world_tx, _world_rx) = world_queue(256, WORLD_QUEUE_BYTES);
    server
        .connections
        .register(id, ConnectionHandles::new(reliable_tx, world_tx), || {
            server.hub.published_knockback_count(id)
        })
        .unwrap();
    assert!(core.submit(mc_core::boundary::PlayerInput::Join {
        connection: id,
        uuid: uuid::Uuid::new_v4(),
        name: "halt".to_owned(),
        permission_level: 0,
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let serve = tokio::spawn(crate::server::serve(listener, server));
    // The player is live once its join burst is routed.
    tokio::time::timeout(std::time::Duration::from_secs(60), reliable_rx.recv())
        .await
        .expect("join burst")
        .expect("reliable queue open");
    // Real failure, no hook: the next WAL append cannot create its batch file.
    std::fs::rename(&journal, journal.with_file_name("journal-gone")).unwrap();
    let needle = b"journal failed: world halted";
    let found = tokio::time::timeout(std::time::Duration::from_secs(60), async {
        while let Some(packet) = reliable_rx.recv().await {
            if packet.windows(needle.len()).any(|window| window == needle) {
                return true;
            }
        }
        false
    })
    .await
    .expect("halt reaches the pump");
    assert!(found, "terminal Disconnect carries the halt reason");
    let ended = tokio::time::timeout(std::time::Duration::from_secs(60), serve)
        .await
        .expect("serve returns after the core halts")
        .unwrap();
    assert!(ended.is_err(), "a dead core ends the server with an error");
    core.shutdown();
}

/// A halted core whose pump is wedged (a task the test holds in `pending`) must
/// still end `serve` and tell every connection why. The terminal Disconnects
/// ride the reliable outbox that only the pump drains, so the halt path gives
/// the pump a bounded chance, aborts it, and kicks each connection itself.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wedged_pump_still_ends_serve_and_kicks_every_connection() {
    let dir = tempfile::tempdir().unwrap();
    let server = Arc::new(server(dir.path()));
    let journal = server.cfg.data.world_dir.join("journal");
    let core = Arc::clone(&server.hub);
    let id = server.connections.joining_id();
    let (reliable_tx, mut reliable_rx) = mpsc::channel(256);
    let (world_tx, _world_rx) = world_queue(256, WORLD_QUEUE_BYTES);
    server
        .connections
        .register(id, ConnectionHandles::new(reliable_tx, world_tx), || {
            server.hub.published_knockback_count(id)
        })
        .unwrap();
    assert!(core.submit(mc_core::boundary::PlayerInput::Join {
        connection: id,
        uuid: uuid::Uuid::new_v4(),
        name: "halt".to_owned(),
        permission_level: 0,
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    // The pump never runs a tick and never drains the outbox.
    let pump = tokio::spawn(std::future::pending::<()>());
    let serve = tokio::spawn(crate::server::serve_with_wedged_pump(
        listener,
        Arc::clone(&server),
        pump,
    ));
    // Real failure, no hook: the next WAL append cannot create its batch file.
    std::fs::rename(&journal, journal.with_file_name("journal-gone")).unwrap();
    let ended = tokio::time::timeout(std::time::Duration::from_secs(60), serve)
        .await
        .expect("a wedged pump must not keep serve alive after a core halt")
        .unwrap();
    assert!(ended.is_err(), "a dead core ends the server with an error");
    // The kick carries the same reason the terminal batch does.
    let needle = b"journal failed: world halted";
    let mut found = false;
    while let Ok(Some(packet)) =
        tokio::time::timeout(std::time::Duration::from_secs(10), reliable_rx.recv()).await
    {
        if packet.windows(needle.len()).any(|window| window == needle) {
            found = true;
            break;
        }
    }
    assert!(found, "the kicked connection carries the halt reason");
    core.shutdown();
}

/// The real pump parked inside a synchronous section cannot be aborted: the
/// test arms the pump gate (the shape of a fanout `install`) and the pump then
/// holds its task thread without reaching an await point. The halt path must
/// run its bounds, deliver the reason itself and end `serve`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pump_parked_in_a_synchronous_section_still_ends_serve() {
    let dir = tempfile::tempdir().unwrap();
    let server = Arc::new(server(dir.path()));
    let journal = server.cfg.data.world_dir.join("journal");
    let core = Arc::clone(&server.hub);
    let id = server.connections.joining_id();
    let (reliable_tx, mut reliable_rx) = mpsc::channel(256);
    let (world_tx, _world_rx) = world_queue(256, WORLD_QUEUE_BYTES);
    server
        .connections
        .register(id, ConnectionHandles::new(reliable_tx, world_tx), || {
            server.hub.published_knockback_count(id)
        })
        .unwrap();
    assert!(core.submit(mc_core::boundary::PlayerInput::Join {
        connection: id,
        uuid: uuid::Uuid::new_v4(),
        name: "halt".to_owned(),
        permission_level: 0,
    }));
    // Park the pump in its gate: a synchronous section no abort can reach.
    let (entered_tx, entered) = std::sync::mpsc::channel::<()>();
    let release = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let parked = Arc::clone(&release);
    crate::server::arm_pump_gate(
        &server,
        Arc::new(move || {
            let _ = entered_tx.send(());
            let (lock, wake) = &*parked;
            let mut released = lock.lock().unwrap();
            while !*released {
                released = wake.wait(released).unwrap();
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let pump = crate::server::spawn_pump(Arc::clone(&server)).unwrap();
    let serve = tokio::spawn(crate::server::serve_with_wedged_pump(
        listener,
        Arc::clone(&server),
        pump,
    ));
    entered
        .recv_timeout(std::time::Duration::from_secs(60))
        .expect("the pump reaches its gate");
    // Real failure, no hook: the next WAL append cannot create its batch file.
    std::fs::rename(&journal, journal.with_file_name("journal-gone")).unwrap();
    let ended = tokio::time::timeout(std::time::Duration::from_secs(120), serve)
        .await
        .expect("a parked pump must not keep serve alive after a core halt")
        .unwrap();
    assert!(ended.is_err(), "a dead core ends the server with an error");
    let needle = b"journal failed: world halted";
    let mut found = false;
    while let Ok(Some(packet)) =
        tokio::time::timeout(std::time::Duration::from_secs(10), reliable_rx.recv()).await
    {
        if packet.windows(needle.len()).any(|window| window == needle) {
            found = true;
            break;
        }
    }
    assert!(found, "the kicked connection carries the halt reason");
    // Release the parked pump so the task ends instead of leaking the gate.
    let (lock, wake) = &*release;
    *lock.lock().unwrap() = true;
    wake.notify_all();
    crate::server::disarm_pump_gate();
    core.shutdown();
}
