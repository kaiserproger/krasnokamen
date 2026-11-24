//! Connection registry: handles, session locks and lookup.
//! Moved verbatim from `server.rs`.

use super::*;

/// A copy of every registered connection's handles.
pub(super) type HandleCopy = Vec<(ConnectionId, ConnectionHandles)>;

pub struct ConnectionRegistry {
    pub(super) next_id: AtomicU64,
    pub(super) inner: parking_lot::Mutex<RegistryInner>,
    pub(super) setup_fences_pending: AtomicBool,
    /// Abnormal closes by cause since start: the per-window error rollup the
    /// receipt cites (a socket drop, a reliable overflow, a failed write).
    /// Bare `&'static str` keys: the dashboard clones this map on request.
    close_causes: parking_lot::Mutex<std::collections::BTreeMap<&'static str, u64>>,
}

/// Session-ending causes that are not errors: the lifecycle's own removal
/// (`remove`), a client's ordinary EOF (`read-frame`), a writer stopped by a
/// delivery-policy close that was already counted (`pump-closed`), and a
/// core-ordered kick (`core-disconnect`).
const NORMAL_CLOSE_CAUSES: [&str; 4] = ["removed", "read-frame", "pump-closed", "core-disconnect"];

#[derive(Default)]
pub(super) struct RegistryInner {
    // A Closing connection retains its slot until native/admin cleanup runs.
    pub(super) admitted: HashSet<ConnectionId>,
    pub(super) joining: HashSet<ConnectionId>,
    /// Wakes the connection owner with the reason the registry dropped it.
    pub(super) closers: HashMap<ConnectionId, tokio::sync::oneshot::Sender<&'static str>>,
    pub(super) handles: HashMap<ConnectionId, ConnectionHandles>,
    /// Advances whenever `handles` gains or loses a connection. The pump keeps
    /// its routing view across ticks and copies it again only when this moves,
    /// so an unchanged tick takes no handle clone at all.
    pub(super) routing_generation: u64,
    pub(super) names: HashMap<ConnectionId, String>,
    pub(super) view_distances: HashMap<ConnectionId, i32>,
    /// Recorder connections ([`crate::recorder`]), fixed at join.
    pub(super) recorders: HashSet<ConnectionId>,
    /// One operator-requested world-queue fence per connection: (tick, nonce).
    pub(super) setup_world_fences: HashMap<ConnectionId, (u64, u64)>,
    /// Client locale last admitted to the core (`ClientInformation.language`);
    /// deduplicates Play updates and is cleared on disconnect.
    pub(super) locales: HashMap<ConnectionId, String>,
    /// Chat setting last admitted to the core (`ClientInformation.chat_visibility`);
    /// deduplicates Play updates and is cleared on disconnect.
    pub(super) chat_visibilities: HashMap<ConnectionId, mc_core::boundary::ChatVisibility>,
    /// Measured tab-list latency and profile id per connection, recorded on
    /// keepalive replies and broadcast batched by the pump; cleared on disconnect.
    pub(super) latencies: HashMap<ConnectionId, (uuid::Uuid, i32)>,
    pub(super) roster_dirty: bool,
    // Join state and its waiters share the queue registry's atomic removal.
    pub(super) pending_joins: HashMap<ConnectionId, JoinedSpawn>,
    pub(super) pending_reliable: HashMap<ConnectionId, Vec<mc_core::boundary::PlayerChange>>,
    pub(super) join_waiters: HashMap<ConnectionId, Vec<tokio::sync::oneshot::Sender<JoinedSpawn>>>,
    pub(super) disconnect_reasons: HashMap<ConnectionId, String>,
}

/// Queue handles for one Play connection.
#[derive(Clone)]
pub struct ConnectionHandles {
    /// Reliable queue sender (join tail, acks, menus, health, chat, syncpos, disconnect).
    pub reliable_tx: mpsc::Sender<bytes::Bytes>,
    /// World queue sender (pre-framed chunk/entity packets), bounded in
    /// frames and bytes.
    pub world_tx: WorldSender,
    pub(crate) chunk_batches: Arc<parking_lot::Mutex<crate::plan::chunk_batch::ChunkBatchState>>,
    /// Sealed under the registry lock from the latest native publication.
    pub(super) knockback_baseline: u64,
}

