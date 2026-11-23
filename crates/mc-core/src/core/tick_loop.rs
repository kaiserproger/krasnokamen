//! Tick thread loop, shutdown and authority halt.
//! Moved verbatim from `core.rs`.

use super::*;
use crate::singleplayer_pause::{SingleplayerPause, online_players};
use crate::tick::phase_laps::Laps;
use std::sync::atomic::{AtomicU64, Ordering};

/// Tick loop: 20 TPS, event-driven wake (no sleeps beyond the tick timer).
/// Channels owned by the tick thread.
pub(super) struct TickChannels {
    pub(super) checkpoint_every: u64,
    /// Vanilla `pause-when-empty-seconds`; zero never pauses.
    pub(super) pause_when_empty_seconds: u32,
    pub(super) input_rx: Receiver<PlayerInput>,
    pub(super) spawn_rx: Receiver<SpawnRequest>,
    pub(super) ticket_rx: Receiver<RemoteTicket>,
    pub(super) moderation_selector_rx: Receiver<moderation_selector::Request>,
    /// Reliable FIFO handoff; only the tick owns the sender. Bounded in bytes
    /// with a non-blocking fail-closed push (see `core/outbox.rs`).
    pub(super) outbox: OutboxSender,
    /// Tick-side time bound on a non-draining outbox (see `core/outbox.rs`).
    pub(super) outbox_stall: StallWatch,
    pub(super) tick_broadcast: broadcast::Sender<TickOutput>,
    pub(super) knockback_publication: KnockbackPublication,
    pub(super) timing_broadcast: broadcast::Sender<(u64, TickTimes)>,
    #[cfg(test)]
    pub(super) pause_state: tokio::sync::watch::Sender<Option<u64>>,
    pub(super) work_tx: std::sync::mpsc::SyncSender<JournalWork>,
    /// Byte bound on queued `Append` payload (see `journal_budget`).
    pub(super) journal_budget: Arc<JournalBudget>,
    /// Edge-visible projection of tick-observed unlocked sessions.
    pub(super) unlocked_sessions: Arc<session_mirror::SessionMirror>,
    /// Set once by [`halt_authority`] before the tick broadcast closes, so the
    /// edge reads the halt's own reason on `Closed` without draining the outbox.
    pub(super) halt_reason: Arc<std::sync::OnceLock<String>>,
    pub(super) shutdown_rx: std::sync::mpsc::Receiver<()>,
    pub(super) checkpoint_rx: std::sync::mpsc::Receiver<std::sync::mpsc::Sender<bool>>,
    pub(super) backup_rx: std::sync::mpsc::Receiver<BackupJob>,
    /// Coherent terrain publications, one per dimension index: swapped by
    /// the tick owner under each source's write lock so edge samplers read
    /// a consistent (snapshot, supplement) bundle plus fence without state.
    pub(super) terrain: [Arc<crate::terrain::TerrainSource>; 3],
    /// Opt-in per-tick input log (`CoreConfig::input_log`).
    pub(super) input_log: Option<crate::input_log::InputLog>,
}

