//! `krasnokamen`: the v2 edge server binary.
//!
//! Serves with `--config PATH`, with stdin commands unless `--no-console`.
//! Offline content, backup/restore and Anvil conversion commands run before
//! starting core owners or a listener. Signals and console `stop` request a
//! clean shutdown and final native checkpoint.

use clap::Parser;
use mc_edge::operator_cli::Cli;
use tracing::info;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();
    let cli = Cli::parse();
    let cfg = mc_edge::operator_cli::load_config(&cli)?;
    // A halt shuts the runtime down in the background: a worker parked in a
    // synchronous section (the fanout pool, a held lock) must not keep the
    // process alive after every player has been disconnected.
    let result = mc_edge::run_on_edge_runtime(run(cli, cfg));
    if let Err(error) = &result {
        tracing::error!(error = %format_args!("{error:#}"), "server stopped with error");
    }
    result
}

async fn run(cli: Cli, mut cfg: mc_edge::config::EdgeConfig) -> anyhow::Result<()> {
    if mc_edge::operator_cli::run_offline(&cli, &mut cfg).await? {
        return Ok(());
    }
    mc_edge::content_cache::prepare_startup(&mut cfg).await?;
    let data = mc_edge::server::load_vanilla_data(&cfg)?;
    let tags = mc_edge::server::load_tags(&cfg, &data)?;
    let mut core_config = mc_edge::server::core_config(&cfg);
    core_config.start_frozen = cli.start_frozen;
    // Registered before the core opens: a stop requested during startup must
    // still reach the clean shutdown below instead of the default signal kill.
    let shutdown = shutdown_signal();
    // Recovery can wait for chunk workers while folding durable block overlays.
    let core = std::sync::Arc::new(
        tokio::task::spawn_blocking(move || mc_core::Core::open(core_config)).await??,
    );
    if let Some(cows) = cli.bench_seed_cows {
        let seed_core = std::sync::Arc::clone(&core);
        tokio::task::spawn_blocking(move || mc_core::bench_seed(&seed_core, cows, true, 0)).await?;
        let [spawn_x, spawn_y, spawn_z] = core.world_spawn();
        info!(cows, spawn_x, spawn_y, spawn_z, "bench seed spawned");
    }
    let shutdown_core = std::sync::Arc::clone(&core);
    let dashboard = cfg.dashboard.enabled.then(|| {
        (
            cfg.dashboard.bind_address.clone(),
            cfg.dashboard.port,
            mc_edge::server::dashboard_allowed(&cfg),
        )
    });

    // Do not expose a listening socket until the edge and legacy profiles are ready.
    let server = mc_edge::server::EdgeServer::new(core, data, tags, cfg)?;
    let listener = tokio::net::TcpListener::bind(server.cfg.bind_addr()).await?;
    info!(
        addr = %listener.local_addr()?,
        name = %server.cfg.server.name,
        "Krasnokamen is listening (edge v2)"
    );
    // Serve the state the tick pump updates (owned by the server).
    if let Some((addr, port, allowed)) = dashboard {
        if allowed {
            info!(addr = %addr, port, "dashboard listening");
            let state = std::sync::Arc::clone(&server.dashboard);
            tokio::spawn(mc_edge::dashboard::serve(addr, port, state));
        } else {
            tracing::warn!(
                addr = %addr,
                "dashboard enabled with non-loopback bind but allow_remote=false; dashboard not started"
            );
        }
    }
    tokio::select! {
        biased;
        result = mc_edge::server::serve_with_console(listener, server, !cli.no_console) => {
            // Record the edge result before shutdown drops the tick broadcast.
            match &result {
                Ok(()) => info!(cause = "edge serve loop returned", "server stopping"),
                Err(error) => tracing::error!(error = %format_args!("{error:#}"), "edge failed; server stopping"),
            }
            spawn_stop_escalation();
            shutdown_core.shutdown();
            result
        }
        cause = shutdown => {
            info!(cause, "shutdown requested; stopping core");
            // The final checkpoint can take minutes on a big dirty set; a
            // second stop signal must not be swallowed now that the first
            // listener is gone.
            spawn_stop_escalation();
            shutdown_core.shutdown();
            info!(cause, "shutdown requested; core stopped");
            // A signal that arrives while `serve` handles a halt drops that
            // future: keep the halt as the outcome, so the runtime is shut down
            // in the background instead of joining a wedged pump.
            if let Some(reason) = shutdown_core.halt_reason() {
                anyhow::bail!("core halted: {reason}");
            }
            Ok(())
        }
    }
}

