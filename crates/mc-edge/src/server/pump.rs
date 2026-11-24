//! Tick output pump: outbox routing, publication and join waiting.
//! Moved verbatim from `server.rs`.

use super::*;
use mc_core::boundary::EntityId;
use std::collections::BTreeMap;

const COREPERF_COLD_PAIR_LIMIT: usize = 1_000_000;

/// A test's pump gate callback.
#[cfg(test)]
pub(crate) type PumpGate = std::sync::Arc<dyn Fn() + Send + Sync>;

/// Test-only: park this server's pump in `gate` until it returns, after the
/// pump recorded a tick and before it takes the registry lock. The gate lives
/// in the server, so parallel tests never see each other's gates.
#[cfg(test)]
pub(crate) fn arm_pump_gate(server: &EdgeServer, gate: PumpGate) {
    *server.pump_gate.lock().expect("pump gate") = Some(gate);
}

#[cfg(test)]
fn pump_gate(server: &EdgeServer) {
    let armed = server.pump_gate.lock().expect("pump gate").clone();
    if let Some(gate) = armed {
        gate();
    }
}

/// Vanilla 26.3 `PlayerList.tick()` (javap) increments `sendAllPlayerInfoIn`
/// and, once it exceeds 600, broadcasts one `UPDATE_LATENCY` player-info packet
/// listing every player and resets it to 0: one packet per 601 ticks.
pub(super) const LATENCY_BROADCAST_INTERVAL_TICKS: u64 = 601;

fn bounded_tracked_union(
    pairs: usize,
    ids: impl Iterator<Item = EntityId>,
    limit: usize,
) -> Option<usize> {
    if pairs > limit {
        return None;
    }
    Some(ids.collect::<HashSet<_>>().len())
}

/// Queue budget of a pump behind the tick: an output that waited longer than
/// this since its publication is skipped like a broadcast `Lagged` once a
/// newer output is queued. The reliable outbox drains only through the
/// published tick, so without it a pump slower than the tick sat at the
/// oldest retained output, 31 outputs behind, and held every reliable change
/// (a `Joined` too) for 31 tick periods: past the 10 s join deadline below
/// about 3 TPS. 1.6 s is the 32-output window at 20 TPS. It bounds only the
/// queue wait: a single long publish, a long tick or a slow socket still
/// delays delivery, and a socket overflow still closes the connection.
pub(super) const PUBLISH_QUEUE_BUDGET: std::time::Duration = std::time::Duration::from_millis(1600);

/// An output to skip: it waited past [`PUBLISH_QUEUE_BUDGET`] since the tick
/// owner published it and a newer one is queued. The newest output and
/// outputs without a publish instant are never stale. `newer_queued` runs
/// only for an old output: a timely pump takes no broadcast tail lock.
pub(super) fn stale(
    out: &TickOutput,
    now: std::time::Instant,
    newer_queued: impl FnOnce() -> bool,
) -> bool {
    out.times
        .published_at
        .is_some_and(|published| now.saturating_duration_since(published) > PUBLISH_QUEUE_BUDGET)
        && newer_queued()
}

/// Skipped publications reconcile from the next processed snapshot against
/// the last published one; its cumulative knockback epochs close only
/// recipients of a skipped impulse.
fn skip_publications(
    server: &EdgeServer,
    viewers: &mut HashMap<ConnectionId, crate::plan::ViewerState>,
    chunk_cache: &crate::plan::ChunkPacketCache,
    skipped: u64,
) {
    server.dashboard.record_pump_skipped(skipped);
    for viewer in viewers.values_mut() {
        viewer.skip_publication(chunk_cache);
    }
}

/// The next output to publish, or `None` once the core closed the broadcast.
/// Skips what a broadcast `Lagged` dropped and every [`stale`] output.
pub(super) async fn next_publication(
    server: &EdgeServer,
    rx: &mut broadcast::Receiver<TickOutput>,
    viewers: &mut HashMap<ConnectionId, crate::plan::ViewerState>,
    chunk_cache: &crate::plan::ChunkPacketCache,
) -> Option<TickOutput> {
    loop {
        match rx.recv().await {
            Ok(out) if stale(&out, std::time::Instant::now(), || !rx.is_empty()) => {
                server.dashboard.record_pump_stale_skipped(1);
                skip_publications(server, viewers, chunk_cache, 1);
            }
            Ok(out) => return Some(out),
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                skip_publications(server, viewers, chunk_cache, skipped);
            }
            Err(broadcast::error::RecvError::Closed) => return None,
        }
    }
}

/// Test/dev pump lag: `KRASNOKAMEN_PUMP_LAG_TICKS=K` makes the pump wait for K core tick events
/// before each publication, so it always meets a backlog and its own skip/drain decides what to
/// publish — the production `next_publication` stays untouched (a control tree that reverts the fix
/// is exercised exactly as the fix is). The wait counts the core's light tick-time stream, created
/// only when the knob is set; no timer is read. At `/tick rate 3` K=32 is about 10 s of age, past
/// the 1.6 s queue budget and the 10 s join deadline. Unset or `0` keeps the shipped pump; an
/// invalid value fails startup. See docs/development/real-client-gates.md.
pub(super) const LAG_ENV: &str = "KRASNOKAMEN_PUMP_LAG_TICKS";

/// The count named by `KRASNOKAMEN_PUMP_LAG_TICKS`; `None` means no extra wait.
pub(super) fn lag_ticks_from_env() -> anyhow::Result<Option<u64>> {
    lag_ticks(std::env::var(LAG_ENV).ok().as_deref()).map_err(anyhow::Error::msg)
}

/// The env value as a tick count: pure, so the parsing rules are testable and never touch the process.
pub(super) fn lag_ticks(value: Option<&str>) -> Result<Option<u64>, String> {
    let Some(raw) = value else { return Ok(None) };
    let ticks: u64 = raw.trim().parse().map_err(|_| {
        format!("{LAG_ENV} must be a whole number of ticks the pump waits before publishing (unset or 0 = none), got {raw:?}")
    })?;
    Ok(Some(ticks).filter(|ticks| *ticks > 0))
}

