//! The edge process's runtime: build it once, run the server future, and on an
//! error shut it down without waiting for a worker that a halt left parked.
//!
//! A halted core can leave an edge task inside a synchronous section that no
//! abort can preempt (the fanout pool's `install`, a held registry lock). The
//! ordinary [`Runtime`] drop joins every worker, so the process would stay alive
//! with every player already disconnected. The halt path has delivered their
//! reason by then; this shutdown lets the process exit.

use anyhow::Result;
use std::future::Future;

/// Build the edge runtime and run `future` on it.
///
/// Success drops the runtime normally (the server stopped its owners and every
/// worker is idle). An error — the core halted — shuts the runtime down in the
/// background: workers stuck in a synchronous section are abandoned instead of
/// joined, so the process exits with the error.
pub fn run_on_edge_runtime<F>(future: F) -> Result<()>
where
    F: Future<Output = Result<()>>,
{
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        // Chunk frames are encoded on runtime workers; keep bounded stack headroom.
        .thread_stack_size(16 * 1024 * 1024)
        .build()?;
    let result = runtime.block_on(future);
    if result.is_err() {
        // `shutdown_background` does not join workers or blocking tasks.
        runtime.shutdown_background();
    }
    result
}

#[cfg(test)]
#[path = "edge_runtime_tests.rs"]
mod tests;
