//! The dashboard's live state: recording incoming samples and answering snapshots.

use super::*;

use super::latency::push_window;
use super::latency::summarize;
use super::latency::tps_of;
use super::latency::unix_ns;
use super::routing::joined;

impl DashboardState {
    /// Create state with fixed world facts; counters start at zero.
    pub fn new(world: DashboardWorld) -> Self {
        Self {
            inner: parking_lot::Mutex::new(StateInner {
                world,
                players: DashboardPlayers::default(),
                simulation_tick: 0,
                arrivals: VecDeque::new(),
                tick_us: VecDeque::new(),
                publish_us: VecDeque::new(),
                pump_skipped_ticks: 0,
                chunks: DashboardChunks::default(),
                entities: DashboardEntities::default(),
                coreperf_visibility: None,
                coreperf_visibility_proof: None,
                network: DashboardNetwork::default(),
                warnings: Vec::new(),
            }),
            started: Instant::now(),
            instance_started_unix_ns: unix_ns(),
            load_metrics: Mutex::new(LoadMetricsInner {
                samples: VecDeque::with_capacity(LOAD_SAMPLE_CAPACITY),
                host_samples: VecDeque::with_capacity(HOST_SAMPLE_CAPACITY),
                recorder_ticks: VecDeque::with_capacity(LOAD_SAMPLE_CAPACITY),
                fanout_through: None,
                last_evicted_tick: None,
                overwritten_samples_total: 0,
                source_skipped_ticks_total: 0,
            }),
            load_changed: Notify::new(),
            queue_source: OnceLock::new(),
            plugin_source: OnceLock::new(),
            input_source: OnceLock::new(),
            metrics_bearer_token: OnceLock::new(),
            coreperf_visibility_requested: AtomicU8::new(
                if std::env::var_os("KRASNOKAMEN_COREPERF_REGIONS").is_some() {
                    0
                } else {
                    u8::MAX
                },
            ),
            coreperf_visibility_changed: Notify::new(),
        }
    }

    /// Protect Prometheus scraping and opt-in coreperf capture with this token.
    /// Without a token, only scraping retains the local unauthenticated default.
    pub fn set_metrics_bearer_token(&self, token: Option<String>) {
        if let Some(token) = token.filter(|token| !token.is_empty()) {
            let _ = self.metrics_bearer_token.set(token);
        }
    }

    /// Attach the existing send-channel registry once; gauges are read only
    /// when an operator requests them, never aggregated on the tick path.
    pub fn set_queue_source(&self, connections: Arc<ConnectionRegistry>) {
        let _ = self.queue_source.set(connections);
    }

    /// Read real component counters on demand, without a tick-path producer.
    pub fn set_plugin_source(&self, core: Arc<mc_core::Core>) {
        let _ = self.plugin_source.set(core);
    }

    /// Wire the core's cumulative input-drop counter. Read on each dashboard
    /// payload, so a full input queue is visible without a pump change.
    pub fn set_input_source(&self, source: InputDropSource) {
        let _ = self.input_source.set(source);
    }

    pub(super) fn metrics_authorized(&self, supplied: Option<&str>) -> bool {
        let Some(expected) = self.metrics_bearer_token.get() else {
            return true;
        };
        supplied.is_some_and(|supplied| {
            let supplied = supplied.as_bytes();
            let expected = expected.as_bytes();
            supplied.len() == expected.len()
                && supplied
                    .iter()
                    .zip(expected)
                    .fold(0u8, |difference, (left, right)| difference | (left ^ right))
                    == 0
        })
    }

    fn plugin_snapshot(&self) -> Option<PluginStats> {
        self.plugin_source.get().map(|core| PluginStats {
            runtime_ready: true,
            loaded_packages: core.plugin_packages().len(),
            wasm_invocations: mc_plugin_host::wasm_invocations(),
            host_operations: core.plugin_host_operations(),
            command_results: core.plugin_command_results(),
        })
    }

