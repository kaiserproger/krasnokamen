//! Read-only operator dashboard for the v2 edge server.
//!
//! A tiny tokio TCP HTTP/1.1 endpoint with a request-head cap,
//! one-request-per-connection, `GET /stats`, `GET /profile`, streaming
//! `GET /load-metrics`, `GET /telemetry.jsonl`, and Prometheus `GET /metrics`
//! routes (404 elsewhere).
//! Payloads stay edge-local.
//!
//! The existing `/stats` rolling 1200 samples remain independent of the
//! preallocated 16384-sample `/load-metrics` ring. Stream `?after_tick=N`
//! with the last consumed tick to retain arbitrarily long wall windows.
//! `&wait=true&min_samples=20` waits on pushed sample notifications, batching
//! at least 20 newer samples (range 1..=1200; default 1). A single 10-second
//! deadline returns currently available samples with `wait_timed_out=true`;
//! nonwaiting requests and satisfied waits carry `false`. Overrun returns
//! immediately even below the requested sample threshold. `overrun`
//! reports overwritten requested samples; noncontiguous tick IDs or increases
//! in `source_skipped_ticks_total` also invalidate affected windows.
//!
//! `/telemetry.jsonl` takes the same query and returns one JSON object per
//! retained tick the edge has fanned out (`application/x-ndjson`): the
//! `/load-metrics` sample plus `host`, the [`HostSample`] taken on that tick
//! (one per 20-tick window, otherwise `null`). Samples of ticks with battle
//! units (staged, paused or running) also carry `battle`, counted after
//! merge; ticks with a recorder connection carry `recorders`
//! ([`RecorderTick`]). Missing ticks are gaps, never filled in.
//!
//! Raw samples retain the core's original UNIX-nanosecond wall start (before
//! input drains), not edge receipt time or a reconstructed 50 ms cadence.
//! Use `wall_us` for full active-owner tick percentiles; `total_us` retains
//! the narrower `run_tick` + journal cost. Both durations are monotonic;
//! wall timestamps can step with the system clock. Queue gauges are request-
//! time, non-atomic observations between their capture/completion timestamps,
//! not historical tick samples or socket/kernel buffered-byte measurements.

use crate::host_telemetry::{HostSample, SAMPLE_TICKS};
use crate::recorder::RecorderTick;
use crate::server::ConnectionRegistry;
use mc_core::boundary::TickTimes;
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Notify;

mod http;
mod latency;
mod prometheus;
mod routing;
mod state;

#[cfg(test)]
mod tests;

pub use http::{serve, serve_bound};
pub(crate) use latency::unix_ns;

/// Bounded storage, about 13.6 minutes at 20 TPS; stream longer windows.
const LOAD_SAMPLE_CAPACITY: usize = 16384;

/// Host samples covering the same ticks as the load ring.
const HOST_SAMPLE_CAPACITY: usize = LOAD_SAMPLE_CAPACITY / SAMPLE_TICKS as usize + 1;

/// A long poll waits on pushes, with this deadline as its only timer.
const LOAD_WAIT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a dashboard peer may take to deliver a complete request head.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Request-head read cap: larger heads are rejected with 431.
const MAX_HEAD_BYTES: usize = 8 * 1024;

/// World identity facts served under `world`.
#[derive(Debug, Clone, Serialize, Default)]
pub struct DashboardWorld {
    /// Server display name from `[server] name`.
    pub name: String,
    /// Message of the day from `[server] motd`.
    pub motd: String,
    /// World seed.
    pub seed: i64,
    /// Chunk view radius.
    pub view_distance: u8,
    /// Chunk simulation radius.
    pub simulation_distance: u8,
    /// Maximum concurrent Play sessions.
    pub max_players: u32,
}

/// Connected-player roster served under `players`.
#[derive(Debug, Clone, Serialize, Default)]
pub struct DashboardPlayers {
    /// Current Play session count.
    pub count: usize,
    /// Configured cap.
    pub max: u32,
    /// Online player names, sorted.
    pub names: Vec<String>,
}

/// Read-only cumulative counter source, polled when a dashboard payload is
/// built. The count itself stays in its owner (the core's submit path).
pub type InputDropSource = Arc<dyn Fn() -> u64 + Send + Sync>;