/// Count one core tick event toward the wait: `Ok(())` is one tick, `Err(dropped)` a lagged run of
/// that many. Pure, so a test feeds events instead of running a core.
pub(super) fn pace_count(remaining: u64, event: Result<(), u64>) -> u64 {
    match event {
        Ok(()) => remaining.saturating_sub(1),
        Err(dropped) => remaining.saturating_sub(dropped),
    }
}

/// Single pump task: subscribes to the core, records tick stats, drains the
/// reliable outbox FIRST, then publishes the world snapshot and routes both.
/// `Lagged` and [`stale`] outputs are skipped ([`next_publication`]);
/// cumulative knockback epochs in the next output close only recipients
/// whose impulse was skipped. The reliable outbox is
/// drained independently. Fanout runs on its own
/// rayon pool: on the global pool the core tick's parallel phases queued
/// behind whole publishes.
pub(super) fn spawn_pump(server: Arc<EdgeServer>) -> anyhow::Result<tokio::task::JoinHandle<()>> {
    let lag_ticks = lag_ticks_from_env()?;
    if let Some(ticks) = lag_ticks {
        tracing::warn!(
            env = LAG_ENV,
            lag_ticks = ticks,
            "pump lag enabled: a test/dev knob for real-client gates, never for production"
        );
    }
    // Only with the knob set: the light tick-time stream counts the core's ticks for the wait.
    let mut pace_rx = lag_ticks.map(|_| server.hub.subscribe_tick_times());
    let mut rx = server.hub.subscribe();
    let pool = rayon::ThreadPoolBuilder::new()
        .thread_name(|i| format!("edge-fanout-{i}"))
        .build()?;
    // Hide enclosed ores in encoded chunks per `[anti_xray]` (built from the
    // core's exact light/opacity oracle, shared by every viewer via the cache).
    let anti_xray = crate::anti_xray::AntiXray::new(
        &server.wire.blocks,
        &server.hub.registry().light,
        &server.cfg.anti_xray,
    )?
    .map(Arc::new);
    Ok(tokio::spawn(async move {
        // Viewer world-view state keyed by connection: what each client
        // holds, reconciled against every newest snapshot.
        let mut viewers: HashMap<ConnectionId, crate::plan::ViewerState> = HashMap::new();
        // Encoded chunk packets shared across viewers: light compute costs
        // ~28 ms/chunk, so a full 81-chunk re-plan (2.3 s) always outruns
        // the 32-tick broadcast window (1.6 s) and the pump would lag,
        // resync, and recompute forever without ever routing. The cache
        // makes re-plans O(1) per chunk; block changes evict their chunk.
        let mut chunk_cache = crate::plan::ChunkPacketCache::with_anti_xray(anti_xray);
        // Latest routed global roster; seeds fresh viewers' player profiles
        // so a first routed Roster never mass-removes tracked entities.
        let mut latest_roster: Arc<[mc_core::dimensions::RosterPlayer]> = Arc::from(Vec::new());
        let coreperf = std::env::var_os("KRASNOKAMEN_COREPERF_REGIONS").is_some();
        let mut knockback_ticks = BTreeMap::new();
        // Routing view kept across ticks: refreshed only when a connection
        // registered or left (`ConnectionRegistry::routing_snapshot`), so an
        // unchanged tick clones no handles at all.
        let mut connections: HashMap<ConnectionId, ConnectionHandles> = HashMap::new();
        let mut connections_generation = 0u64;
        loop {
            if let (Some(ticks), Some(tick_rx)) = (lag_ticks, pace_rx.as_mut()) {
                let mut remaining = ticks;
                while remaining > 0 {
                    let event = match tick_rx.recv().await {
                        Ok(_) => Ok(()),
                        Err(broadcast::error::RecvError::Lagged(dropped)) => Err(dropped),
                        Err(broadcast::error::RecvError::Closed) => break,
                    };
                    remaining = pace_count(remaining, event);
                }
                // Knob-only debug event: the tick stream's backlog at the wait's completion, which
                // is the pump's distance behind the core when the harness holds a join.
                tracing::debug!(
                    lag_ticks = tick_rx.len(),
                    knob_ticks = ticks,
                    "pump knob wait completed"
                );
            }
            let Some(out) = next_publication(&server, &mut rx, &mut viewers, &chunk_cache).await
            else {
                // Drain terminal reliable disconnects before closing, both
                // for graceful shutdown and for an authoritative failure.
                refresh_routing(&server, &mut connections, &mut connections_generation);
                route_outbox(
                    &server,
                    &mut viewers,
                    &mut connections,
                    None,
                    &mut latest_roster,
                );
                if server.hub.shutdown_requested() {
                    tracing::info!(
                        connections = connections.len(),
                        "core tick broadcast closed after shutdown request"
                    );
                } else {
                    tracing::error!(
                        connections = connections.len(),
                        "core tick broadcast closed unexpectedly; core halted"
                    );
                }
                break;
            };
            server.tick_stats.record(out.tick);
            // Test-only: a test parks the real pump here, in a synchronous
            // section before the registry lock, the shape a fanout `install`
            // takes.
            #[cfg(test)]
            pump_gate(&server);
            let roster = refresh_routing(&server, &mut connections, &mut connections_generation);
            route_outbox(
                &server,
                &mut viewers,
                &mut connections,
                Some(out.tick),
                &mut latest_roster,
            );
            server.console_state.drained(out.tick);
            close_skipped_knockback(
                &server,
                &out,
                &mut knockback_ticks,
                &mut viewers,
                &mut connections,
            );
            let publish_start = std::time::Instant::now();
            publish_and_route(
                &server,
                &pool,
                &mut viewers,
                &out,
                &mut chunk_cache,
                &mut connections,
            )
            .await;
            broadcast_latencies(
                &server.connections,
                &server.net_counters,
                &mut connections,
                out.tick,
            );
            let publish_us = publish_start
                .elapsed()
                .as_micros()
                .min(u128::from(u64::MAX)) as u64;
            server
                .dashboard
                .record_pump_tick(crate::dashboard::PumpTick {
                    tick: out.tick,
                    core_us: out.times.total_us,
                    publish_us,
                    bytes_written: server.net_counters.bytes_written.load(Ordering::Relaxed),
                    reliable_drops: server.net_counters.reliable_drops.load(Ordering::Relaxed),
                    chunks: DashboardChunks {
                        loaded_total: out.snapshot.chunks.len(),
                    },
                    entities: DashboardEntities {
                        total: out.snapshot.entities.len(),
                    },
                    players: roster.map(|names| crate::dashboard::DashboardPlayers {
                        count: names.len(),
                        max: server.cfg.server.max_players,
                        names,
                    }),
                });
            if coreperf {
                let pairs = viewers
                    .values()
                    .map(|viewer| {
                        viewer.tracked_entities.len().saturating_sub(usize::from(
                            viewer.tracked_entities.contains_key(&viewer.entity_id),
                        ))
                    })
                    .sum();
                if server.dashboard.take_coreperf_visibility_request() {
                    let ids = viewers.values().flat_map(|viewer| {
                        viewer
                            .tracked_entities
                            .keys()
                            .copied()
                            .filter(move |id| *id != viewer.entity_id)
                    });
                    let unique = bounded_tracked_union(pairs, ids, COREPERF_COLD_PAIR_LIMIT);
                    server.dashboard.record_coreperf_visibility_proof(
                        crate::dashboard::CorePerfVisibilityProof {
                            source_tick: out.tick,
                            viewers: viewers.len(),
                            tracked_entity_viewer_pairs: pairs,
                            unique_tracked_entities: unique,
                            pair_limit: COREPERF_COLD_PAIR_LIMIT,
                        },
                    );
                }
                server.dashboard.record_coreperf_visibility(out.tick, pairs);
            }
            tracing::debug!(
                tick = out.tick,
                viewers = viewers.len(),
                elapsed_us = publish_us,
                "pump published tick"
            );
            // Yield after every publish. Routing wakes connection tasks into
            // this worker's unstealable LIFO slot, and `rx.recv()` returns
            // Ready while outputs are queued, so a catching-up pump never
            // reached an await point: a Java client's writer sat queued
            // behind it with 512 routed chunks for 5+ s (stuck loading).
            tokio::task::yield_now().await;
        }
    }))
}