    /// Record a timing-lane sample without allocation. Its independent
    /// subscriber does not wait for edge fanout and preserves source gaps.
    pub fn record_load_tick(&self, tick: u64, times: TickTimes) {
        let Ok(mut inner) = self.load_metrics.lock() else {
            return;
        };
        if inner.samples.len() == LOAD_SAMPLE_CAPACITY {
            inner.last_evicted_tick = inner.samples.pop_front().map(|sample| sample.tick);
            inner.overwritten_samples_total = inner.overwritten_samples_total.saturating_add(1);
        }
        inner.samples.push_back(LoadTickSample {
            tick,
            wall_start_unix_ns: times.wall_start_unix_ns,
            wall_us: times.wall_us,
            total_us: times.total_us,
            coreperf: times.coreperf,
            battle: times.battle,
            phase_us: LoadPhaseTimes {
                input_us: times.input_us,
                chunks_us: times.chunks_us,
                entity_us: times.entity_us,
                merge_us: times.merge_us,
                block_us: times.block_us,
                commit_us: times.commit_us,
                snapshot_us: times.snapshot_us,
                views_us: times.views_us,
                publish_us: times.publish_us,
                plugins_us: times.plugins_us,
                coordination_us: times.coordination_us,
                journal_build_us: times.journal_build_us,
                journal_queue_us: times.journal_queue_us,
                generated_us: times.generated_us,
                owner_pre_us: times.owner_pre_us,
                owner_post_us: times.owner_post_us,
            },
        });
        drop(inner);
        self.load_changed.notify_waiters();
    }

    /// Count timing broadcast loss independently of world pump lag.
    pub fn record_load_skipped(&self, skipped: u64) {
        if let Ok(mut inner) = self.load_metrics.lock() {
            inner.source_skipped_ticks_total =
                inner.source_skipped_ticks_total.saturating_add(skipped);
        }
        self.load_changed.notify_waiters();
    }

    /// Newest core tick on the timing lane, which never waits for the world
    /// pump; `None` before the first sample.
    pub(crate) fn latest_core_tick(&self) -> Option<u64> {
        self.load_metrics
            .lock()
            .ok()?
            .samples
            .back()
            .map(|sample| sample.tick)
    }

    /// One core tick from the timing lane: the host sample (when `host`
    /// takes one) is stored before the tick becomes visible, so a reader
    /// woken by the tick never serves it without its sample.
    pub(crate) fn record_core_tick(
        &self,
        host: &mut crate::host_telemetry::HostSampler,
        tick: u64,
        times: TickTimes,
    ) {
        if let Some(sample) = host.on_tick(tick) {
            self.record_host_sample(tick, sample);
        }
        self.record_load_tick(tick, times);
    }

    /// Record the host sample taken on `tick` (ascending ticks).
    pub(super) fn record_host_sample(&self, tick: u64, sample: HostSample) {
        let Ok(mut inner) = self.load_metrics.lock() else {
            return;
        };
        if inner.host_samples.len() == HOST_SAMPLE_CAPACITY {
            inner.host_samples.pop_front();
        }
        inner.host_samples.push_back((tick, sample));
    }

    /// The pump finished fanning out `tick`; `recorders` when the tick had
    /// a recorder connection (ascending ticks).
    pub(crate) fn record_fanout_tick(&self, tick: u64, recorders: Option<RecorderTick>) {
        let Ok(mut inner) = self.load_metrics.lock() else {
            return;
        };
        if let Some(recorders) = recorders {
            if inner.recorder_ticks.len() == LOAD_SAMPLE_CAPACITY {
                inner.recorder_ticks.pop_front();
            }
            inner.recorder_ticks.push_back((tick, recorders));
        }
        inner.fanout_through = Some(tick);
        drop(inner);
        self.load_changed.notify_waiters();
    }