/// Exit code for a second stop signal: 130 is the Ctrl-C convention, a
/// non-zero status distinguishable from a clean stop and from a crash.
const SECOND_STOP_EXIT_CODE: i32 = 130;

/// Arm the escalation path while a clean shutdown runs: a second SIGTERM or
/// SIGINT logs one line and exits immediately instead of waiting for the final
/// checkpoint. Safe because the WAL is crash-consistent — the next start
/// replays it, exactly as after a power loss.
fn spawn_stop_escalation() {
    spawn_stop_escalation_with(|cause| {
        tracing::error!(
            cause,
            code = SECOND_STOP_EXIT_CODE,
            "second stop signal: exiting without the final checkpoint; the WAL replays on the next start"
        );
        std::process::exit(SECOND_STOP_EXIT_CODE);
    });
}

/// [`spawn_stop_escalation`] with the exit replaced, so a test can observe the
/// signal half in-process. The listener is registered synchronously here, before
/// the caller blocks: a signal that arrives during the first checkpoint still
/// lands on it.
fn spawn_stop_escalation_with(on_second: impl FnOnce(&'static str) + Send + 'static) {
    let second = shutdown_signal();
    tokio::spawn(async move {
        let cause = second.await;
        on_second(cause);
    });
}

/// Ctrl-C or SIGTERM: either requests shutdown, then the process exits.
/// Handlers are installed when this is called, not when the future is polled.
fn shutdown_signal() -> impl Future<Output = &'static str> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let streams = signal(SignalKind::terminate())
            .and_then(|term| signal(SignalKind::interrupt()).map(|interrupt| (term, interrupt)));
        async move {
            match streams {
                Ok((mut term, mut interrupt)) => {
                    tokio::select! {
                        _ = term.recv() => "SIGTERM",
                        _ = interrupt.recv() => "SIGINT",
                    }
                }
                Err(error) => {
                    tracing::error!(%error, "could not register shutdown signals; falling back to Ctrl-C");
                    match tokio::signal::ctrl_c().await {
                        Ok(()) => "Ctrl-C",
                        Err(error) => {
                            tracing::error!(%error, "shutdown signal listener failed");
                            "shutdown signal listener failed"
                        }
                    }
                }
            }
        }
    }
    #[cfg(not(unix))]
    async {
        match tokio::signal::ctrl_c().await {
            Ok(()) => "Ctrl-C",
            Err(error) => {
                tracing::error!(%error, "shutdown signal listener failed");
                "shutdown signal listener failed"
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    #[tokio::test]
    async fn shutdown_signal_reports_sigterm() {
        // Registration is synchronous, before the signal is delivered.
        let shutdown = super::shutdown_signal();
        let sent = std::process::Command::new("kill")
            .args(["-TERM", &std::process::id().to_string()])
            .status()
            .expect("send test SIGTERM");
        assert!(sent.success());
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(5), shutdown)
                .await
                .expect("shutdown signal"),
            "SIGTERM"
        );
    }

    /// After the first stop signal has started the clean shutdown, a second one
    /// must reach the escalation listener: before the fix the listener was
    /// dropped with the completed select, so only SIGKILL could end the wait for
    /// the final checkpoint.
    #[tokio::test]
    async fn second_stop_signal_reaches_the_escalation_listener() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        super::spawn_stop_escalation_with(move |cause| {
            let _ = tx.send(cause);
        });
        let sent = std::process::Command::new("kill")
            .args(["-TERM", &std::process::id().to_string()])
            .status()
            .expect("send test SIGTERM");
        assert!(sent.success());
        let cause = tokio::time::timeout(std::time::Duration::from_secs(5), rx)
            .await
            .expect("second stop signal is heard while shutting down")
            .expect("escalation listener reports the cause");
        assert_eq!(cause, "SIGTERM");
    }
}