impl ConnectionHandles {
    /// `chunks_per_tick` caps chunk packets per publish for this connection
    /// (`[network] chunks_per_tick_per_player`).
    pub fn new(
        reliable_tx: mpsc::Sender<bytes::Bytes>,
        world_tx: WorldSender,
        chunks_per_tick: usize,
    ) -> Self {
        Self {
            reliable_tx,
            world_tx,
            chunk_batches: Arc::new(parking_lot::Mutex::new(
                crate::plan::chunk_batch::ChunkBatchState::with_cap(chunks_per_tick),
            )),
            knockback_baseline: 0,
        }
    }
}

impl ConnectionRegistry {
    /// Empty registry; ids start at 1.
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(FIRST_CONNECTION_ID),
            inner: parking_lot::Mutex::new(RegistryInner::default()),
            setup_fences_pending: AtomicBool::new(false),
            close_causes: parking_lot::Mutex::new(std::collections::BTreeMap::new()),
        }
    }

    /// Every abnormal close since start by cause, in cause order. The map is
    /// cloned whole: the dashboard reads it on request, never on the tick path.
    pub fn close_causes(&self) -> std::collections::BTreeMap<&'static str, u64> {
        self.close_causes.lock().clone()
    }

    /// Whether a cause enters the rollup at all: an abnormal cause does, a normal end does
    /// not. [`record_close_cause`](Self::record_close_cause) and the play loop's
    /// `writer_recorded` decision read this one rule, so exactly one of them counts a
    /// session's end.
    pub(crate) fn close_cause_counts(cause: &str) -> bool {
        !NORMAL_CLOSE_CAUSES.contains(&cause)
    }

    /// Count one session-ending cause, once, where it is known: a delivery-policy
    /// [`close`](Self::close), the play writer's own terminal stage, or the play
    /// loop's exit stage. A normal end (see [`NORMAL_CLOSE_CAUSES`]) is not an
    /// error and never enters the rollup.
    pub(crate) fn record_close_cause(&self, cause: &'static str) {
        if !Self::close_cause_counts(cause) {
            return;
        }
        *self.close_causes.lock().entry(cause).or_insert(0) += 1;
    }

    /// Allocate the next connection id.
    pub fn alloc_id(&self) -> ConnectionId {
        ConnectionId(self.next_id.fetch_add(1, Ordering::Relaxed))
    }

    /// Reserve a player slot atomically with the actual admission policy.
    /// Joining connections count too; the policy retains operator bypass.
    pub(super) fn admit(
        &self,
        id: ConnectionId,
        maximum: usize,
        check: impl FnOnce(bool) -> Option<serde_json::Value>,
    ) -> Option<serde_json::Value> {
        let mut inner = self.inner.lock();
        let refusal = check(inner.admitted.len() >= maximum);
        if refusal.is_none() {
            inner.admitted.insert(id);
            inner.joining.insert(id);
        }
        refusal
    }

    /// Whether every player slot is taken, counting joining connections like [`admit`](Self::admit).
    pub(super) fn is_full(&self, maximum: usize) -> bool {
        self.inner.lock().admitted.len() >= maximum
    }

    #[cfg(test)]
    pub(crate) fn joining_id(&self) -> ConnectionId {
        let id = self.alloc_id();
        assert!(self.admit(id, usize::MAX, |_| None).is_none());
        id
    }

    /// Register a connection's queue handles and take the reliable changes the
    /// pump stashed before registration, in one critical section with
    /// [`handles_or_stash`][Self::handles_or_stash]: every drained change is
    /// either returned here for replay or routed by the pump through the
    /// registered handles, never stranded in between.
    /// The count supplier runs inside the same critical section, so the pump
    /// cannot observe handles without their immutable publication baseline.
    pub fn register(
        &self,
        id: ConnectionId,
        mut handles: ConnectionHandles,
        published_knockback_count: impl FnOnce() -> u64,
    ) -> anyhow::Result<Vec<mc_core::boundary::PlayerChange>> {
        let mut inner = self.inner.lock();
        anyhow::ensure!(
            inner.joining.remove(&id),
            "connection admission already closed"
        );
        handles.knockback_baseline = published_knockback_count();
        inner.handles.insert(id, handles);
        inner.routing_generation += 1;
        Ok(inner.pending_reliable.remove(&id).unwrap_or_default())
    }

    /// The chat setting last admitted for this connection; Full while none is known.
    pub(crate) fn chat_visibility(&self, id: ConnectionId) -> mc_core::boundary::ChatVisibility {
        self.inner
            .lock()
            .chat_visibilities
            .get(&id)
            .copied()
            .unwrap_or(mc_core::boundary::ChatVisibility::Full)
    }

    /// Read actual mpsc occupied-or-reserved slots only on telemetry requests.
    /// Senders cannot reveal queued versus reserved slots, or byte totals.
    pub fn load_queues(&self) -> Option<LoadQueues> {
        let captured_unix_ns = unix_ns();
        let inner = self.inner.lock();
        let mut reliable = LoadQueueGauge::default();
        let mut world = LoadQueueGauge::default();
        for handles in inner.handles.values() {
            for (gauge, capacity, free) in [
                (
                    &mut reliable,
                    handles.reliable_tx.max_capacity(),
                    handles.reliable_tx.capacity(),
                ),
                (
                    &mut world,
                    handles.world_tx.max_capacity(),
                    handles.world_tx.capacity(),
                ),
            ] {
                let occupied = capacity.saturating_sub(free);
                gauge.capacity_slots += capacity;
                gauge.queued_or_reserved_slots += occupied;
                gauge.max_viewer_queued_or_reserved_slots =
                    gauge.max_viewer_queued_or_reserved_slots.max(occupied);
            }
        }
        Some(LoadQueues {
            captured_unix_ns,
            completed_unix_ns: unix_ns(),
            viewers: inner.handles.len(),
            capabilities: LoadQueueCapabilities {
                queued_or_reserved_slots: true,
                queued_slots: false,
                reserved_slots: false,
                queued_bytes: false,
                reserved_bytes: false,
            },
            reliable,
            world,
        })
    }

    /// Stash a Joined the pump drained before the connection registered.
    /// Also fires any registered join waiters for this connection.
    pub(super) fn stash_join(&self, id: ConnectionId, joined: JoinedSpawn) {
        let senders = {
            let mut inner = self.inner.lock();
            if !inner.joining.contains(&id) && !inner.handles.contains_key(&id) {
                return;
            }
            inner.pending_joins.insert(id, joined);
            inner.join_waiters.remove(&id).unwrap_or_default()
        };
        for sender in senders {
            let _ = sender.send(joined);
        }
    }

    /// Take a stashed Joined for a joining connection. Non-removing: the
    /// stashed row doubles as the authoritative entity id for later viewer
    /// construction, so joins stay correct after the first take.
    pub fn take_join(&self, id: ConnectionId) -> Option<JoinedSpawn> {
        self.inner.lock().pending_joins.get(&id).copied()
    }

    /// Pump side of the join race: the handles of a registered connection,
    /// or else stash the drained `changes` for replay at
    /// [`register`][Self::register], atomically with the handle check. Their
    /// `Joined` is published for [`wait_join`][Self::wait_join] only after
    /// that, and the connection registers only after its join wait, so the
    /// stash is always in place before registration takes it.
    pub fn handles_or_stash(
        &self,
        id: ConnectionId,
        changes: &mut Vec<mc_core::boundary::PlayerChange>,
    ) -> Option<ConnectionHandles> {
        let joined = changes.iter().find_map(|change| match change {
            mc_core::boundary::PlayerChange::Joined {
                entity,
                position,
                yaw,
                pitch,
                game_mode,
                dimension,
            } => Some(JoinedSpawn {
                entity: *entity,
                position: *position,
                rotation: [*yaw, *pitch],
                game_mode: *game_mode,
                dimension: *dimension,
            }),
            _ => None,
        });
        {
            let mut inner = self.inner.lock();
            if let Some(handles) = inner.handles.get(&id) {
                return Some(handles.clone());
            }
            if !inner.joining.contains(&id) {
                changes.clear();
                return None;
            }
            inner
                .pending_reliable
                .entry(id)
                .or_default()
                .append(changes);
        }
        if let Some(joined) = joined {
            self.stash_join(id, joined);
        }
        None
    }

    /// Stash a core-ordered disconnect reason for a connection. The play
    /// loop takes it after flushing the kick packet and logs peer+reason.
    pub fn stash_disconnect_reason(&self, id: ConnectionId, reason: String) {
        let mut inner = self.inner.lock();
        if inner.joining.contains(&id) || inner.handles.contains_key(&id) {
            inner.disconnect_reasons.insert(id, reason);
        }
    }

    /// The core refused this connection's Join (no `Joined` will come, e.g. a duplicate
    /// login): fail its join waiters now. The connection owner then reads the stashed
    /// disconnect reason and sends it, instead of waiting out the join deadline.
    pub fn refuse_join(&self, id: ConnectionId) {
        self.inner.lock().join_waiters.remove(&id);
    }

    /// Take a stashed core-ordered disconnect reason, if the pump routed one.
    pub fn take_disconnect_reason(&self, id: ConnectionId) -> Option<String> {
        self.inner.lock().disconnect_reasons.remove(&id)
    }

    /// Register interest in this connection's Joined. The pump fires the
    /// receiver when it routes the Joined (via [`stash_join`][Self::stash_join]).
    /// Callers must check [`take_join`][Self::take_join] first (fast path
    /// when the Joined already arrived), then await the receiver.
    pub fn wait_join(&self, id: ConnectionId) -> tokio::sync::oneshot::Receiver<JoinedSpawn> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let mut inner = self.inner.lock();
        if let Some(joined) = inner.pending_joins.get(&id) {
            let _ = sender.send(*joined);
        } else if inner.joining.contains(&id) {
            inner.join_waiters.entry(id).or_default().push(sender);
        }
        receiver
    }

    /// Stop delivery and wake the connection owner, including a blocked writer.
    /// Admission is released only by the lifecycle owner's final cleanup.
    /// Drops queue senders, the stashed join, stashed pre-registration
    /// reliable changes, any join waiters (their receivers fail closed), and
    /// the stashed disconnect reason.
    pub fn remove(&self, id: ConnectionId) {
        self.close(id, "removed");
    }

    /// [`Self::remove`] naming why: the connection owner logs `cause`, and an
    /// abnormal cause counts once in [`close_causes`](Self::close_causes). A
    /// normal end (`removed`) counts nothing.
    ///
    /// The cause counts only when this call takes the session's live closer:
    /// one bare `close` per id is a session end, while a second close (or a
    /// close of an id that never had a live session) is bookkeeping after the
    /// end was already counted somewhere else.
    pub fn close(&self, id: ConnectionId, cause: &'static str) {
        crate::vanilla_items::unmark(id);
        let closer = {
            let mut inner = self.inner.lock();
            inner.joining.remove(&id);
            // Only a connection that actually held handles moves the routing
            // view: a second close, a refused login or a Status ping must not
            // make the pump copy every remaining handle again.
            if inner.handles.remove(&id).is_some() {
                inner.routing_generation += 1;
            }
            if inner.names.remove(&id).is_some() {
                inner.roster_dirty = true;
            }
            inner.pending_joins.remove(&id);
            inner.pending_reliable.remove(&id);
            inner.join_waiters.remove(&id);
            inner.disconnect_reasons.remove(&id);
            inner.view_distances.remove(&id);
            inner.recorders.remove(&id);
            inner.setup_world_fences.remove(&id);
            if inner.setup_world_fences.is_empty() {
                self.setup_fences_pending.store(false, Ordering::Release);
            }
            inner.locales.remove(&id);
            inner.chat_visibilities.remove(&id);
            inner.latencies.remove(&id);
            inner.closers.remove(&id)
        };
        if let Some(closer) = closer {
            self.record_close_cause(cause);
            let _ = closer.send(cause);
        }
    }

    /// The lifecycle's final cleanup: drop the entry and release admission. The
    /// session's own end was already recorded where it was known, so this removal
    /// counts nothing.
    pub(super) fn finish(&self, id: ConnectionId) {
        self.remove(id);
        self.inner.lock().admitted.remove(&id);
    }

    /// Look up one connection's handles.
    pub fn get(&self, id: ConnectionId) -> Option<ConnectionHandles> {
        self.inner.lock().handles.get(&id).cloned()
    }

    /// Snapshot all handles for pump routing.
    pub fn snapshot(&self) -> Vec<(ConnectionId, ConnectionHandles)> {
        self.inner
            .lock()
            .handles
            .iter()
            .map(|(id, handles)| (*id, handles.clone()))
            .collect()
    }

    /// The pump's routing view for one tick. Handles are returned only when a
    /// connection registered or left since `generation` last observed a copy,
    /// and the copy and the recorded generation come from one critical
    /// section: an unchanged tick takes the mutex, compares one counter and
    /// clones no handle. Roster work follows [`set_name`](Self::set_name).
    pub(super) fn routing_snapshot(
        &self,
        generation: &mut u64,
    ) -> (Option<HandleCopy>, Option<Vec<String>>) {
        let (handles, names) = {
            let mut inner = self.inner.lock();
            let handles = if inner.routing_generation == *generation {
                None
            } else {
                *generation = inner.routing_generation;
                Some(
                    inner
                        .handles
                        .iter()
                        .map(|(id, handles)| (*id, handles.clone()))
                        .collect(),
                )
            };
            // Owned copy under the same guard; the sort runs after it drops,
            // so a busy roster never extends the critical section.
            let names = std::mem::take(&mut inner.roster_dirty)
                .then(|| inner.names.values().cloned().collect::<Vec<_>>());
            (handles, names)
        };
        let names = names.map(|mut names| {
            names.sort();
            names
        });
        (handles, names)
    }

    /// Every handle copy plus the pending roster, for tests that build a fresh
    /// routing view for one tick. Production keeps the view across ticks and
    /// uses [`routing_snapshot`](Self::routing_snapshot).
    #[cfg(test)]
    pub(crate) fn routing_view(
        &self,
    ) -> (
        HashMap<ConnectionId, ConnectionHandles>,
        Option<Vec<String>>,
    ) {
        let mut generation = 0;
        let (handles, roster) = self.routing_snapshot(&mut generation);
        (handles.unwrap_or_default().into_iter().collect(), roster)
    }

    /// Set a player name for the dashboard roster.
    pub fn set_name(&self, id: ConnectionId, name: String) {
        let mut inner = self.inner.lock();
        if inner.names.get(&id) != Some(&name) {
            inner.names.insert(id, name);
            inner.roster_dirty = true;
        }
    }

    /// Record a keepalive-measured latency for the pump's batched broadcast.
    pub fn set_latency(&self, id: ConnectionId, profile: uuid::Uuid, latency: i32) {
        self.inner.lock().latencies.insert(id, (profile, latency));
    }

    /// Every connection's last measured latency, sorted by profile id.
    pub(super) fn latencies(&self) -> Vec<(uuid::Uuid, i32)> {
        let mut latencies: Vec<_> = self.inner.lock().latencies.values().copied().collect();
        latencies.sort_by_key(|(profile, _)| *profile);
        latencies
    }

    /// Sorted roster names for the dashboard.
    pub fn roster(&self) -> Vec<String> {
        let mut names: Vec<_> = self.inner.lock().names.values().cloned().collect();
        names.sort();
        names
    }

    /// Current connection count.
    pub fn len(&self) -> usize {
        self.inner.lock().handles.len()
    }

    /// Whether no connections are registered.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for ConnectionRegistry {
    fn default() -> Self {
        Self::new()
    }
}