/// Counters committed together after one pump publish.
#[derive(Default)]
pub(crate) struct PumpTick {
    pub tick: u64,
    pub core_us: u64,
    pub publish_us: u64,
    pub bytes_written: u64,
    pub reliable_drops: u64,
    pub chunks: DashboardChunks,
    pub entities: DashboardEntities,
    pub players: Option<DashboardPlayers>,
}

/// Percentiles over the last 1200 ticks, in microseconds.
#[derive(Debug, Clone, Copy, Serialize, Default)]
pub struct TickLatency {
    /// Tick that produced the newest sample in the window.
    pub source_tick: u64,
    /// p50, µs.
    pub p50_us: u64,
    /// p95, µs.
    pub p95_us: u64,
    /// p99, µs.
    pub p99_us: u64,
    /// Maximum in the window, µs.
    pub max_us: u64,
    /// Samples in the window.
    pub samples: usize,
}

/// Chunk pipeline counters.
#[derive(Debug, Clone, Serialize, Default)]
pub struct DashboardChunks {
    /// Chunks resident in the newest core snapshot.
    pub loaded_total: usize,
}

/// Entity population totals.
#[derive(Debug, Clone, Serialize, Default)]
pub struct DashboardEntities {
    /// Simulated entities, players included.
    pub total: usize,
}

/// Network throughput and reliability counters.
#[derive(Debug, Clone, Serialize, Default)]
pub struct DashboardNetwork {
    /// Total bytes written to Play connections.
    pub bytes_written: u64,
    /// Reliable-queue overflows that disconnected a client.
    pub reliable_drops: u64,
    /// Player inputs refused by the bounded core input queue since open
    /// ([`mc_core::Core::dropped_inputs`], read on request). A refused input is
    /// gone: the server corrects the client, so the symptom is rubber-banding.
    pub input_drops: u64,
}

/// Actual production component execution counters, not submissions or bot writes.
#[derive(Debug, Clone, Serialize)]
pub struct PluginStats {
    pub runtime_ready: bool,
    pub loaded_packages: usize,
    pub wasm_invocations: u64,
    pub host_operations: u64,
    pub command_results: u64,
}

/// Opt-in edge-plan observation. Pairs are not distinct entities or delivery acknowledgements.
#[derive(Debug, Clone, Serialize)]
pub struct CorePerfVisibility {
    pub source_tick: u64,
    pub tracked_entity_viewer_pairs: usize,
}

/// One bounded, opt-in pre-measurement union of server-side tracked identities.
#[derive(Debug, Clone, Serialize)]
pub struct CorePerfVisibilityProof {
    pub source_tick: u64,
    pub viewers: usize,
    pub tracked_entity_viewer_pairs: usize,
    pub unique_tracked_entities: Option<usize>,
    pub pair_limit: usize,
}

/// Journal pruning health (process counters, read on demand).
#[derive(Debug, Clone, Copy, Serialize, Default)]
pub struct DashboardJournal {
    /// Failed covered-batch removals since start.
    pub prune_failures_total: u64,
    /// Bytes the latest prune attempt could not remove.
    pub prune_retained_bytes: u64,
}

/// The complete `/stats` payload.
#[derive(Debug, Clone, Serialize)]
pub struct StatsSnapshot {
    /// Crate version string.
    pub version: String,
    /// Seconds since the edge process started.
    pub uptime_secs: u64,
    /// Latest tick number observed by the stats rollup.
    pub simulation_tick: u64,
    /// World identity facts.
    pub world: DashboardWorld,
    /// Connected players.
    pub players: DashboardPlayers,
    /// Core ticks per second over the sample window, from published tick
    /// numbers, so ticks the pump skipped while lagging still count.
    pub tps: f64,
    /// Server-side tick cost (core `run_tick` + journal), from `TickOutput.times`.
    pub tick: TickLatency,
    /// Edge pump fanout time per tick (plan + route to every viewer).
    pub publish: TickLatency,
    /// Core ticks the pump skipped (`broadcast` lag) since start; nonzero
    /// means fanout, not the core, is behind.
    pub pump_skipped_ticks: u64,
    /// Core ticks the pump skipped because they waited past the publish queue budget (the
    /// `stale` path only; a broadcast `Lagged` run counts in `pump_skipped_ticks` alone).
    pub pump_stale_skipped_ticks: u64,
    /// Chunk pipeline counters.
    pub chunks: DashboardChunks,
    /// Entity population totals.
    pub entities: DashboardEntities,
    /// Most recent opt-in fanout observation; tagged with its own source tick.
    pub coreperf_visibility: Option<CorePerfVisibility>,
    /// Most recent requested cold proof, not a client-delivery acknowledgement.
    pub coreperf_visibility_proof: Option<CorePerfVisibilityProof>,
    /// Network counters.
    pub network: DashboardNetwork,
    /// Journal prune health: a failing prune means the WAL grows silently.
    pub journal: DashboardJournal,
    /// Absent when no production component deployment is configured.
    pub plugin: Option<PluginStats>,
    /// Autoscale is not implemented on the edge; always `false`.
    pub autoscale: bool,
    /// Operator warnings (empty unless the backend reports trouble).
    pub warnings: Vec<String>,
}