pub(super) fn tick_loop(mut state: TickState, mut channels: TickChannels) {
    let mut next_tick = std::time::Instant::now();
    let mut admission_reserve_us = state
        .tick_control
        .period()
        .as_micros()
        .saturating_sub(crate::world::ADMISSION_BUDGET_US);
    let admission_progress_reservations = Arc::new(AtomicU64::new(0));
    let checkpoint_every = channels.checkpoint_every.max(1);
    let mut since_checkpoint = 0u64;
    // At most one checkpoint in flight: its acknowledgement (chunks written
    // at which revision, or failure) returns here and is drained each tick.
    let (checkpoint_done_tx, checkpoint_done_rx) = std::sync::mpsc::channel::<CheckpointAck>();
    let mut checkpoint_in_flight = false;
    // Disconnects due for Leave-equivalent removal (sealed a full tick ago,
    // so the edge has drained the terminal `Disconnect`).
    let mut pending_leaves: Vec<ConnectionId> = Vec::new();
    let mut pending_input = None;
    let (plugin_ack_tx, plugin_ack_rx) = std::sync::mpsc::channel();
    let mut plugin_written_tick = state.tick;
    // Empty-server pause bookkeeping (vanilla `pause-when-empty-seconds`).
    let mut empty_ticks = 0u64;
    let mut paused = false;
    let mut singleplayer_pause = SingleplayerPause::default();
    let mut knockback_ticks = Arc::clone(&channels.knockback_publication.read());
    // Session state the published unlocked-session mirror was built from; the
    // projection runs only when this pair moves.
    let mut published_sessions = (0u64, 0u64);
    'ticks: loop {
        let wall_start = std::time::Instant::now();
        let wall_start_unix_ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos().min(u128::from(u64::MAX)) as u64)
            .unwrap_or(0);
        while let Ok(written) = checkpoint_done_rx.try_recv() {
            checkpoint_in_flight = false;
            acknowledge_checkpoint(&mut state, written);
        }
        while let Ok(tick) = plugin_ack_rx.try_recv() {
            plugin_written_tick = plugin_written_tick.max(tick);
        }
        crate::plugin::acknowledge_events(&mut state, plugin_written_tick);
        // Event-driven input drain: everything submitted before this tick.
        let mut inputs = Vec::new();
        for connection in std::mem::take(&mut pending_leaves) {
            inputs.push(PlayerInput::Leave { connection });
        }
        if let Some(input) = pending_input.take() {
            inputs.push(input);
        }
        while let Ok(input) = channels.input_rx.try_recv() {
            inputs.push(input);
        }
        let singleplayer_runs =
            match singleplayer_pause.filter_inputs(&state, &mut inputs, &channels.outbox) {
                Ok(runs) => runs,
                Err(refusal) => {
                    // A refused reliable pause answer is a halt, never a silent
                    // drop: the same fail-closed path as the tick's own push.
                    halt_authority(&mut state, &channels, &refusal.reason());
                    break;
                }
            };
        let mut activity = !inputs.is_empty();
        while let Ok(request) = channels.moderation_selector_rx.try_recv() {
            moderation_selector::answer(&state, request);
        }
        // Debug/test spawns drain here (bench + tests only).
        let logging = channels.input_log.is_some();
        let mut logged_spawns = Vec::new();
        while singleplayer_runs && let Ok(request) = channels.spawn_rx.try_recv() {
            if logging {
                logged_spawns.push(crate::input_log::LoggedSpawn::new(&request));
            }
            apply_spawn_request(&mut state, &request);
            activity = true;
        }
        // Remote tickets drain here; the journal records them each tick.
        let mut logged_tickets = Vec::new();
        while singleplayer_runs && let Ok(ticket) = channels.ticket_rx.try_recv() {
            if logging {
                logged_tickets.push(crate::input_log::LoggedTicket {
                    owner: ticket.owner.clone(),
                    x: ticket.chunk.x,
                    z: ticket.chunk.z,
                    radius: ticket.radius,
                });
            }
            apply_ticket(
                &mut state,
                ticket.owner,
                ticket.chunk.x,
                ticket.chunk.z,
                ticket.radius,
            );
            activity = true;
        }
        // Vanilla `pause-when-empty-seconds`: after the grace period with no
        // online player the world stops advancing. Inputs (a join, a console
        // command) wake one tick; the first player resets the counter and
        // resumes continuous ticking. Saves, backups and shutdown stay
        // serviced while paused, exactly as the vanilla server does.
        let pause_limit = u64::from(channels.pause_when_empty_seconds) * 20;
        let pause_tick = state.tick;
        if pause_limit == 0 {
            paused = false;
        } else if online_players(&state) == 0 {
            empty_ticks = empty_ticks.saturating_add(1);
            if empty_ticks >= pause_limit && !paused {
                paused = true;
                tracing::info!(
                    seconds = channels.pause_when_empty_seconds,
                    "server empty, pausing world"
                );
                // Vanilla autoSave()-s once on entering the pause.
                if !checkpoint_in_flight {
                    checkpoint_in_flight = start_checkpoint(
                        &mut state,
                        &channels.work_tx,
                        &checkpoint_done_tx,
                        pause_tick,
                    );
                    if checkpoint_in_flight {
                        since_checkpoint = 0;
                    }
                }
            }
        } else {
            empty_ticks = 0;
            paused = false;
        }
        let pause_world = (paused && !activity) || !singleplayer_runs;
        #[cfg(test)]
        channels.pause_state.send_if_modified(|observed| {
            let next = pause_world.then_some(pause_tick);
            if *observed == next {
                return false;
            }
            *observed = next;
            true
        });
        if pause_world {
            // A paused world still owns a charged outbox: check the stall here,
            // before any new checkpoint admission, or a wedged pump could hold
            // the queue forever while simulation is stopped.
            if let Some(stalled) = channels
                .outbox_stall
                .stalled_at(channels.outbox.budget(), std::time::Instant::now())
            {
                halt_authority(
                    &mut state,
                    &channels,
                    &format!(
                        "edge pump not draining for {} s, {} bytes queued while the world is paused (fail closed)",
                        stalled.as_secs(),
                        channels.outbox.queued_bytes()
                    ),
                );
                break;
            }
            if !checkpoint_in_flight && let Ok(reply) = channels.checkpoint_rx.try_recv() {
                checkpoint_in_flight = start_checkpoint_with_reply(
                    &mut state,
                    &channels.work_tx,
                    &checkpoint_done_tx,
                    pause_tick,
                    Some(reply),
                );
                if checkpoint_in_flight {
                    since_checkpoint = 0;
                }
            }
            while let Ok(job) = channels.backup_rx.try_recv() {
                if let Err(std::sync::mpsc::SendError(JournalWork::Backup(job))) =
                    channels.work_tx.send(JournalWork::Backup(job))
                {
                    let _ = job
                        .reply
                        .send(Err(CoreError::Storage("journal thread stopped".into())));
                }
            }
            if channels.shutdown_rx.try_recv().is_ok() {
                start_checkpoint(
                    &mut state,
                    &channels.work_tx,
                    &checkpoint_done_tx,
                    pause_tick,
                );
                send_shutdown(&mut state, &channels.work_tx);
                break;
            }
            next_tick += TICK_PERIOD;
            let now = std::time::Instant::now();
            if next_tick <= now {
                next_tick = now + TICK_PERIOD;
            }
            if channels.shutdown_rx.recv_timeout(next_tick - now).is_ok() {
                start_checkpoint(
                    &mut state,
                    &channels.work_tx,
                    &checkpoint_done_tx,
                    pause_tick,
                );
                send_shutdown(&mut state, &channels.work_tx);
                break;
            }
            continue;
        }
        let tick_start = std::time::Instant::now();
        // One deadline across every dimension and repeated ensure call. Reserve
        // measured non-admission work rather than granting each world 50 ms.
        let admission_slack_us = state
            .tick_control
            .period()
            .as_micros()
            .saturating_sub(admission_reserve_us);
        let admission_deadline = wall_start
            + std::time::Duration::from_micros(admission_slack_us.min(u64::MAX as u128) as u64);
        admission_progress_reservations.store(0, Ordering::Relaxed);
        state
            .world
            .begin_admission_tick(admission_deadline, &admission_progress_reservations);
        for dimension in state.other_dimensions.values_mut() {
            dimension
                .world
                .begin_admission_tick(admission_deadline, &admission_progress_reservations);
        }
        // Secrets: connections the auth gate holds locked, before and after
        // this tick (a join locks during it; a plugin unlock lifts it).
        let mut locked = Vec::new();
        if logging && state.require_plugin_login {
            locked = locked_connections(&state, &inputs);
        }
        let (mut output, drains, times) = run_tick(&mut state, &inputs);
        observe_explosion_knockback(&state, &mut output, &mut knockback_ticks);
        if logging && state.require_plugin_login {
            locked.extend(locked_connections(&state, &inputs));
        }
        let run_time = tick_start.elapsed();
        // Journal: this tick's entries (blocks + players + entities). `send`
        // blocks while the bounded queue is full (backpressure) and fails
        // only when the journal thread is gone (receiver disconnected):
        // HALT — stop committing and publishing. Disconnect everyone so
        // the edge drops the world instead of playing on undurable state,
        // refuse new inputs, publish nothing further.
        let entries = match journal_entries_for_tick(&mut state, &output) {
            Ok(entries) => entries,
            Err(error) => {
                halt_authority(
                    &mut state,
                    &channels,
                    &format!("journal capture failed: {error}"),
                );
                break;
            }
        };
        let journal_time = tick_start.elapsed();
        let journal_queue_start = std::time::Instant::now();
        let tick = output.tick;
        // The input log gets exactly what this tick consumed; the drained
        // vectors move to the writer thread (no clone, no I/O on this path).
        if let Some(log) = &mut channels.input_log {
            log.record(tick, inputs, logged_spawns, logged_tickets, locked);
        }
        let out_tick = tick;
        let plugin_effects = match crate::plugin::seal_events(&mut state, tick) {
            Ok(active) => active,
            Err(reason) => {
                halt_authority(&mut state, &channels, reason);
                break;
            }
        };
        let generated_time = match send_tick_journal(
            &mut state,
            &channels.work_tx,
            &channels.journal_budget,
            tick,
            entries,
            plugin_effects.then(|| plugin_ack_tx.clone()),
        ) {
            Ok(generated_time) => generated_time,
            Err(reason) => {
                halt_authority(&mut state, &channels, reason);
                break;
            }
        };
        let journal_queue_time = journal_queue_start.elapsed();
        // Timings ride in the output they describe (never a shared
        // last-tick slot a reader could pair with the wrong tick).
        output.times = TickTimes {
            wall_start_unix_ns,
            owner_pre_us: tick_start.duration_since(wall_start).as_micros() as u64,
            journal_build_us: (journal_time - run_time).as_micros() as u64,
            journal_queue_us: journal_queue_time.as_micros() as u64,
            generated_us: generated_time.as_micros() as u64,
            total_us: tick_start.elapsed().as_micros().min(u64::MAX as u128) as u64,
            ..times
        };
        let owner_post_start = std::time::Instant::now();
        // Hand off before waking edge subscribers. The receiver coalesces
        // batches in FIFO order and never extends a sealed Disconnect queue.
        // The push charges the batch against the byte cap and never blocks: a
        // batch that does not fit stops the core fail-closed instead of being
        // dropped (see `core/outbox.rs`).
        if !drains.is_empty()
            && let Err(refusal) = channels.outbox.push(output.tick, drains)
        {
            halt_authority(&mut state, &channels, &refusal.reason());
            break;
        }
        // A pump that is alive but not keeping up is invisible to task
        // supervision; a non-empty outbox whose drain epoch has not moved for
        // the whole limit halts the same way.
        if let Some(stalled) = channels
            .outbox_stall
            .stalled_at(channels.outbox.budget(), std::time::Instant::now())
        {
            halt_authority(
                &mut state,
                &channels,
                &format!(
                    "edge pump not draining for {} s, {} bytes queued (fail closed)",
                    stalled.as_secs(),
                    channels.outbox.queued_bytes()
                ),
            );
            break;
        }
        // Project the mirror only when a session joined, left, moved or
        // changed its plugin-login lock: an unchanged tick compares two
        // counters, and the projection itself is built outside the lock.
        if state.require_plugin_login {
            let stamp = session_mirror::stamp(&state);
            if stamp != published_sessions {
                published_sessions = stamp;
                channels
                    .unlocked_sessions
                    .publish(session_mirror::unlocked_sessions(&state));
            }
        }

        // Broadcast: laggards skip to latest (world state is skippable).
        // Terrain freshness: publish the coherent per-dimension bundle
        // (resident snapshot + committed-but-unflushed supplement, fenced)
        // so off-tick sampling never mixes a new snapshot with an old
        // supplement (or the reverse) under an unchanged fence.
        // Block edits, chunk loads and unloads all surface in the change
        // set; anything else is not terrain. Changed publications advance
        // each per-dimension fence; idle ticks leave every fence (and every
        // bundle) untouched so the edge before/after sampling check does
        // not reject slow generations and the shared cache does not
        // invalidate continuously.
        let terrain_changed = crate::dimensions::Dimension::ALL
            .into_iter()
            .any(|dimension| {
                output.in_dimension(dimension).is_some_and(|output| {
                    !output.changes.blocks.is_empty()
                        || !output.changes.chunks_loaded.is_empty()
                        || !output.changes.chunks_unloaded.is_empty()
                })
            });
        if terrain_changed {
            // Mark every fence busy BEFORE swapping any bundle: a sampler
            // that grabbed a pre-fence retries instead of mixing a swapped
            // dimension bundle with a not-yet-swapped one.
            for source in &channels.terrain {
                source.begin_publish();
            }
        }
        // Committed-but-unflushed supplement per dimension: resident dirty
        // chunks plus non-resident overlay writes. Covers edit -> evict ->
        // sample before checkpoint, when neither the snapshot nor the
        // native store carries the edit. Shared-empty when clean (no
        // per-tick allocation); rebuilt every publication so flushed
        // entries drop as soon as the checkpoint ACK clears them.
        // Per-dimension bundle swap, indexed explicitly
        // (0 = overworld, 1 = nether, 2 = end): never rely on map order.
        for (index, dimension) in crate::dimensions::Dimension::ALL.into_iter().enumerate() {
            let world = if dimension == state.dimension {
                &state.world
            } else {
                let Some(world) = state.other_dimensions.get(&dimension) else {
                    // Dimension absent this tick (never on the live path:
                    // all three exist): on a changed publication the fence
                    // is already busy and the bundle swaps below with no
                    // snapshot; on idle there is nothing to publish.
                    if terrain_changed {
                        let source = &channels.terrain[index];
                        source.finish_publish(
                            None,
                            crate::terrain::TerrainSource::empty_supplement(),
                            output.tick,
                        );
                    }
                    continue;
                };
                &world.world
            };
            let source = &channels.terrain[index];
            if !terrain_changed {
                // Idle: leave the fence AND the bundle untouched (no
                // re-swap of identical Arcs) so long generations under a
                // stable fence are never spuriously rejected.
                continue;
            }
            let revision = output.tick;
            let snapshot = output
                .in_dimension(dimension)
                .map(|output| Arc::clone(&output.snapshot));
            let dirty = world.dirty_supplement_data();
            let supplement = if dirty.chunks.is_empty() && dirty.overlay.is_empty() {
                crate::terrain::TerrainSource::empty_supplement()
            } else {
                Arc::new(dirty)
            };
            source.finish_publish(snapshot, supplement, revision);
        }
        let mut tick_times = output.times.clone();
        {
            // Registration reads its count while holding the edge registry
            // lock. Swap the canonical Arc and send under this write guard:
            // registration therefore falls wholly before or after publication.
            // This synchronous broadcast never enters the edge registry, so
            // there is no reverse lock order and no second count authority.
            let mut published = channels.knockback_publication.write();
            if !Arc::ptr_eq(&published, &output.snapshot.player_knockback_ticks) {
                *published = Arc::clone(&output.snapshot.player_knockback_ticks);
            }
            output.times.published_at = Some(std::time::Instant::now());
            let _ = channels.tick_broadcast.send(output);
        }
        // Disconnects sealed before this tick: the edge has had a full tick
        // to drain the `Disconnect`, so remove the player as a Leave would
        // (via synthetic input next tick, so despawns flow through `changes`).
        let due: Vec<ConnectionId> = state
            .disconnects
            .iter()
            .filter(|(_, sealed)| *sealed < tick)
            .map(|(connection, _)| *connection)
            .collect();
        state.disconnects.retain(|(_, sealed)| *sealed >= tick);
        pending_leaves.extend(due);
        // The request channel is already FIFO. Leave requests there while the
        // prior capture is in flight, then admit one before the cadence check.
        if !checkpoint_in_flight && let Ok(reply) = channels.checkpoint_rx.try_recv() {
            checkpoint_in_flight = start_checkpoint_with_reply(
                &mut state,
                &channels.work_tx,
                &checkpoint_done_tx,
                out_tick,
                Some(reply),
            );
            if checkpoint_in_flight {
                since_checkpoint = 0;
            }
        }
        // Hot backup requests ride the same FIFO as checkpoints, so the journal
        // thread sees them behind every append of this and earlier ticks.
        while let Ok(job) = channels.backup_rx.try_recv() {
            if let Err(std::sync::mpsc::SendError(JournalWork::Backup(job))) =
                channels.work_tx.send(JournalWork::Backup(job))
            {
                let _ = job
                    .reply
                    .send(Err(CoreError::Storage("journal thread stopped".into())));
            }
        }
        // Checkpoint: every `checkpoint_every` ticks, one in flight at a
        // time (a cadence that comes due while one is writing starts as soon
        // as it is acknowledged). See `start_checkpoint`.
        since_checkpoint += 1;
        if since_checkpoint >= checkpoint_every && !checkpoint_in_flight {
            since_checkpoint = 0;
            checkpoint_in_flight =
                start_checkpoint(&mut state, &channels.work_tx, &checkpoint_done_tx, out_tick);
        }
        // Shutdown check (non-blocking): final checkpoint first so the
        // reopened world sees this tick's domains, then Shutdown (FIFO after
        // it on the journal thread) and exit.
        if channels.shutdown_rx.try_recv().is_ok() {
            crate::battle::halt_exposure(&mut state);
            start_checkpoint(&mut state, &channels.work_tx, &checkpoint_done_tx, out_tick);
            send_shutdown(&mut state, &channels.work_tx);
            tick_times.owner_post_us = owner_post_start.elapsed().as_micros() as u64;
            tick_times.wall_us = wall_start.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
            let _ = channels.timing_broadcast.send((tick, tick_times));
            break;
        }
        tick_times.owner_post_us = owner_post_start.elapsed().as_micros() as u64;
        tick_times.wall_us = wall_start.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
        let admission_us = state.world.admission_elapsed().as_micros()
            + state
                .other_dimensions
                .values()
                .map(|dimension| dimension.world.admission_elapsed().as_micros())
                .sum::<u128>();
        let other_us = u128::from(tick_times.wall_us).saturating_sub(admission_us);
        // React immediately to expensive later phases; release their reserve
        // gradually, so one cheap tick cannot promise all of the next tick.
        admission_reserve_us = other_us.max((admission_reserve_us * 3 + other_us).div_ceil(4));
        state.tick_control.record_tick(tick_times.wall_us);
        if std::env::var("MC_CORE_PHASE_LOG").is_ok() {
            let pre_time = tick_start.duration_since(wall_start);
            let mut outer = Laps::new(true, wall_start);
            outer.record("pre", pre_time);
            outer.record("run", run_time);
            outer.record("journal", journal_time.saturating_sub(run_time));
            outer.record(
                "seal_send_generated",
                std::time::Duration::from_micros(tick_times.total_us).saturating_sub(journal_time),
            );
            outer.record(
                "post",
                std::time::Duration::from_micros(
                    tick_times
                        .wall_us
                        .saturating_sub(tick_times.total_us)
                        .saturating_sub(pre_time.as_micros() as u64),
                ),
            );
            outer.print_summary("tick outer", format_args!("tick={tick}"));
            // Stages of `journal` above, in order; they partition it.
            let build = state.journal_scan.times;
            let mut journal = Laps::new(true, wall_start);
            journal.record("before_entities", build.before_entities);
            journal.record("entity_touched", build.touched);
            journal.record("entity_images", build.images);
            journal.record("entity_compare", build.compare);
            journal.record("after_entities", build.after_entities);
            journal.print_summary("journal build", format_args!("tick={tick}"));
        }
        let _ = channels.timing_broadcast.send((tick, tick_times));
        // Keep the hook-free shutdown/deadline wait unchanged. A hook-enabled
        // tick also services read-only previews when explicitly unparked: that
        // spends no extra game tick and never renews the admission deadline.
        next_tick += state.tick_control.period();
        let now = std::time::Instant::now();
        // A sprint runs ticks back to back; it still takes the shutdown poll
        // of the behind-schedule branch below.
        if state.tick_control.sprinting() {
            next_tick = now;
        }
        if next_tick > now {
            if state.build_queries.is_none() {
                if channels.shutdown_rx.recv_timeout(next_tick - now).is_ok() {
                    start_checkpoint(&mut state, &channels.work_tx, &checkpoint_done_tx, out_tick);
                    send_shutdown(&mut state, &channels.work_tx);
                    break;
                }
            } else {
                loop {
                    if channels.shutdown_rx.try_recv().is_ok() {
                        start_checkpoint(
                            &mut state,
                            &channels.work_tx,
                            &checkpoint_done_tx,
                            out_tick,
                        );
                        send_shutdown(&mut state, &channels.work_tx);
                        break 'ticks;
                    }
                    // Retire FIFO metadata fences without simulating another
                    // tick. Stop at the first real input: no preview behind it
                    // can become ready until that input has actually applied.
                    for _ in 0..mc_script::precommit::MAX_PRECOMMIT_REQUESTS {
                        if pending_input.is_some() {
                            break;
                        }
                        match channels.input_rx.try_recv() {
                            Ok(PlayerInput::BuildPreviewFence { fence }) => fence.apply(),
                            Ok(input) => pending_input = Some(input),
                            Err(_) => break,
                        }
                    }
                    crate::build_precommit::answer_previews(&mut state);
                    let remaining = next_tick.saturating_duration_since(std::time::Instant::now());
                    if remaining.is_zero() {
                        break;
                    }
                    std::thread::park_timeout(remaining);
                }
            }
        } else {
            // Behind: skip catch-up waits, never burst. Still poll
            // shutdown once so a queued stop never stalls behind load.
            next_tick = now;
            if channels.shutdown_rx.try_recv().is_ok() {
                start_checkpoint(&mut state, &channels.work_tx, &checkpoint_done_tx, out_tick);
                send_shutdown(&mut state, &channels.work_tx);
                break;
            }
        }
    }
    if let Some(log) = channels.input_log.take() {
        log.finish(state.tick);
    }
}