    /// Retained ticks after `after_tick` whose fanout finished, as JSON
    /// lines, each joined with the host sample and recorder stats of that
    /// tick. Copies while locked and serializes after, like
    /// [`Self::load_snapshot`]. `None` when the ring is poisoned.
    pub fn telemetry_jsonl(&self, after_tick: Option<u64>) -> Option<Vec<u8>> {
        type Line = (LoadTickSample, Option<HostSample>, Option<RecorderTick>);
        let lines: Vec<Line> = {
            let inner = self.load_metrics.lock().ok()?;
            let through = inner.fanout_through;
            let mut hosts = inner.host_samples.iter().peekable();
            let mut recorders = inner.recorder_ticks.iter().peekable();
            inner
                .samples
                .iter()
                .filter(|sample| after_tick.is_none_or(|after| sample.tick > after))
                .take_while(|sample| through.is_some_and(|through| sample.tick <= through))
                .map(|sample| {
                    (
                        sample.clone(),
                        joined(&mut hosts, sample.tick),
                        joined(&mut recorders, sample.tick),
                    )
                })
                .collect()
        };
        let mut body = Vec::new();
        for (sample, host, recorders) in &lines {
            serde_json::to_writer(
                &mut body,
                &TelemetryLine {
                    sample,
                    host: host.as_ref(),
                    recorders: recorders.as_ref(),
                },
            )
            .ok()?;
            body.push(b'\n');
        }
        Some(body)
    }

    /// Copy only requested raw samples while locked, then read live queues.
    /// A poisoned metrics lock is unavailable, never a fabricated empty ring.
    pub fn load_snapshot(&self, after_tick: Option<u64>) -> Option<LoadMetricsSnapshot> {
        let captured_unix_ns = unix_ns();
        let inner = self.load_metrics.lock().ok()?;
        let snapshot = LoadMetricsSnapshot {
            schema_version: 1,
            captured_unix_ns,
            instance_started_unix_ns: self.instance_started_unix_ns,
            capacity: LOAD_SAMPLE_CAPACITY,
            after_tick,
            oldest_tick: inner.samples.front().map(|sample| sample.tick),
            latest_tick: inner.samples.back().map(|sample| sample.tick),
            overrun: after_tick.is_some_and(|after| {
                inner
                    .last_evicted_tick
                    .is_some_and(|evicted| after < evicted)
            }),
            wait_timed_out: false,
            overwritten_samples_total: inner.overwritten_samples_total,
            source_skipped_ticks_total: inner.source_skipped_ticks_total,
            samples: inner
                .samples
                .iter()
                .filter(|sample| after_tick.is_none_or(|after| sample.tick > after))
                .cloned()
                .collect(),
            queues: None,
        };
        drop(inner);
        Some(LoadMetricsSnapshot {
            queues: self
                .queue_source
                .get()
                .and_then(|source| source.load_queues()),
            ..snapshot
        })
    }

    /// Register before checking the ring to avoid lost wakeups. Neither this
    /// wait nor the producer allocates per tick; only readers create futures.
    /// `fanned_out` counts only ticks whose fanout finished
    /// (`/telemetry.jsonl`).
    pub(super) async fn wait_for_load_samples(
        &self,
        after_tick: Option<u64>,
        min_samples: usize,
        fanned_out: bool,
    ) -> Option<()> {
        let notified = self.load_changed.notified();
        tokio::pin!(notified);
        loop {
            notified.as_mut().enable();
            {
                let inner = self.load_metrics.lock().ok()?;
                let overrun = after_tick.is_some_and(|after| {
                    inner
                        .last_evicted_tick
                        .is_some_and(|evicted| after < evicted)
                });
                let through = if fanned_out {
                    inner.fanout_through
                } else {
                    Some(u64::MAX)
                };
                let available = inner
                    .samples
                    .iter()
                    .rev()
                    .skip_while(|sample| through.is_none_or(|through| sample.tick > through))
                    .take_while(|sample| after_tick.is_none_or(|after| sample.tick > after))
                    .take(min_samples)
                    .count();
                if overrun || available >= min_samples {
                    return Some(());
                }
            }
            notified.as_mut().await;
            notified.set(self.load_changed.notified());
        }
    }

