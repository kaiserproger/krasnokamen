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

/// Releases a parked pump and disarms the gate even when an assertion fails
/// first: a gate left armed would park a later test's pump forever.
struct PumpGateGuard(Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>);

impl Drop for PumpGateGuard {
    fn drop(&mut self) {
        let (lock, wake) = &*self.0;
        *lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        wake.notify_all();
        crate::server::disarm_pump_gate();
    }
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

/// The halt reason is the core's own, never another `Disconnect` still queued
/// behind a wedged pump: here the outbox holds only a duplicate-login reason
/// for a connection the pump never routed, and the halt must not hand that
/// text to every connection (or to the server's error) as its own.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wedged_pump_halt_uses_the_core_reason_not_a_queued_disconnect() {
    let dir = tempfile::tempdir().unwrap();
    let server = Arc::new(server(dir.path()));
    let journal = server.cfg.data.world_dir.join("journal");
    let core = Arc::clone(&server.hub);
    let first = server.connections.joining_id();
    let (reliable_tx, mut reliable_rx) = mpsc::channel(256);
    let (world_tx, _world_rx) = world_queue(256, WORLD_QUEUE_BYTES);
    server
        .connections
        .register(first, ConnectionHandles::new(reliable_tx, world_tx), || {
            server.hub.published_knockback_count(first)
        })
        .unwrap();
    let mut ticks = core.subscribe();
    let uuid = uuid::Uuid::new_v4();
    let second = server.connections.joining_id();
    assert!(core.submit(mc_core::boundary::PlayerInput::Join {
        connection: first,
        uuid,
        name: "halt".to_owned(),
        permission_level: 0,
    }));
    // A newer login of the same profile displaces the first (a queued
    // DUPLICATE_LOGIN Disconnect) and never reaches Play: no live connection
    // remains for the halt, so the only queued Disconnect is not the halt's.
    // (A second `Join` would be refused instead and leave the first in Play.)
    assert!(core.submit(mc_core::boundary::PlayerInput::ProfileLogin {
        connection: second,
        uuid,
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let pump = tokio::spawn(std::future::pending::<()>());
    let serve = tokio::spawn(crate::server::serve_with_wedged_pump(
        listener,
        Arc::clone(&server),
        pump,
    ));
    // Inputs submitted before a tick starts are consumed by it: after three
    // ticks both inputs have been applied.
    for _ in 0..3 {
        tokio::time::timeout(std::time::Duration::from_secs(60), ticks.recv())
            .await
            .expect("the core ticks")
            .expect("the core is running");
    }
    // Real failure, no hook: the next WAL append cannot create its batch file.
    std::fs::rename(&journal, journal.with_file_name("journal-gone")).unwrap();
    let ended = tokio::time::timeout(std::time::Duration::from_secs(60), serve)
        .await
        .expect("a wedged pump must not keep serve alive after a core halt")
        .unwrap();
    let error = format!(
        "{:#}",
        ended.expect_err("a dead core ends the server with an error")
    );
    assert!(
        error.contains("journal failed: world halted"),
        "serve reports the core's halt reason: {error}"
    );
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
    assert!(found, "the kick carries the core's halt reason");
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
    let gate = PumpGateGuard(Arc::clone(&release));
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
    drop(gate);
    core.shutdown();
}

/// Set in the child process that
/// `a_halt_on_a_single_blocked_worker_still_ends_the_process` spawns.
const SINGLE_WORKER_CHILD: &str = "CORESERVECLOSE_SINGLE_WORKER_CHILD";
/// The child's stage markers on stdout, in order.
const CHILD_STATUS_ANSWERED: &str = "single-worker child: status answered";
const CHILD_PUMP_PARKED: &str = "single-worker child: pump parked";
const CHILD_HALT_ARMED: &str = "single-worker child: halt armed";

/// Handshake into the status state plus the status request, as a client sends them.
fn status_request_frames(port: u16) -> Vec<u8> {
    use mc_protocol::frame::{Compression, append_frame};
    use mc_protocol::packets::handshake::{Handshake, NextState};
    use mc_protocol::packets::status::StatusRequest;
    let mut frames = bytes::BytesMut::new();
    let mut body = Vec::new();
    Handshake {
        protocol_version: mc_protocol::PROTOCOL_VERSION,
        server_address: "localhost".into(),
        server_port: port,
        next_state: NextState::Status,
    }
    .encode(&mut body)
    .unwrap();
    append_frame(&mut frames, Handshake::ID, &body, Compression::Disabled).unwrap();
    body.clear();
    StatusRequest.encode(&mut body).unwrap();
    append_frame(&mut frames, StatusRequest::ID, &body, Compression::Disabled).unwrap();
    frames.to_vec()
}

/// The scenario, run in a child with one runtime worker: a live connection task
/// waits for its client's ping, then the real pump parks in a synchronous
/// section on that only worker and the core halts. `serve`'s halt bounds are
/// runtime timers no worker is left to drive, so only the halt watchdog `serve`
/// started can end this process (status 1).
#[test]
fn single_blocked_worker_child() {
    if std::env::var_os(SINGLE_WORKER_CHILD).is_none() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_owned();
    let result = crate::run_on_edge_runtime(async move {
        let server = Arc::new(server(&root));
        let journal = server.cfg.data.world_dir.join("journal");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let park = Arc::new(AtomicBool::new(false));
        let (entered_tx, entered) = std::sync::mpsc::channel::<()>();
        let parking = Arc::clone(&park);
        crate::server::arm_pump_gate(
            &server,
            Arc::new(move || {
                if parking.load(Ordering::Acquire) {
                    let _ = entered_tx.send(());
                    // A synchronous section nothing releases.
                    loop {
                        std::thread::park();
                    }
                }
            }),
        );
        let pump = crate::server::spawn_pump(Arc::clone(&server)).unwrap();
        std::thread::spawn(move || {
            use std::io::Read;
            let mut client = std::net::TcpStream::connect(address).unwrap();
            client
                .write_all(&status_request_frames(address.port()))
                .unwrap();
            let mut first = [0u8; 1];
            client
                .read_exact(&mut first)
                .expect("the connection task answers the status request");
            println!("{CHILD_STATUS_ANSWERED}");
            // The connection task now waits for a ping that never comes.
            park.store(true, Ordering::Release);
            entered.recv().expect("the pump parks in its gate");
            println!("{CHILD_PUMP_PARKED}");
            // Real failure, no hook: the next WAL append cannot create its batch file.
            std::fs::rename(&journal, journal.with_file_name("journal-gone")).unwrap();
            println!("{CHILD_HALT_ARMED}");
            // Keep the connection open until the process ends.
            loop {
                std::thread::park();
            }
        });
        crate::server::serve_with_wedged_pump(listener, server, pump).await
    });
    // Reaching this line means `serve` finished on its own: the child exits 0,
    // which the parent reports as the scenario not being reproduced.
    println!("serve returned: {result:?}");
}

/// One runtime worker, the pump parked on it, a connection task pending and
/// the core halted: the process still ends (status 1) through the watchdog
/// `serve` starts, independent of the blocked runtime.
#[test]
fn a_halt_on_a_single_blocked_worker_still_ends_the_process() {
    use std::io::BufRead;
    let exe = std::env::current_exe().expect("test binary path");
    let mut child = std::process::Command::new(exe)
        .args([
            "--exact",
            "server::core_halt_tests::single_blocked_worker_child",
            "--nocapture",
        ])
        .env(SINGLE_WORKER_CHILD, "1")
        .env("TOKIO_WORKER_THREADS", "1")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .expect("spawn the single-worker child");
    let stdout = child.stdout.take().expect("child stdout");
    // One message per stdout line, then `None` when the child's stdout closes
    // (it exited).
    let (line_tx, lines) = std::sync::mpsc::channel::<Option<String>>();
    let reader = std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            let _ = line_tx.send(Some(line));
        }
        let _ = line_tx.send(None);
    });
    fn fail(child: &mut std::process::Child, seen: &[String], why: &str) -> ! {
        let _ = child.kill();
        let _ = child.wait();
        panic!("{why}; child said: {seen:?}");
    }
    let mut seen = Vec::new();
    // Setup (vanilla data, core open, status answer, pump parked) varies with
    // host load; the watchdog's bound starts only once the halt is armed.
    let setup = std::time::Instant::now() + std::time::Duration::from_secs(600);
    loop {
        let left = setup.saturating_duration_since(std::time::Instant::now());
        match lines.recv_timeout(left) {
            Ok(Some(line)) => {
                let armed = line == CHILD_HALT_ARMED;
                seen.push(line);
                if armed {
                    break;
                }
            }
            Ok(None) => fail(&mut child, &seen, "the child exited before arming the halt"),
            Err(_) => fail(&mut child, &seen, "the child never armed the halt"),
        }
    }
    let exit = std::time::Instant::now()
        + halt_watchdog::HALT_EXIT_BOUND
        + std::time::Duration::from_secs(60);
    loop {
        let left = exit.saturating_duration_since(std::time::Instant::now());
        match lines.recv_timeout(left) {
            Ok(Some(line)) => seen.push(line),
            Ok(None) => break,
            Err(_) => fail(
                &mut child,
                &seen,
                "a halt on a single blocked runtime worker kept the process alive",
            ),
        }
    }
    let status = child.wait().expect("reap the single-worker child");
    assert_eq!(
        status.code(),
        Some(1),
        "the halt watchdog ends the process with status 1: {status}; child said: {seen:?}"
    );
    let _ = reader.join();
}