/// Apply one checkpoint outcome: only a durable commit clears the captured
/// chunk/queue revisions and advances entity custody to its tick.
pub(super) fn acknowledge_checkpoint(state: &mut TickState, ack: CheckpointAck) {
    if let Some((tick, written, block_ticks, fluid_ticks, dimensions)) = ack {
        state.world.mark_flushed(&written);
        state.block_ticks.acknowledge_flushed(&block_ticks);
        state.fluid_ticks.acknowledge_flushed(&fluid_ticks);
        state.entities.checkpoint_acknowledged(tick);
        crate::dimensions::checkpoint::acknowledge(state, tick, dimensions);
    }
}

/// Queue a tick's immutable generated baselines, then its dependent WAL batch.
/// The journal owner commits each baseline on the native writer and waits for
/// it before it appends anything queued later, so no published WAL edit can
/// depend on a base that a crash or input change could regenerate. Even vanilla
/// inputs may change before restart. Only the journal owner waits; publication
/// still permits an unflushed tail. Returns the baseline handoff time, or the
/// halt reason when the journal thread is gone. A refused send says only that
/// the thread ended (it logs its own cause), so either send reports the same
/// journal halt, not the work item that happened to find it gone.
pub(super) fn send_tick_journal(
    state: &mut TickState,
    work_tx: &std::sync::mpsc::SyncSender<JournalWork>,
    budget: &JournalBudget,
    tick: u64,
    entries: Vec<JournalEntry>,
    plugin_ack: Option<std::sync::mpsc::Sender<u64>>,
) -> Result<std::time::Duration, &'static str> {
    const JOURNAL_HALT: &str = "journal failed: world halted (fail closed)";
    let generated_start = std::time::Instant::now();
    while let Some(batch) = crate::dimensions::checkpoint::take_generated(state) {
        work_tx
            .send(JournalWork::RequireGenerated {
                capture: Box::new(batch),
                wake: work_tx.clone(),
            })
            .map_err(|_| JOURNAL_HALT)?;
    }
    let generated_time = generated_start.elapsed();
    send_append(work_tx, budget, tick, entries, plugin_ack).map_err(|_| JOURNAL_HALT)?;
    Ok(generated_time)
}