/// Drain `Core::try_drain_outbox` and route reliable changes FIRST (before
/// the world publish). Each `PlayerChange` is mapped to framed reliable
/// packets via `plan::plan_player_changes` (single-sourced with the world
/// plan; includes `GameModeChanged`/cursor). A full reliable queue
/// disconnects; changes for unknown/gone connections are dropped with the
/// connection.
pub(super) fn configured_viewer(
    server: &EdgeServer,
    connection: ConnectionId,
    entity: mc_core::boundary::EntityId,
    position: [f64; 3],
    dimension: mc_core::dimensions::Dimension,
) -> crate::plan::ViewerState {
    let mut viewer = crate::plan::ViewerState::new(connection, entity, position);
    viewer.dimension = dimension;
    {
        let registry = server.connections.inner.lock();
        viewer.view_distance = registry.view_distances.get(&connection).copied();
        viewer.recorder = registry.recorders.contains(&connection);
    }
    viewer.dimension_type_ids = server.wire.dimension_type_ids;
    if let Some(packets) = crate::recipes::packets_for(&server.hub.current_registry()) {
        viewer.advancements = Arc::clone(&packets.advancements);
    }
    viewer
}

/// Refresh the pump's routing view when a connection registered or left.
///
/// The view outlives a tick, so the pump no longer rebuilds every handle
/// triplet per tick: a registration, a disconnect and a refusing write all
/// move the registry's generation, and the next refresh replaces the view
/// wholesale — the same content the per-tick clone used to produce.
fn refresh_routing(
    server: &Arc<EdgeServer>,
    connections: &mut HashMap<ConnectionId, ConnectionHandles>,
    generation: &mut u64,
) -> Option<Vec<String>> {
    let (handles, roster) = server.connections.routing_snapshot(generation);
    if let Some(handles) = handles {
        connections.clear();
        connections.extend(handles);
    }
    roster
}

pub(super) fn route_outbox(
    server: &Arc<EdgeServer>,
    viewers: &mut HashMap<ConnectionId, crate::plan::ViewerState>,
    connections: &mut HashMap<ConnectionId, ConnectionHandles>,
    through_tick: Option<u64>,
    latest_roster: &mut Arc<[mc_core::dimensions::RosterPlayer]>,
) {
    route_outbox_batches(
        server,
        viewers,
        connections,
        through_tick.map_or_else(
            || server.hub.try_drain_outbox(),
            |tick| server.hub.try_drain_outbox_through(tick),
        ),
        latest_roster,
    );
}

/// Seed a freshly created viewer's player profiles from the latest routed
/// roster. Pre-registration rosters are stashed and replayed through a
/// throwaway viewer, so without this seeding a new viewer's
/// `player_profiles` stays empty while publish already tracks entities; the
/// next routed Roster then treats every tracked player as uuid-replaced and
/// fans out one reliable `RemoveEntities` per player into the bounded queue
/// (at 1000 players that alone overflows the 512-slot bound in one tick).
/// Vanilla sends one roster packet per join and never per-player removals on
/// roster changes.
fn seed_player_profiles(
    viewer: &mut crate::plan::ViewerState,
    roster: &[mc_core::dimensions::RosterPlayer],
) {
    for player in roster {
        viewer.player_profiles.insert(player.entity, player.uuid);
    }
}

