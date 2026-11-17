//! The halt's last resort, outside the edge runtime.
//!
//! `serve` handles a core halt on the edge runtime: bounded waits, kicks, then
//! an error that ends the process. Those bounds are runtime timers, and a timer
//! fires only while some runtime worker is free to drive it. A pump parked in a
//! synchronous section on the only worker (one CPU, `TOKIO_WORKER_THREADS=1`),
//! or every worker blocked behind a lock the pump holds, starves them, and the
//! process would outlive its core. This OS thread waits for the core to close
//! and, after a halt, gives `serve` [`HALT_EXIT_BOUND`] to return; when it has
//! not, it joins the core's owners (the journal's final flush) and exits the
//! process with status 1.

use mc_core::Core;
use mc_core::boundary::TickOutput;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::Duration;
use tokio::sync::broadcast;

/// Time a halted core gives `serve` to finish its own halt path (whose bounds
/// total 25 s) before the watchdog ends the process.
pub(super) const HALT_EXIT_BOUND: Duration = Duration::from_secs(if cfg!(test) { 20 } else { 60 });

/// Start the watchdog for `core`. The returned sender is `serve`'s liveness:
/// dropping it (serve returned, or its future was dropped) releases the
/// watchdog without action.
pub(super) fn spawn_halt_watchdog(core: Arc<Core>) -> std::io::Result<Sender<()>> {
    let (serving, done) = std::sync::mpsc::channel();
    let ticks = core.subscribe();
    std::thread::Builder::new()
        .name("edge-halt-watchdog".to_owned())
        .spawn(move || watch(&core, ticks, &done))?;
    Ok(serving)
}

fn watch(core: &Core, mut ticks: broadcast::Receiver<TickOutput>, serving: &Receiver<()>) {
    // Ticks and lag are not the signal; only `Closed` ends this.
    while let Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) = ticks.blocking_recv() {}
    // A requested shutdown runs its own final checkpoint, however long.
    if core.shutdown_requested() {
        return;
    }
    if let Err(RecvTimeoutError::Timeout) = serving.recv_timeout(HALT_EXIT_BOUND) {
        tracing::error!(
            reason = core.halt_reason().as_deref().unwrap_or("the core stopped"),
            bound_s = HALT_EXIT_BOUND.as_secs(),
            "edge did not finish the core halt in time; exiting the process"
        );
        // Join the tick and journal owners first: the journal's final flush.
        core.shutdown();
        std::process::exit(1);
    }
}
