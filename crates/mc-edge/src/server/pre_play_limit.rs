//! Server-wide bound on concurrent pre-play connections.
//!
//! The per-address controls ([`crate::conn_limit`] plus the fixed eight
//! simultaneous pre-play sockets per address) stop one host. An IPv6 client
//! normally controls a whole /64 and a botnet has many more sources, while
//! every pre-play socket can hold a task, its stack and up to 1 MiB of decoded
//! buffer. This is the server-wide backstop: a connection refused here is
//! closed before its handshake is read, so it costs one accepted socket and no
//! per-connection task.
//!
//! The bound counts pre-play only: a session that reaches Play drops its slot
//! and is bounded by `server.max_players` instead.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Fixed headroom above `max_players * 2`: status pings, reconnects and a
/// burst of new joins while every player still holds a Play session.
const HEADROOM: usize = 64;

/// Cap derived from `max_players`: the cap counts pre-play connections only
/// (handshake, status, login), and a Play session drops its slot, so the worst
/// legitimate case is every configured player joining at once after a restart or
/// a proxy reconnect, plus [`HEADROOM`] for status pings and join bursts. The
/// derivation is used when `[network] max_pre_play_connections` is 0.
pub(super) fn derived_cap(max_players: u32) -> usize {
    (max_players as usize)
        .saturating_mul(2)
        .saturating_add(HEADROOM)
}

/// Concurrent pre-play admission for one listener.
#[derive(Debug)]
pub(super) struct PrePlayLimit {
    cap: usize,
    live: AtomicUsize,
}

impl PrePlayLimit {
    /// `cap` is clamped to at least one: a configured zero means "derive",
    /// never "refuse every connection".
    pub(super) fn new(cap: usize) -> Self {
        Self {
            cap: cap.max(1),
            live: AtomicUsize::new(0),
        }
    }

    /// The configured non-zero `[network] max_pre_play_connections`, else
    /// [`derived_cap`] of `server.max_players`.
    pub(super) fn resolve(cfg: &crate::config::EdgeConfig) -> Self {
        let configured = cfg.network.max_pre_play_connections as usize;
        let cap = if configured == 0 {
            derived_cap(cfg.server.max_players)
        } else {
            configured
        };
        Self::new(cap)
    }

    /// The bound this listener admits up to.
    pub(super) fn cap(&self) -> usize {
        self.cap
    }

    /// Live pre-play connections (accepted, not yet in Play).
    pub(super) fn live(&self) -> usize {
        self.live.load(Ordering::SeqCst)
    }

    /// Reserve a pre-play slot, or `None` when the listener already holds
    /// [`Self::cap`] of them.
    pub(super) fn try_acquire(self: &Arc<Self>) -> Option<PrePlaySlot> {
        let mut live = self.live.load(Ordering::SeqCst);
        loop {
            if live >= self.cap {
                return None;
            }
            match self.live.compare_exchange_weak(
                live,
                live + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => {
                    return Some(PrePlaySlot {
                        limit: Arc::clone(self),
                    });
                }
                Err(current) => live = current,
            }
        }
    }
}

/// One admitted pre-play connection. Dropped when the session enters Play (or
/// ends), which keeps the bound about pre-play rather than about `max_players`.
#[derive(Debug)]
pub(super) struct PrePlaySlot {
    limit: Arc<PrePlayLimit>,
}

impl Drop for PrePlaySlot {
    fn drop(&mut self) {
        self.limit.live.fetch_sub(1, Ordering::SeqCst);
    }
}