pub(super) fn route_outbox_batches(
    server: &Arc<EdgeServer>,
    viewers: &mut HashMap<ConnectionId, crate::plan::ViewerState>,
    connections: &mut HashMap<ConnectionId, ConnectionHandles>,
    batches: Vec<(ConnectionId, Vec<mc_core::boundary::PlayerChange>)>,
    latest_roster: &mut Arc<[mc_core::dimensions::RosterPlayer]>,
) {
    // Ensure every live connection has a viewer entry before planning.
    // The entity id comes from the stashed `Joined` when available (the
    // core allocates it; ids diverge from connection ids in ordinary
    // operation). Otherwise fall back to spawn position [0.5,64,0.5] with
    // the entity still unknown — `plan_player_changes` adopts it from the
    // first `Joined` it plans.
    for &id in connections.keys() {
        if let std::collections::hash_map::Entry::Vacant(slot) = viewers.entry(id) {
            let (entity, position, dimension) = server
                .connections
                .take_join(id)
                .map(|joined| (joined.entity, joined.position, joined.dimension))
                .unwrap_or((
                    mc_core::boundary::EntityId(id.0 as i32),
                    [0.5, 64.0, 0.5],
                    Default::default(),
                ));
            let viewer = slot.insert(configured_viewer(server, id, entity, position, dimension));
            seed_player_profiles(viewer, latest_roster);
        }
    }
    for (connection, changes) in batches {
        for change in changes.iter() {
            if let mc_core::boundary::PlayerChange::Roster { players, .. } = change {
                *latest_roster = Arc::clone(players);
            }
        }
        if connection == mc_core::command_console::CONNECTION {
            crate::console::output(&server.console_state, &changes);
            continue;
        }
        if tracing::enabled!(tracing::Level::DEBUG)
            && changes
                .iter()
                .any(|change| matches!(change, mc_core::boundary::PlayerChange::Joined { .. }))
        {
            tracing::debug!(
                connection = connection.0,
                pump_tick = server.tick_stats.tick(),
                "join drained"
            );
        }
        // Core-ordered kick: stash the reason so the play loop can log
        // peer+reason after flushing the kick packet. The packet itself
        // is planned below (reliable, last) and routed in order.
        for change in changes.iter() {
            if let mc_core::boundary::PlayerChange::Disconnect { reason, .. } = change {
                server
                    .connections
                    .stash_disconnect_reason(connection, reason.clone());
                // A Disconnect without a Joined in the same ordered list is a refused Join
                // (e.g. duplicate login): wake the connection's join wait so it can send
                // the reason instead of waiting out the join deadline.
                if !changes
                    .iter()
                    .any(|change| matches!(change, mc_core::boundary::PlayerChange::Joined { .. }))
                {
                    server.connections.refuse_join(connection);
                }
                break;
            }
        }
        // Join race: the pump may drain before the connection registered
        // its queues (join-wait runs pre-registration). Stash the Joined
        // for `wait_for_join` AND the whole ordered reliable list for
        // replay at registration — dropping the tail would lose the
        // authoritative MenuContents/Health/Experience burst.
        // A registration can race the tick snapshot: a newcomer's handles are
        // looked up live, and if it is still unregistered its whole ordered
        // list (Joined first, for `wait_join`) is stashed for replay at
        // registration, atomically with that lookup.
        let mut changes = changes;
        if let std::collections::hash_map::Entry::Vacant(slot) = connections.entry(connection) {
            let Some(handles) = server
                .connections
                .handles_or_stash(connection, &mut changes)
            else {
                continue;
            };
            slot.insert(handles);
            let (entity, position, dimension) = server
                .connections
                .take_join(connection)
                .map(|joined| (joined.entity, joined.position, joined.dimension))
                .unwrap_or((
                    mc_core::boundary::EntityId(connection.0 as i32),
                    [0.5, 64.0, 0.5],
                    Default::default(),
                ));
            if let std::collections::hash_map::Entry::Vacant(slot) = viewers.entry(connection) {
                let viewer = slot.insert(configured_viewer(
                    server, connection, entity, position, dimension,
                ));
                seed_player_profiles(viewer, latest_roster);
            }
        }
        let Some(handles) = connections.get(&connection) else {
            continue;
        };
        let Some(viewer) = viewers.get_mut(&connection) else {
            continue;
        };
        // The authenticated roster is also the edge's unlock notification.
        // Restore the private operator command tree once, not on every tick.
        if !viewer.player_profiles.contains_key(&viewer.entity_id)
            && let Some(uuid) = changes.iter().find_map(|change| match change {
                mc_core::boundary::PlayerChange::Roster { players, .. } => players
                    .iter()
                    .find(|player| player.connection == connection)
                    .map(|player| player.uuid),
                _ => None,
            })
            && let Err(refusal) = server.send(
                connection,
                &crate::command_tree::tree(
                    server.admin.level(uuid),
                    server.hub.plugin_commands(),
                ),
            )
        {
            // `send` counts a full queue as this session's end. A queue whose
            // receiver is already gone belongs to a session that is ending on
            // its own: evict it from the routing view without a second cause.
            if !matches!(refusal, SendFailure::Closed) {
                server
                    .connections
                    .close(connection, "command-tree-send-failed");
            }
            connections.remove(&connection);
            viewers.remove(&connection);
            continue;
        }
        if !route_changes(&changes, viewer, handles, server) {
            connections.remove(&connection);
            viewers.remove(&connection);
        }
    }
}

pub(super) fn route_changes(
    changes: &[mc_core::boundary::PlayerChange],
    viewer: &mut crate::plan::ViewerState,
    handles: &ConnectionHandles,
    server: &Arc<EdgeServer>,
) -> bool {
    if changes.iter().any(|change| {
        matches!(
            change,
            mc_core::boundary::PlayerChange::DimensionChanged { .. }
        )
    }) {
        handles.world_tx.reset();
    }
    for planned in crate::plan::plan_routed_player_changes(changes, viewer) {
        match handles.reliable_tx.try_send(planned.data) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                server
                    .connections
                    .close(viewer.connection, "route-changes-reliable-full");
                server
                    .net_counters
                    .reliable_drops
                    .fetch_add(1, Ordering::Relaxed);
                return false;
            }
            // A receiver the writer already released is a session that is
            // ending on its own: evict it, count nothing.
            Err(mpsc::error::TrySendError::Closed(_)) => return false,
        }
    }
    true
}

