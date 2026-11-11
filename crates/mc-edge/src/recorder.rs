//! Recorder connections: a camera client that records the exact server
//! stream for offline replay (battle video, `.solrec` written client-side).
//!
//! A login name listed in `[recorder] players`, connecting directly from a
//! loopback address (any address with `allow_remote = true`), gets for that
//! connection only:
//! - the server's full `view_distance`, whatever its client requests;
//! - every entity type tracked to that distance (the vanilla per-type
//!   ranges, such as arrows at 4 chunks, do not apply; types vanilla never
//!   sends stay unsent);
//! - a larger world queue ([`crate::server::RECORDER_WORLD_QUEUE_FRAMES`]
//!   frames, `world_queue_bytes` bytes), so a loopback recorder is never thinned;
//! - a `solaris:tick` custom payload as the first world frame of every
//!   publish that has a free world-queue frame slot: `tick` then `wall_ns`,
//!   both u64 big-endian, where `wall_ns` is the core tick's UNIX wall start
//!   (`wall_start_unix_ns` of the same tick in `/load-metrics` and
//!   `/telemetry.jsonl`). Every world frame after marker T and before the
//!   next marker was planned from tick T, except `solaris:lod/tile` frames,
//!   which another task queues when their tile is ready. Reliable-queue
//!   packets are not ordered against the world queue.
//!
//! Every publish with a recorder also reports [`RecorderTick`] to
//! `/telemetry.jsonl` (`recorders`): entity work left without world credit,
//! which a complete recording keeps at zero.
//!
//! Everyone else keeps vanilla tracking and the player queue bounds. With
//! offline-mode auth any client may claim a listed name, so the loopback
//! gate is what keeps strangers out of recorder mode.

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

/// Custom payload channel of the per-tick marker.
pub const TICK_MARKER_CHANNEL: &str = "solaris:tick";

/// `[recorder]`: connections that record the server stream.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RecorderSection {
    /// Login names (ASCII case-insensitive) that join in recorder mode.
    pub players: Vec<String>,
    /// Soft byte bound of a recorder's world queue (default 16 MiB).
    pub world_queue_bytes: usize,
    /// Grant recorder mode to listed names from any address, or through a
    /// proxy; by default only a direct loopback peer gets it.
    pub allow_remote: bool,
}

impl Default for RecorderSection {
    fn default() -> Self {
        Self {
            players: Vec::new(),
            world_queue_bytes: 16 << 20,
            allow_remote: false,
        }
    }
}

impl RecorderSection {
    /// Whether `name` is listed in `players`.
    #[must_use]
    pub fn is_recorder(&self, name: &str) -> bool {
        self.players
            .iter()
            .any(|player| player.eq_ignore_ascii_case(name))
    }

    /// Whether a connection named `name` from TCP peer `peer` joins in
    /// recorder mode: a listed name, and a direct loopback peer (not
    /// `proxied`; IPv4-mapped `::ffff:127.0.0.1` from a dual-stack listener
    /// counts as loopback) unless `allow_remote`.
    #[must_use]
    pub fn grants(&self, name: &str, peer: Option<std::net::IpAddr>, proxied: bool) -> bool {
        self.is_recorder(name)
            && (self.allow_remote
                || (!proxied && peer.is_some_and(|ip| ip.to_canonical().is_loopback())))
    }
}

/// The framed `solaris:tick` marker for one published tick.
pub(crate) fn tick_marker_frame(tick: u64, wall_unix_ns: u64) -> bytes::Bytes {
    static CHANNEL: LazyLock<mc_data::Identifier> = LazyLock::new(|| {
        mc_data::Identifier::parse(TICK_MARKER_CHANNEL).expect("valid tick marker channel")
    });
    let mut payload = Vec::with_capacity(16);
    payload.extend_from_slice(&tick.to_be_bytes());
    payload.extend_from_slice(&wall_unix_ns.to_be_bytes());
    crate::lod::frame(CHANNEL.clone(), payload).expect("a 16-byte custom payload frames")
}

/// Recorder connections' deferred entity work at one publish.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct RecorderTick {
    /// Recorder connections planned this publish.
    pub connections: usize,
    /// Entity removals, spawns and updates left without world credit, over
    /// all recorders; they go out in a later publish.
    pub deferred_updates_sum: usize,
    /// The largest single recorder's count.
    pub deferred_updates_max: usize,
}

impl RecorderTick {
    /// Totals over the recorder viewers just planned; `None` without one.
    pub(crate) fn of<'a>(
        viewers: impl IntoIterator<Item = &'a crate::plan::ViewerState>,
    ) -> Option<Self> {
        let mut tick = Self::default();
        for viewer in viewers.into_iter().filter(|viewer| viewer.recorder) {
            tick.connections += 1;
            tick.deferred_updates_sum += viewer.deferred_updates;
            tick.deferred_updates_max = tick.deferred_updates_max.max(viewer.deferred_updates);
        }
        (tick.connections > 0).then_some(tick)
    }
}

#[cfg(test)]
#[path = "recorder_tests.rs"]
mod recorder_tests;
