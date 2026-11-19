//! Accept-loop guard: a pump that ends is a server-wide failure, not a silent
//! "serve nothing" state. Every client frame comes from the pump, so serving
//! after it is gone freezes every viewer without a log line.
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

/// A pump that returns (its broadcast closed) must end `serve` with the
/// explicit pump error instead of accepting connections that never receive a
/// tick.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn finished_pump_stops_the_server_with_the_pump_error() {
    let dir = tempfile::tempdir().unwrap();
    let server = Arc::new(server(dir.path()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let pump = tokio::spawn(async {});
    let ended = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        serve_with_pump(listener, Arc::clone(&server), false, Some(pump)),
    )
    .await
    .expect("serve returns once its pump is gone")
    .expect_err("a finished pump ends the server");
    assert!(
        format!("{ended:#}").contains("edge pump stopped"),
        "the accept loop names the pump as the cause: {ended:#}"
    );
    server.hub.shutdown();
}

/// A pump that panics (any unexpected panic in planning) must end `serve` with
/// the panic as the cause, not leave the task dead and the listener alive.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn panicked_pump_stops_the_server_with_its_cause() {
    let dir = tempfile::tempdir().unwrap();
    let server = Arc::new(server(dir.path()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let pump = tokio::spawn(async { panic!("pump publish panicked") });
    let ended = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        serve_with_pump(listener, Arc::clone(&server), false, Some(pump)),
    )
    .await
    .expect("serve returns once its pump is gone")
    .expect_err("a panicked pump ends the server");
    assert!(
        format!("{ended:#}").contains("edge pump died"),
        "the accept loop reports the panic as the cause: {ended:#}"
    );
    server.hub.shutdown();
}