/// Counts are cumulative across published ticks, not just the latest impulse
/// tick: a retained impulse must not conceal an earlier, skipped one.
pub(super) fn close_skipped_knockback(
    server: &Arc<EdgeServer>,
    out: &TickOutput,
    seen: &mut BTreeMap<ConnectionId, u64>,
    viewers: &mut HashMap<ConnectionId, crate::plan::ViewerState>,
    connections: &mut HashMap<ConnectionId, ConnectionHandles>,
) {
    seen.retain(|id, _| connections.contains_key(id));
    for (&id, handles) in connections.iter() {
        seen.entry(id).or_insert(handles.knockback_baseline);
    }
    for (&id, &(count, last_tick)) in out.snapshot.player_knockback_ticks.iter() {
        if !connections.contains_key(&id) {
            continue;
        }
        let previous = seen[&id];
        seen.insert(id, previous.max(count));
        let current = u64::from(last_tick == out.tick);
        let reason = if count.saturating_sub(previous) > current {
            Some("skipped-explosion-knockback")
        } else {
            None
        };
        if let Some(reason) = reason {
            server.connections.close(id, reason);
            connections.remove(&id);
            viewers.remove(&id);
        }
    }
}

/// Publish one tick to all viewers and route the addressed packets.
/// Reliable-overflow victims disconnect.
pub(super) async fn publish_and_route(
    server: &Arc<EdgeServer>,
    pool: &rayon::ThreadPool,
    viewers: &mut HashMap<ConnectionId, crate::plan::ViewerState>,
    out: &TickOutput,
    chunk_cache: &mut crate::plan::ChunkPacketCache,
    connections: &mut HashMap<ConnectionId, ConnectionHandles>,
) {
    viewers.retain(|id, _| connections.contains_key(id));
    // Planning spends this credit (slots and bytes), but it is not exclusive:
    // an LOD tile (`lod.rs`) also takes world-queue slots, at any time from
    // this read until the send, which is the whole planning time of every
    // viewer, so a planned frame can still find the queue full below.
    {
        let registry = server.connections.inner.lock();
        for (id, viewer) in viewers.iter_mut() {
            viewer.view_distance = registry.view_distances.get(id).copied();
            if let Some(handles) = connections.get(id) {
                viewer.chunk_batches = Arc::clone(&handles.chunk_batches);
            }
            (viewer.world_credit, viewer.world_bytes) =
                connections.get(id).map_or((0, 0), |handles| {
                    (handles.world_tx.capacity(), handles.world_tx.free_bytes())
                });
        }
    }

    let view_distance = i32::from(server.cfg.server.view_distance);
    // Real sidecar registries: the `publish` single-biome fallback would
    // fail every non-plains chunk encode and send no terrain at all.
    let registries = crate::plan::PlanRegistries {
        biomes: &server.wire.biomes,
        blocks: &server.wire.blocks,
        items: &server.wire.items,
        block_entity_types: &server.wire.block_entity_types,
    };
    let packets = pool.install(|| {
        crate::fanout::publish_with_registries(
            out,
            viewers.values_mut().filter(|viewer| {
                // A delayed pump may still be draining publications from
                // before queue registration. Do not replay those impulses
                // into the new client; its next eligible snapshot reconciles
                // the world view without advancing it through this old tick.
                !out.snapshot
                    .player_knockback_ticks
                    .get(&viewer.connection)
                    .is_some_and(|&(count, last_tick)| {
                        last_tick == out.tick
                            && count <= connections[&viewer.connection].knockback_baseline
                    })
            }),
            view_distance,
            &registries,
            chunk_cache,
        )
    });
    let recorders = crate::recorder::RecorderTick::of(viewers.values());
    // Reliable overflow takes precedence: disconnect victims first.
    for victim in crate::fanout::reliable_overflow(&packets) {
        server.connections.close(victim, "reliable-overflow");
        connections.remove(&victim);
        server
            .net_counters
            .reliable_drops
            .fetch_add(1, Ordering::Relaxed);
        viewers.remove(&victim);
    }
    route_planned_packets(server, viewers, connections, packets);
    flush_setup_world_fences(server, viewers, connections, out, view_distance);
    // Last: `/telemetry.jsonl` serves a tick only once its fanout is done.
    server.dashboard.record_fanout_tick(out.tick, recorders);
}

/// Preserve each viewer's world dependency order even if its writer frees
/// queue slots after a refused frame. Reliable traffic and other viewers
/// remain independent; snapshot repair retries the omitted world suffix.
fn route_planned_packets(
    server: &Arc<EdgeServer>,
    viewers: &mut HashMap<ConnectionId, crate::plan::ViewerState>,
    connections: &mut HashMap<ConnectionId, ConnectionHandles>,
    packets: impl IntoIterator<Item = crate::fanout::AddressedPacket>,
) {
    use crate::fanout::Priority;
    let mut refused = std::collections::HashSet::new();
    let mut packets = packets.into_iter();
    while let Some(packet) = packets.next() {
        let skip = !connections.contains_key(&packet.connection)
            || (packet.priority != Priority::Reliable && refused.contains(&packet.connection));
        if skip {
            // A skipped group still owns its following chunk/finished frames.
            // Do not enqueue them separately or charge unacknowledged debt.
            if let Priority::ChunkBatch(chunks) = packet.priority {
                for _ in packets.by_ref().take(chunks + 1) {}
            }
            continue;
        }
        let handles = &connections[&packet.connection];
        let world_refused = match packet.priority {
            Priority::Reliable => {
                match handles.reliable_tx.try_send(packet.data) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        server
                            .connections
                            .close(packet.connection, "reliable-queue-full");
                        server
                            .net_counters
                            .reliable_drops
                            .fetch_add(1, Ordering::Relaxed);
                        connections.remove(&packet.connection);
                        viewers.remove(&packet.connection);
                    }
                    // The session is already ending (its writer released the
                    // receivers before the routing view advanced): leave the
                    // local view and count nothing.
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        connections.remove(&packet.connection);
                        viewers.remove(&packet.connection);
                    }
                }
                false
            }
            Priority::ChunkBatch(chunks) => {
                let mut frames = Vec::with_capacity(chunks + 2);
                frames.push(packet.data);
                frames.extend(packets.by_ref().take(chunks + 1).map(|packet| packet.data));
                handles
                    .world_tx
                    .try_send_chunk_batch(&frames, chunks, &handles.chunk_batches)
                    .is_err()
            }
            Priority::BestEffort => handles.world_tx.try_send(packet.data).is_err(),
        };
        if world_refused {
            refused.insert(packet.connection);
            if let Some(viewer) = viewers.get_mut(&packet.connection) {
                viewer.world_frame_unsent();
            }
        }
    }
}

