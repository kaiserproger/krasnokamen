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
/// signal half in-process.
///
/// The listener runs on its own OS thread with a private current-thread
/// runtime, never as a task on the edge runtime: the shutdown it has to
/// interrupt is exactly the case where no edge worker is free (a synchronous
/// section holding the only worker, the failure mode the halt path guards
/// against), and a `tokio::spawn` task would never be polled there. The signal
/// stream is registered synchronously, before the caller blocks on the final
/// checkpoint, so a signal that arrives then still lands on it.
fn spawn_stop_escalation_with(on_second: impl FnOnce(&'static str) + Send + 'static) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!(%error, "second stop signal listener unavailable");
            return;
        }
    };
    // Registration needs the runtime that owns the signal driver; the driver
    // itself is polled on the new thread.
    let second = {
        let _context = runtime.enter();
        shutdown_signal()
    };
    let thread = std::thread::Builder::new()
        .name("edge-stop-escalation".to_owned())
        .spawn(move || {
            let cause = runtime.block_on(second);
            on_second(cause);
        });
    if let Err(error) = thread {
        tracing::error!(%error, "second stop signal listener thread unavailable");
    }
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

    /// Child side of [`second_stop_signal_survives_a_wedged_runtime`]: arms the
    /// production escalation, wedges the runtime's only worker inside a
    /// synchronous section (the core-halt failure mode: nothing on the edge
    /// runtime can run any more), and announces readiness. Only the next
    /// SIGTERM can end this process, and it must do so with code 130.
    #[test]
    fn escalation_child_wedges_its_only_worker() {
        use std::io::Write;
        if std::env::var_os("EH_ESCALATION_CHILD").is_none() {
            return;
        }
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("child runtime");
        runtime.block_on(async {
            super::spawn_stop_escalation();
            tokio::spawn(async {
                println!("EH_READY");
                std::io::stdout().flush().expect("child readiness flush");
                // A synchronous section no runtime timer can escape.
                loop {
                    std::thread::park();
                }
            });
            std::future::pending::<()>().await;
        });
    }

    /// The escalation must not depend on an edge worker: with the only worker
    /// blocked in a synchronous section, a `tokio::spawn` listener is never
    /// polled and the process outlives the stop request.
    #[test]
    fn second_stop_signal_survives_a_wedged_runtime() {
        use std::io::BufRead;
        let exe = std::env::current_exe().expect("test binary");
        let mut child = std::process::Command::new(exe)
            .args([
                "tests::escalation_child_wedges_its_only_worker",
                "--exact",
                "--nocapture",
            ])
            .env("EH_ESCALATION_CHILD", "1")
            .env("TOKIO_WORKER_THREADS", "1")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn escalation child");
        let pid = child.id();
        // Readiness is a line from the child, not a delay.
        let stdout = child.stdout.take().expect("child stdout");
        let mut lines = std::io::BufReader::new(stdout).lines();
        loop {
            match lines.next() {
                Some(Ok(line)) if line.contains("EH_READY") => break,
                Some(Ok(_)) => continue,
                other => panic!("child ended before readiness: {other:?}"),
            }
        }
        let sent = std::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .expect("send child SIGTERM");
        assert!(sent.success());
        // Exit is the event; the receiver bounds the wait, the thread reaps.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(child.wait());
        });
        let status = match rx.recv_timeout(std::time::Duration::from_secs(60)) {
            Ok(status) => status.expect("wait for the escalation child"),
            Err(_) => {
                let _ = std::process::Command::new("kill")
                    .args(["-KILL", &pid.to_string()])
                    .status();
                panic!("a wedged worker starved the second-signal escalation");
            }
        };
        assert_eq!(
            status.code(),
            Some(super::SECOND_STOP_EXIT_CODE),
            "the escalation exits with 130: {status:?}"
        );
    }
}