/// Hand the remaining generated baselines to the journal thread, then stop it.
pub(super) fn send_shutdown(
    state: &mut TickState,
    work_tx: &std::sync::mpsc::SyncSender<JournalWork>,
) {
    tracing::info!(
        tick = state.tick,
        cause = "shutdown requested",
        "core tick owner stopping"
    );
    while let Some(batch) = crate::dimensions::checkpoint::take_generated(state) {
        if work_tx
            .send(JournalWork::RequireGenerated {
                capture: Box::new(batch),
                wake: work_tx.clone(),
            })
            .is_err()
        {
            tracing::error!(
                cause = "journal work channel closed",
                "core shutdown could not save generated chunks"
            );
            return;
        }
    }
    if work_tx.send(JournalWork::Shutdown).is_err() {
        tracing::error!(
            cause = "journal work channel closed",
            "core shutdown could not flush journal"
        );
    }
}

/// Halt on an authoritative journal or callback-delivery failure: send
/// `Disconnect{reason}` to every live connection through the reliable
/// outbox, then exit WITHOUT publishing, checkpointing, or accepting
/// further inputs. The broadcast sender drops with us, so subscribers see
/// `Closed` (no further TickOutput, ever). No retry worker.
pub(super) fn halt_authority(state: &mut TickState, channels: &TickChannels, reason: &str) {
    tracing::error!(
        tick = state.tick,
        cause = reason,
        "core tick owner halted (fail closed)"
    );
    let _ = channels.halt_reason.set(reason.to_owned());
    crate::battle::halt_exposure(state);
    let live: Vec<(ConnectionId, crate::boundary::PlayerChange)> = std::iter::once(&*state)
        .chain(state.other_dimensions.values().map(|world| &**world))
        .flat_map(|world| world.connections.all_connections())
        .map(|connection| {
            (
                connection,
                crate::boundary::PlayerChange::Disconnect {
                    reason: reason.to_owned(),
                },
            )
        })
        .collect();
    for (connection, change) in live {
        let world = if state.connections.entity_of(connection).is_some() {
            &mut *state
        } else {
            state
                .other_dimensions
                .values_mut()
                .find(|world| world.connections.entity_of(connection).is_some())
                .map(|world| &mut **world)
                .expect("collected native live connection")
        };
        world.push_player(connection, change);
    }
    let drains = std::mem::take(&mut state.outbox)
        .into_iter()
        .chain(
            state
                .other_dimensions
                .values_mut()
                .flat_map(|world| std::mem::take(&mut world.outbox)),
        )
        .collect();
    // One admission past the cap: the reason must reach every live connection
    // even when the cap is exactly what failed.
    if let Err(refusal) = channels.outbox.push_terminal(state.tick, drains) {
        tracing::error!(
            tick = state.tick,
            cause = %refusal.reason(),
            "terminal disconnect could not be queued"
        );
    }
    // Flush what the journal already accepted (file and directory fsync, no
    // checkpoint); the halt itself is still the fail-stop, not a commit.
    if channels.work_tx.send(JournalWork::Shutdown).is_err() {
        tracing::error!(
            tick = state.tick,
            "halt could not ask the journal for its final flush"
        );
    }
}