/// Broadcast tab-list latencies at vanilla cadence: every
/// [`LATENCY_BROADCAST_INTERVAL_TICKS`] ticks one batched
/// `PlayerInfoUpdate(UPDATE_LATENCY)` frame with every measured player goes
/// to every connection. That is one reliable-queue slot per interval per
/// connection, not one slot per other connection per keepalive reply: the
/// per-reply fan-out filled the 512-slot reliable queue of a slow reader in
/// seconds and the delivery policy disconnected it (`roster-reliable-full`).
pub(super) fn broadcast_latencies(
    registry: &ConnectionRegistry,
    net_counters: &NetCounters,
    connections: &mut HashMap<ConnectionId, ConnectionHandles>,
    tick: u64,
) {
    use mc_protocol::packets::Packet as _;
    if !tick.is_multiple_of(LATENCY_BROADCAST_INTERVAL_TICKS) {
        return;
    }
    let entries = registry.latencies();
    if entries.is_empty() {
        return;
    }
    let update = mc_protocol::packets::play::PlayerInfoUpdate {
        actions: mc_protocol::packets::play::PlayerInfoActions::UPDATE_LATENCY,
        entries: entries
            .into_iter()
            .map(
                |(profile_id, latency)| mc_protocol::packets::play::PlayerInfoEntry {
                    profile_id,
                    name: String::new(),
                    properties: Vec::new(),
                    listed: false,
                    latency,
                    game_mode: 0,
                    list_order: 0,
                    show_hat: false,
                },
            )
            .collect(),
    };
    let mut body = Vec::new();
    if update.encode(&mut body).is_err() {
        return;
    }
    let mut frame = bytes::BytesMut::new();
    if mc_protocol::frame::append_frame(
        &mut frame,
        mc_protocol::packets::play::PlayerInfoUpdate::ID,
        &body,
        mc_protocol::frame::Compression::Threshold(256),
    )
    .is_err()
    {
        return;
    }
    let frame = frame.freeze();
    let mut ended = Vec::new();
    for (id, handles) in connections.iter() {
        match handles.reliable_tx.try_send(frame.clone()) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                registry.close(*id, "roster-reliable-full");
                net_counters.reliable_drops.fetch_add(1, Ordering::Relaxed);
            }
            // A receiver the writer released is a session that is ending on
            // its own: drop it from the routing view, count nothing.
            Err(mpsc::error::TrySendError::Closed(_)) => ended.push(*id),
        }
    }
    for id in ended {
        // The pump's next publication refills the view from the registry;
        // adding it here is what keeps this tick from routing to a dead sender
        // again.
        connections.remove(&id);
    }
}

pub(super) fn flush_setup_world_fences(
    server: &EdgeServer,
    viewers: &HashMap<ConnectionId, crate::plan::ViewerState>,
    connections: &HashMap<ConnectionId, ConnectionHandles>,
    out: &TickOutput,
    view_distance: i32,
) {
    if server
        .connections
        .setup_fences_pending
        .load(Ordering::Acquire)
    {
        let mut registry = server.connections.inner.lock();
        registry
            .setup_world_fences
            .retain(|id, (after_tick, nonce)| {
                let Some(handles) = connections.get(id) else {
                    return false;
                };
                let Some(viewer) = viewers.get(id) else {
                    return false;
                };
                let Some(projection) = out.in_dimension(viewer.dimension) else {
                    return true;
                };
                if out.tick <= *after_tick
                    || !viewer.geometry_drained(view_distance, &projection.snapshot.chunks)
                    || handles.world_tx.capacity() == 0
                {
                    return true;
                }
                // The world FIFO contains every previously planned chunk/update.
                // A reliable response would be able to overtake that FIFO.
                let channel = mc_data::Identifier::parse(mc_protocol::SETUP_WORLD_FENCE_CHANNEL)
                    .expect("valid setup world fence channel");
                let frame = crate::lod::frame(channel, nonce.to_be_bytes().to_vec())
                    .expect("encode setup world fence");
                match handles.world_tx.try_send(frame) {
                    Ok(()) => false,
                    Err(_) => !handles.world_tx.is_closed(),
                }
            });
        if registry.setup_world_fences.is_empty() {
            server
                .connections
                .setup_fences_pending
                .store(false, Ordering::Release);
        }
    }
}

/// A connection's view radius: its request within the server limit, or the
/// whole limit for a recorder ([`crate::recorder`]).
pub(super) fn effective_view_distance(client: i8, server: u8, recorder: bool) -> i32 {
    let client = if recorder { i8::MAX } else { client };
    i32::from(client).max(2).min(i32::from(server))
}