/// Original per-tick phase costs, without reconstructing missing samples.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct LoadPhaseTimes {
    pub input_us: u64,
    pub chunks_us: u64,
    pub entity_us: u64,
    pub merge_us: u64,
    pub block_us: u64,
    pub commit_us: u64,
    pub snapshot_us: u64,
    pub views_us: u64,
    pub publish_us: u64,
    pub plugins_us: u64,
    pub coordination_us: u64,
    pub journal_build_us: u64,
    pub journal_queue_us: u64,
    pub generated_us: u64,
    pub owner_pre_us: u64,
    pub owner_post_us: u64,
}

/// A completed active core iteration, distinct from edge fanout.
#[derive(Debug, Clone, Serialize)]
pub struct LoadTickSample {
    pub tick: u64,
    pub wall_start_unix_ns: u64,
    pub wall_us: u64,
    pub total_us: u64,
    pub phase_us: LoadPhaseTimes,
    pub coreperf: Option<std::sync::Arc<mc_core::boundary::CorePerfTick>>,
    /// `{"units": n, "alive": {"<faction>": n}, "in_weapon_reach": n,
    /// "fighting": n, "acting": n, "strikes": n, "shields_raised": n, "shield_blocks": n,
    /// "weapon_swaps": n, "holding_spear": n, "holding_sword": n,
    /// "holding_bow": n}`; the key is absent without battle units.
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_battle"
    )]
    pub battle: Option<std::sync::Arc<mc_core::boundary::BattleCounts>>,
}

/// [`LoadTickSample::battle`] as an object whose `alive` maps each faction
/// (as a string key, the JSON form of integer keys) to its alive units.
fn serialize_battle<S: serde::Serializer>(
    battle: &Option<std::sync::Arc<mc_core::boundary::BattleCounts>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::{SerializeMap, SerializeStruct};
    struct Alive<'a>(&'a [(u16, u32)]);
    impl Serialize for Alive<'_> {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            let mut map = serializer.serialize_map(Some(self.0.len()))?;
            for (faction, alive) in self.0 {
                map.serialize_entry(faction, alive)?;
            }
            map.end()
        }
    }
    let Some(battle) = battle else {
        return serializer.serialize_none();
    };
    let mut out = serializer.serialize_struct("BattleCounts", 12)?;
    out.serialize_field("units", &battle.units)?;
    out.serialize_field("alive", &Alive(&battle.alive))?;
    out.serialize_field("in_weapon_reach", &battle.in_weapon_reach)?;
    out.serialize_field("fighting", &battle.fighting)?;
    out.serialize_field("acting", &battle.acting)?;
    out.serialize_field("strikes", &battle.strikes)?;
    out.serialize_field("shields_raised", &battle.shields_raised)?;
    out.serialize_field("shield_blocks", &battle.shield_blocks)?;
    out.serialize_field("weapon_swaps", &battle.weapon_swaps)?;
    out.serialize_field("holding_spear", &battle.holding_spear)?;
    out.serialize_field("holding_sword", &battle.holding_sword)?;
    out.serialize_field("holding_bow", &battle.holding_bow)?;
    out.end()
}

