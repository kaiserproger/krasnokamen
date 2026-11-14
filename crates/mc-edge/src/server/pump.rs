//! Tick output pump: outbox routing, publication and join waiting.
//! Moved verbatim from `server.rs`.

use super::*;
use mc_core::boundary::EntityId;

const COREPERF_COLD_PAIR_LIMIT: usize = 1_000_000;

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

/// Single pump task: subscribes to the core, records tick stats, drains the
/// reliable outbox FIRST, then publishes the world snapshot and routes both.
/// `Lagged` = world-skip (viewers reconcile on the next output) but the outbox is still
/// drained: reliable results are never skipped. Fanout runs on its own
/// rayon pool: on the global pool the core tick's parallel phases queued
/// behind whole publishes.
pub(super) fn spawn_pump(server: Arc<EdgeServer>) -> anyhow::Result<tokio::task::JoinHandle<()>> {
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
        let coreperf = std::env::var_os("KRASNOKAMEN_COREPERF_REGIONS").is_some();
        loop {
            let out = match rx.recv().await {
                Ok(out) => out,
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    // World-skip, not data loss: the next received output
                    // is reconciled against each viewer's held state. The
                    // reliable outbox is drained regardless (never
                    // skipped). A lag is never fatal: the next `recv`
                    // either returns the newest output or reports a further
                    // lag handled here again.
                    server.dashboard.record_pump_skipped(skipped);
                    for viewer in viewers.values_mut() {
                        viewer.skipped = true;
                    }
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => {
                    // Drain terminal reliable disconnects before closing, both
                    // for graceful shutdown and for an authoritative failure.
                    let (mut connections, _) = server.connections.routing_snapshot();
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
                }
            };
            server.tick_stats.record(out.tick);
            let (mut connections, roster) = server.connections.routing_snapshot();
            route_outbox(
                &server,
                &mut viewers,
                &mut connections,
                Some(out.tick),
                &mut latest_roster,
            );
            server.console_state.drained(out.tick);
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
                .map(|(entity, position, _, dimension)| (entity, position, dimension))
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
                .map(|(entity, position, _, dimension)| (entity, position, dimension))
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
            && server
                .send(
                    connection,
                    &crate::command_tree::tree(
                        server.admin.level(uuid),
                        server.hub.plugin_commands(),
                    ),
                )
                .is_err()
        {
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
    for planned in crate::plan::plan_routed_player_changes(changes, viewer) {
        if handles.reliable_tx.try_send(planned.data).is_err() {
            server
                .connections
                .close(viewer.connection, "route-changes-reliable-full");
            server
                .net_counters
                .reliable_drops
                .fetch_add(1, Ordering::Relaxed);
            return false;
        }
    }
    true
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
    // Planning spends this credit, but it is not exclusive: an LOD tile
    // (`lod.rs`) also takes world-queue slots, at any time from this read
    // until the send, which is the whole planning time of every viewer, so a
    // planned frame can still find the queue full below.
    {
        let registry = server.connections.inner.lock();
        for (id, viewer) in viewers.iter_mut() {
            viewer.view_distance = registry.view_distances.get(id).copied();
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
            viewers.values_mut(),
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
    for packet in packets {
        let Some(handles) = connections.get(&packet.connection) else {
            continue;
        };
        match packet.priority {
            crate::fanout::Priority::Reliable => {
                // In-order, never dropped: a full reliable queue
                // disconnects the connection.
                if handles.reliable_tx.try_send(packet.data).is_err() {
                    server
                        .connections
                        .close(packet.connection, "reliable-queue-full");
                    connections.remove(&packet.connection);
                    server
                        .net_counters
                        .reliable_drops
                        .fetch_add(1, Ordering::Relaxed);
                    viewers.remove(&packet.connection);
                }
            }
            crate::fanout::Priority::BestEffort => {
                if handles.world_tx.try_send(packet.data).is_err()
                    && let Some(viewer) = viewers.get_mut(&packet.connection)
                {
                    // The viewer recorded this publication as delivered:
                    // repair the geometry and entity state it carried.
                    viewer.world_frame_unsent();
                }
            }
        }
    }
    flush_setup_world_fences(server, viewers, connections, out, view_distance);
    // Last: `/telemetry.jsonl` serves a tick only once its fanout is done.
    server.dashboard.record_fanout_tick(out.tick, recorders);
}

/// Broadcast tab-list latencies at vanilla cadence: every
/// [`LATENCY_BROADCAST_INTERVAL_TICKS`] ticks one batched
/// `PlayerInfoUpdate(UPDATE_LATENCY)` frame with every measured player goes
/// to every connection. That is one reliable-queue slot per interval per
/// connection, not one slot per other connection per keepalive reply: the
/// per-reply fan-out filled the 512-slot reliable queue of a slow reader in
/// seconds and the delivery policy disconnected it.
pub(super) fn broadcast_latencies(
    registry: &ConnectionRegistry,
    net_counters: &NetCounters,
    connections: &mut HashMap<ConnectionId, ConnectionHandles>,
    tick: u64,
) {
    use mc_protocol::packets::Packet as _;
    if tick % LATENCY_BROADCAST_INTERVAL_TICKS != 0 {
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
    for (id, handles) in connections.iter() {
        if handles.reliable_tx.try_send(frame.clone()).is_err() {
            registry.remove(*id);
            net_counters.reliable_drops.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Broadcast tab-list latencies at vanilla cadence: every
/// [`LATENCY_BROADCAST_INTERVAL_TICKS`] ticks one batched
/// `PlayerInfoUpdate(UPDATE_LATENCY)` frame with every measured player goes
/// to every connection. That is one reliable-queue slot per interval per
/// connection, not one slot per other connection per keepalive reply: the
/// per-reply fan-out filled the 512-slot reliable queue of a slow reader in
/// seconds and the delivery policy disconnected it.
pub(super) fn broadcast_latencies(
    registry: &ConnectionRegistry,
    net_counters: &NetCounters,
    connections: &mut HashMap<ConnectionId, ConnectionHandles>,
    tick: u64,
) {
    use mc_protocol::packets::Packet as _;
    if tick % LATENCY_BROADCAST_INTERVAL_TICKS != 0 {
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
    for (id, handles) in connections.iter() {
        if handles.reliable_tx.try_send(frame.clone()).is_err() {
            registry.remove(*id);
            net_counters.reliable_drops.fetch_add(1, Ordering::Relaxed);
        }
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
    // Fast path: Joined arrived before we registered interest.
    if let Some(joined) = server.connections.take_join(id) {
        return Ok(joined);
    }
    let receiver = server.connections.wait_join(id);
    // Re-check after registering: the pump may have stashed between the
    // fast-path check and `wait_join` (stash fires waiters, but ours was
    // registered after the stash — `take_join` covers that window).
    if let Some(joined) = server.connections.take_join(id) {
        return Ok(joined);
    }
    let deadline = tokio::time::sleep(std::time::Duration::from_secs(10));
    tokio::pin!(deadline);
    tokio::select! {
        biased;
        _ = &mut deadline => anyhow::bail!("timed out waiting for join"),
        result = receiver => {
            result.map_err(|_| anyhow::anyhow!("join waiter dropped before Joined"))
        }
    }
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

    fn test_server(root: &std::path::Path) -> Arc<EdgeServer> {
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
                ConnectionHandles {
                    reliable_tx,
                    world_tx,
                },
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