/// Wait (bounded) for our [`PlayerChange::Joined`]. Only the pump drains
/// the core outbox; this waiter never touches it. Fast path: the pump may
/// have stashed our Joined already. Otherwise register a one-shot waiter
/// (fired by the pump's [`stash_join`][ConnectionRegistry::stash_join])
/// and await it — no polling, no draining, no other connection's changes.
/// A `recorder` joins in recorder mode ([`crate::recorder`]).
pub(super) async fn wait_for_join(
    server: &Arc<EdgeServer>,
    id: ConnectionId,
    recorder: bool,
    client_view_distance: i8,
    client_language: &str,
    chat_visibility: mc_core::boundary::ChatVisibility,
) -> anyhow::Result<JoinedSpawn> {
    {
        let mut inner = server.connections.inner.lock();
        if recorder {
            inner.recorders.insert(id);
        }
        inner.view_distances.insert(
            id,
            effective_view_distance(
                client_view_distance,
                server.cfg.server.view_distance,
                recorder,
            ),
        );
        // Already submitted before Join: remember it so the first Play
        // ClientInformation does not repeat the same locale.
        inner.locales.insert(id, client_language.to_owned());
        inner.chat_visibilities.insert(id, chat_visibility);
    }
    // Newest core tick the edge had seen when the wait began (timing lane,
    // independent of the pump): an estimate of the submit tick. The exact
    // ticks are the core's `join applied` and the pump's `join drained`
    // records, keyed by the same connection id as `join landed`.
    let waited_from = server.dashboard.latest_core_tick();
    // Fast path: Joined arrived before we registered interest.
    if let Some(joined) = server.connections.take_join(id) {
        join_landed(server, id, waited_from, "stashed");
        return Ok(joined);
    }
    let receiver = server.connections.wait_join(id);
    // Re-check after registering: the pump may have stashed between the
    // fast-path check and `wait_join` (stash fires waiters, but ours was
    // registered after the stash — `take_join` covers that window).
    if let Some(joined) = server.connections.take_join(id) {
        join_landed(server, id, waited_from, "rechecked");
        return Ok(joined);
    }
    let deadline = tokio::time::sleep(std::time::Duration::from_secs(10));
    tokio::pin!(deadline);
    tokio::select! {
        biased;
        _ = &mut deadline => {
            // A stopped core shows no tick progress; a pump behind the core
            // shows a published tick far behind the core's.
            let tick = |tick: Option<u64>| tick.map_or_else(|| "unknown".to_owned(), |tick| tick.to_string());
            anyhow::bail!(
                "timed out waiting for join (core tick {} seen when the wait began, {} now; pump published tick {})",
                tick(waited_from),
                tick(server.dashboard.latest_core_tick()),
                server.tick_stats.tick()
            )
        }
        result = receiver => {
            let joined = result.map_err(|_| anyhow::anyhow!("join waiter dropped before Joined"))?;
            join_landed(server, id, waited_from, "woken");
            Ok(joined)
        }
    }
}

/// Debug record of a join wait that ended: which path took the `Joined`, the
/// core ticks the edge had seen when the wait began and now (estimates), and
/// the pump's published tick.
fn join_landed(
    server: &EdgeServer,
    id: ConnectionId,
    waited_from: Option<u64>,
    path: &'static str,
) {
    tracing::debug!(
        connection = id.0,
        path,
        core_tick_seen_at_wait = ?waited_from,
        core_tick_seen = ?server.dashboard.latest_core_tick(),
        pump_tick = server.tick_stats.tick(),
        "join landed"
    );
}

/// Newest tick output available without blocking (for the join burst).
pub(super) fn latest_output(server: &Arc<EdgeServer>) -> TickOutput {
    let mut rx = server.hub.subscribe();
    // A fresh broadcast subscriber only sees outputs published after
    // subscribing; the join lands on the next tick, so fall back to an
    // empty output and let the burst carry position data.
    match rx.try_recv() {
        Ok(out) => out,
        Err(_) => empty_output(),
    }
}

pub(super) fn empty_output() -> TickOutput {
    TickOutput {
        tick: 0,
        snapshot: Arc::new(mc_core::boundary::WorldSnapshot {
            tick: 0,
            day_time: 0,
            chunks: Arc::default(),
            day_time_rate: 1.0,
            ..Default::default()
        }),
        changes: Arc::new(ChangeSet::default()),
        times: Default::default(),
    }
}

#[cfg(test)]
mod coreperf_visibility_tests {
    use super::*;

    #[test]
    fn cold_union_deduplicates_overlapping_viewers_and_enforces_bound() {
        let ids = [EntityId(1), EntityId(2), EntityId(2), EntityId(3)];
        assert_eq!(
            bounded_tracked_union(ids.len(), ids.into_iter(), 4),
            Some(3)
        );
        assert_eq!(bounded_tracked_union(ids.len(), ids.into_iter(), 3), None);
    }
}

#[cfg(test)]
mod roster_seed_tests {
    use super::*;
    use mc_core::boundary::PlayerChange;
    use mc_core::dimensions::RosterPlayer;
    use mc_protocol::frame::{Compression, try_decode_frame};
    use mc_protocol::packets::play::RemoveEntities;

    pub(super) fn test_server(root: &std::path::Path) -> Arc<EdgeServer> {
        let mut config = mc_core::CoreConfig::new(root.join("world"), 42);
        config.vanilla_data_dir =
            Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla"));
        let hub = Arc::new(mc_core::Core::open(config).unwrap());
        let mut cfg = EdgeConfig {
            server_dir: root.join("server"),
            ..Default::default()
        };
        cfg.auth.online_mode = false;
        std::fs::create_dir_all(&cfg.server_dir).unwrap();
        Arc::new(
            EdgeServer::new(
                hub,
                Arc::new(mc_data::krasnokamen_required_data()),
                Arc::default(),
                cfg,
            )
            .unwrap(),
        )
    }

    fn register(
        server: &EdgeServer,
        id: ConnectionId,
        bound: usize,
    ) -> mpsc::Receiver<bytes::Bytes> {
        let (reliable_tx, reliable_rx) = mpsc::channel(bound);
        let (world_tx, _world_rx) = world_queue(bound, WORLD_QUEUE_BYTES);
        server
            .connections
            .register(
                id,
                ConnectionHandles::new(reliable_tx, world_tx, crate::plan::CHUNKS_PER_TICK),
                || server.hub.published_knockback_count(id),
            )
            .unwrap();
        reliable_rx
    }