/// Real send-channel gauges. `capacity()`, unlike a receiver length, includes
/// outstanding reservations: queued and reserved slots cannot be separated.
#[derive(Debug, Default, Serialize)]
pub struct LoadQueueGauge {
    pub capacity_slots: usize,
    pub queued_or_reserved_slots: usize,
    pub max_viewer_queued_or_reserved_slots: usize,
    pub queued_slots: Option<usize>,
    pub reserved_slots: Option<usize>,
    pub queued_bytes: Option<u64>,
    pub reserved_bytes: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct LoadQueueCapabilities {
    pub queued_or_reserved_slots: bool,
    pub queued_slots: bool,
    pub reserved_slots: bool,
    pub queued_bytes: bool,
    pub reserved_bytes: bool,
}

/// Aggregation over currently registered Play connections. Channel writers
/// may drain between observations; this is not a globally atomic snapshot.
#[derive(Debug, Serialize)]
pub struct LoadQueues {
    pub captured_unix_ns: u64,
    pub completed_unix_ns: u64,
    pub viewers: usize,
    pub capabilities: LoadQueueCapabilities,
    pub reliable: LoadQueueGauge,
    pub world: LoadQueueGauge,
}

/// Versioned streaming telemetry; a null queue object means unavailable.
#[derive(Debug, Serialize)]
pub struct LoadMetricsSnapshot {
    pub schema_version: u32,
    pub captured_unix_ns: u64,
    pub instance_started_unix_ns: u64,
    pub capacity: usize,
    pub after_tick: Option<u64>,
    pub oldest_tick: Option<u64>,
    pub latest_tick: Option<u64>,
    pub overrun: bool,
    /// True only when a requested push-driven wait reached its deadline.
    pub wait_timed_out: bool,
    pub overwritten_samples_total: u64,
    pub source_skipped_ticks_total: u64,
    pub samples: Vec<LoadTickSample>,
    pub queues: Option<LoadQueues>,
}

struct LoadMetricsInner {
    samples: VecDeque<LoadTickSample>,
    /// `(tick, sample)`, ascending tick.
    host_samples: VecDeque<(u64, HostSample)>,
    /// `(tick, stats)` of fanned-out ticks that had a recorder, ascending.
    recorder_ticks: VecDeque<(u64, RecorderTick)>,
    /// Newest tick whose fanout finished; `/telemetry.jsonl` serves no
    /// later tick, so a served line never misses its `recorders`.
    fanout_through: Option<u64>,
    last_evicted_tick: Option<u64>,
    overwritten_samples_total: u64,
    source_skipped_ticks_total: u64,
}

/// One `/telemetry.jsonl` line.
#[derive(Serialize)]
struct TelemetryLine<'a> {
    #[serde(flatten)]
    sample: &'a LoadTickSample,
    host: Option<&'a HostSample>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recorders: Option<&'a RecorderTick>,
}

/// Mutable dashboard state. Metrics have an independent lock and never
/// serialize on the core thread. All producer storage is allocated at setup.
pub struct DashboardState {
    inner: parking_lot::Mutex<StateInner>,
    started: Instant,
    instance_started_unix_ns: u64,
    load_metrics: Mutex<LoadMetricsInner>,
    load_changed: Notify,
    queue_source: OnceLock<Arc<ConnectionRegistry>>,
    plugin_source: OnceLock<Arc<mc_core::Core>>,
    /// Cumulative input-drop counter, read at request time like the queue
    /// gauges (the count lives in the core's submit path, not in the pump).
    input_source: OnceLock<InputDropSource>,
    /// Idle=0, queued=1, consumed=2; disabled=u8::MAX.
    coreperf_visibility_requested: AtomicU8,
    coreperf_visibility_changed: Notify,
    metrics_bearer_token: OnceLock<String>,
}

#[derive(Debug)]
struct StateInner {
    world: DashboardWorld,
    players: DashboardPlayers,
    simulation_tick: u64,
    /// Pump arrivals `(tick, at)` for TPS; ring buffer capped at 1200.
    arrivals: VecDeque<(u64, Instant)>,
    /// Core tick cost per tick, µs; ring buffer capped at 1200.
    tick_us: VecDeque<u64>,
    /// Pump publish time per tick, µs; ring buffer capped at 1200.
    publish_us: VecDeque<u64>,
    pump_skipped_ticks: u64,
    pump_stale_skipped_ticks: u64,
    chunks: DashboardChunks,
    entities: DashboardEntities,
    coreperf_visibility: Option<CorePerfVisibility>,
    coreperf_visibility_proof: Option<CorePerfVisibilityProof>,
    network: DashboardNetwork,
    warnings: Vec<String>,
}

struct Request {
    method: String,
    path: String,
    query: Option<String>,
    bearer_token: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct LoadMetricsQuery {
    after_tick: Option<u64>,
    wait: bool,
    min_samples: usize,
}