    /// Commit one published tick and its counters in one critical section.
    pub(crate) fn record_pump_tick(&self, sample: PumpTick) {
        let now = Instant::now();
        let mut inner = self.inner.lock();
        inner.simulation_tick = inner.simulation_tick.max(sample.tick);
        if inner.arrivals.len() >= 1200 {
            inner.arrivals.pop_front();
        }
        inner.arrivals.push_back((sample.tick, now));
        push_window(&mut inner.tick_us, sample.core_us);
        push_window(&mut inner.publish_us, sample.publish_us);
        inner.network.bytes_written = sample.bytes_written;
        inner.network.reliable_drops = sample.reliable_drops;
        inner.chunks = sample.chunks;
        inner.entities = sample.entities;
        if let Some(players) = sample.players {
            inner.players = players;
        }
    }

    pub(crate) fn record_coreperf_visibility(&self, tick: u64, pairs: usize) {
        self.inner.lock().coreperf_visibility = Some(CorePerfVisibility {
            source_tick: tick,
            tracked_entity_viewer_pairs: pairs,
        });
    }

    pub(crate) fn take_coreperf_visibility_request(&self) -> bool {
        self.coreperf_visibility_requested
            .compare_exchange(1, 2, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    pub(crate) fn record_coreperf_visibility_proof(&self, proof: CorePerfVisibilityProof) {
        self.inner.lock().coreperf_visibility_proof = Some(proof);
        self.coreperf_visibility_changed.notify_waiters();
    }

    pub(super) async fn wait_for_coreperf_visibility_proof(&self) -> CorePerfVisibilityProof {
        let notified = self.coreperf_visibility_changed.notified();
        tokio::pin!(notified);
        loop {
            notified.as_mut().enable();
            if let Some(proof) = self.inner.lock().coreperf_visibility_proof.clone() {
                return proof;
            }
            notified.as_mut().await;
            notified.set(self.coreperf_visibility_changed.notified());
        }
    }

    /// Count core ticks the pump skipped after a `broadcast` lag.
    pub fn record_pump_skipped(&self, ticks: u64) {
        let mut inner = self.inner.lock();
        inner.pump_skipped_ticks = inner.pump_skipped_ticks.saturating_add(ticks);
    }

    /// Replace the operator warning list.
    pub fn set_warnings(&self, warnings: Vec<String>) {
        let mut inner = self.inner.lock();
        inner.warnings = warnings;
    }

    /// Render the current `/stats` snapshot.
    pub fn snapshot(&self) -> StatsSnapshot {
        let uptime_secs = self.started.elapsed().as_secs();
        // Request-time counter, read before the state lock: the source is one
        // atomic load in its owner and must never run under this mutex.
        let input_drops = self.input_source.get().map_or(0, |source| source());
        let inner = self.inner.lock();
        let tick = summarize(&inner.tick_us, inner.simulation_tick);
        let publish = summarize(&inner.publish_us, inner.simulation_tick);
        let tps = tps_of(&inner.arrivals);
        StatsSnapshot {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            uptime_secs,
            simulation_tick: inner.simulation_tick,
            world: inner.world.clone(),
            players: inner.players.clone(),
            tps,
            pump_skipped_ticks: inner.pump_skipped_ticks,
            tick,
            publish,
            chunks: inner.chunks.clone(),
            entities: inner.entities.clone(),
            coreperf_visibility: inner.coreperf_visibility.clone(),
            coreperf_visibility_proof: inner.coreperf_visibility_proof.clone(),
            network: DashboardNetwork {
                input_drops,
                ..inner.network.clone()
            },
            journal: {
                let stats = mc_core::journal::journal_prune_stats();
                DashboardJournal {
                    prune_failures_total: stats.failures_total,
                    prune_retained_bytes: stats.retained_bytes,
                }
            },
            plugin: self.plugin_snapshot(),
            autoscale: false,
            warnings: inner.warnings.clone(),
        }
    }
}