    /// A viewer created after other connections already routed the global
    /// roster must be seeded with those profiles. Publish can track entities
    /// for it before any Roster is routed to it; the next routed Roster must
    /// then NOT fan out one reliable RemoveEntities per tracked player (that
    /// burst alone overflows the 512-slot queue at 1000 players).
    /// Regression: without the seeding hunks this emits 600 RemoveEntities.
    #[tokio::test]
    async fn fresh_viewer_seeded_from_latest_roster_does_not_mass_remove_tracked_entities() {
        let root = tempfile::tempdir().unwrap();
        let server = test_server(root.path());
        let a = server.connections.joining_id();
        let _a_rx = register(&server, a, crate::fanout::RELIABLE_QUEUE_BOUND);

        let players: Arc<[RosterPlayer]> = (1..=600i32)
            .map(|i| RosterPlayer {
                connection: ConnectionId(u64::try_from(i).unwrap() + 1000),
                entity: EntityId(i),
                uuid: uuid::Uuid::from_u128(u128::try_from(i).unwrap()),
                name: format!("scale_{i:04}"),
                game_mode: 1,
            })
            .collect::<Vec<_>>()
            .into();
        let roster_change = || PlayerChange::Roster {
            players: Arc::clone(&players),
            removed: Arc::from([]),
        };

        let mut latest_roster: Arc<[RosterPlayer]> = Arc::from(Vec::new());
        let mut connections: HashMap<ConnectionId, ConnectionHandles> =
            server.connections.snapshot().into_iter().collect();
        let mut viewers: HashMap<ConnectionId, crate::plan::ViewerState> = HashMap::new();
        // Tick 1: connection A routes the roster; latest_roster now has 600 players.
        route_outbox_batches(
            &server,
            &mut viewers,
            &mut connections,
            vec![(a, vec![roster_change()])],
            &mut latest_roster,
        );
        assert_eq!(latest_roster.len(), 600);
        // Connection B registers only now (pre-registration rosters were stashed
        // and replayed through the throwaway join-replay viewer, as in the
        // xp-1000 failure), so its pump viewer is created after the roster.
        let b = server.connections.joining_id();
        let mut b_rx = register(&server, b, 8);
        connections.extend(
            server
                .connections
                .snapshot()
                .into_iter()
                .filter(|(id, _)| *id == b),
        );
        // Tick 2: no batches; B's viewer is created by the ensure loop and must
        // be seeded from latest_roster.
        route_outbox_batches(
            &server,
            &mut viewers,
            &mut connections,
            vec![],
            &mut latest_roster,
        );
        // Publish tracked 600 entities for B before any roster reached it.
        let viewer_b = viewers.get_mut(&b).expect("viewer B created");
        for i in 1..=600i32 {
            viewer_b
                .tracked_entities
                .insert(EntityId(i), crate::plan::TrackedEntity::default());
        }
        // Tick 3: the same roster routes to B through the reliable queue.
        route_outbox_batches(
            &server,
            &mut viewers,
            &mut connections,
            vec![(b, vec![roster_change()])],
            &mut latest_roster,
        );
        assert!(viewers.contains_key(&b), "B must survive routing");
        let mut remove_entities = 0usize;
        let mut frames = 0usize;
        while let Ok(data) = b_rx.try_recv() {
            frames += 1;
            let mut buf = bytes::BytesMut::from(data.as_ref());
            let frame = try_decode_frame(&mut buf, Compression::Threshold(256))
                .unwrap()
                .unwrap();
            if frame.id == RemoveEntities::ID {
                remove_entities += 1;
            }
        }
        assert_eq!(
            remove_entities, 0,
            "seeded viewer must not mass-remove tracked entities (frames={frames})"
        );
        assert_eq!(
            frames, 1,
            "only the single roster PlayerInfoUpdate is expected"
        );
    }
}

#[cfg(test)]
#[path = "pump_delivery_tests.rs"]
mod delivery_tests;

#[cfg(test)]
mod lag_tests {
    use super::*;

    #[test]
    fn unset_or_zero_is_todays_pump() {
        assert_eq!(lag_ticks(None), Ok(None));
        assert_eq!(lag_ticks(Some("0")), Ok(None));
        assert_eq!(lag_ticks(Some(" 0 ")), Ok(None));
    }

    #[test]
    fn a_count_is_taken_as_lag() {
        assert_eq!(lag_ticks(Some("32")), Ok(Some(32)));
        assert_eq!(lag_ticks(Some("1")), Ok(Some(1)));
    }

    #[test]
    fn an_invalid_value_is_an_error_not_an_ignore() {
        for raw in ["abc", "", "-1", "1.5", "inf", "NaN", "32t"] {
            assert!(lag_ticks(Some(raw)).is_err(), "{raw:?} must be refused");
        }
    }

    #[test]
    fn an_unset_env_leaves_the_pump_unwaited() {
        // Reading is safe; the crate forbids env mutation, so a gate that exports the variable is
        // respected by leaving this assertion to the unset case (lag_ticks covers the other values).
        if std::env::var(LAG_ENV).is_ok() {
            return;
        }
        assert_eq!(lag_ticks_from_env().expect("unset is fine"), None);
    }

    #[test]
    fn the_wait_counts_tick_events_and_lagged_runs() {
        // Fed events, no core and no timer: three single ticks reach a count of three.
        let mut remaining = 3u64;
        for _ in 0..3 {
            remaining = pace_count(remaining, Ok(()));
        }
        assert_eq!(remaining, 0);
        // A lagged run of five satisfies a wait of five in one event.
        assert_eq!(pace_count(5, Err(5)), 0);
        // A lagged run larger than the wait saturates instead of underflowing.
        assert_eq!(pace_count(2, Err(9)), 0);
        // A short run leaves the rest to wait.
        assert_eq!(pace_count(9, Err(4)), 5);
    }

    #[test]
    fn a_publication_follows_the_wait() {
        // The wait releases the pump only after the count is spent; the next call publishes.
        let mut remaining = lag_ticks(Some("2")).expect("valid").expect("set");
        let mut published = 0;
        for event in [Ok(()), Ok(())] {
            remaining = pace_count(remaining, event);
            if remaining == 0 {
                published += 1;
            }
        }
        assert_eq!(published, 1);
    }
}
