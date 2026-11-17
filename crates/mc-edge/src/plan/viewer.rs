//! Per-viewer cached world view and delivered entity baselines.
//!
//! What one connection already holds, so planning emits only deltas.
use super::*;

/// One delivered entity baseline; retaining the appearance Arc makes pointer
/// comparison a sound change detector until the next successful delivery.
#[derive(Debug)]
#[cfg_attr(test, derive(Default))]
pub(crate) struct TrackedEntity {
    pub(crate) type_id: u32,
    pub(super) baseline: MoveBaseline,
    pub(super) head_yaw: u8,
    pub(super) appearance: Arc<EntityAppearance>,
    pub(super) seen: u64,
    /// Frames of this entity were lost and it cannot be removed (it carries
    /// the viewer): the next plan sends its whole state again, over the
    /// client's copy.
    pub(super) resend: bool,
    /// The viewer's entity round of the last delivery (spawn or update):
    /// how long a pending update has waited, for priority under credit.
    pub(super) updated: u64,
}

impl TrackedEntity {
    pub(super) fn new(entity: &EntityView, seen: u64, round: u64) -> Self {
        Self {
            type_id: entity.type_id,
            baseline: (entity.position, entity.yaw, entity.pitch, entity.head_yaw),
            head_yaw: pack_degrees(entity.head_yaw),
            appearance: Arc::clone(&entity.appearance),
            seen,
            resend: false,
            updated: round,
        }
    }

    /// Stand-in for an entity the client may still hold though the viewer
    /// forgot it: its identity matches no entity, so the plan removes it.
    pub(super) fn undelivered() -> Self {
        Self {
            type_id: UNDELIVERED_TYPE,
            baseline: ([0.0; 3], 0.0, 0.0, 0.0),
            head_yaw: 0,
            appearance: Arc::default(),
            seen: 0,
            resend: false,
            updated: 0,
        }
    }
}

/// Point the passengers of tracked `vehicle` at it, replacing the entries of
/// its `old` passengers. Every change to a tracked appearance goes through
/// here, so the map holds exactly the passengers of the vehicles a viewer
/// tracks ([`ViewerState::vehicles`]).
pub(super) fn relink_passengers(
    vehicles: &mut HashMap<EntityId, EntityId>,
    vehicle: EntityId,
    old: &[EntityId],
    new: &[EntityId],
) {
    for passenger in old {
        if vehicles.get(passenger) == Some(&vehicle) {
            vehicles.remove(passenger);
        }
    }
    for &passenger in new {
        vehicles.insert(passenger, vehicle);
    }
}

/// Per-viewer cached world view. Tracks what this connection already has so
/// planning emits only deltas. Teleport ids are allocated by the core.
#[derive(Debug)]
pub struct ViewerState {
    pub connection: ConnectionId,
    pub entity_id: EntityId,
    /// Camera last planned on the ordered world FIFO; None means delivery debt.
    pub(super) camera: Option<EntityId>,
    pub position: [f64; 3],
    pub dimension: mc_core::dimensions::Dimension,
    /// Negotiated connection radius; None uses the caller's server limit.
    pub(crate) view_distance: Option<i32>,
    pub(crate) chunk_batches: Arc<parking_lot::Mutex<chunk_batch::ChunkBatchState>>,
    /// Registry indices resolved by the edge from its exact vanilla data.
    pub dimension_type_ids: [i32; 3],
    pub advancements: Arc<crate::advancements::AdvancementPackets>,
    pub recipe_catalog: Option<Arc<crate::recipes::RecipeCatalog>>,
    /// Chunks the client holds, with the content revision it holds.
    pub loaded_chunks: HashMap<(i32, i32), u64>,
    /// Full repair debt for chunks still held by the client. Kept separately
    /// from initial admission so live edits cannot starve an unseen window.
    pub(super) chunk_repairs: HashSet<(i32, i32)>,
    /// Chunks with public block-entity changes since the last snapshot plan;
    /// they explain content revisions just like block deltas.
    pub(super) block_entity_updates: HashSet<(i32, i32)>,
    /// Last delivered baked section layers; shared nibble allocations are COW.
    pub(super) light_baseline: crate::light::LightBaseline,
    pub(super) weather_baseline: crate::light::WeatherBaseline,
    pub(super) boss_baseline: crate::boss_bars::BossBaseline,
    pub(super) border_baseline: Option<mc_core::command_border::Border>,
    /// Includes the self appearance baseline, but self is never spawned/removed.
    pub(crate) tracked_entities: HashMap<EntityId, TrackedEntity>,
    /// Passenger -> vehicle for the passengers of the vehicles in
    /// `tracked_entities`, as last sent; a passenger keeps its entry while it
    /// is out of view, so its pairing can mount it when it comes back.
    pub(super) vehicles: HashMap<EntityId, EntityId>,
    /// Player-only identity baselines, changed by reliable roster events, not
    /// per-entity tick work. Authentication may replace a provisional UUID.
    pub(crate) player_profiles: HashMap<EntityId, uuid::Uuid>,
    /// Per entity-plan stamp, not snapshot tick: repeated snapshots are normal.
    pub(super) plan_stamp: u64,
    /// Last evaluated snapshot tick, for reconciling a due phase skipped by the pump.
    pub(super) entity_tick: Option<u64>,
    pub(super) entity_interest: entity_interest::EntityInterest,
    /// Positions whose block update is still owed (no credit when the
    /// change happened): batched by section later with the newest state.
    pub(super) pending_blocks: HashSet<mc_world::chunk::BlockPos>,
    /// Public block entity projections owed after the corresponding block state.
    pub(super) pending_block_entities: HashSet<mc_world::chunk::BlockPos>,
    pub center_sent: Option<(i32, i32)>,
    pub(super) radius_sent: Option<i32>,
    /// The pump skipped outputs since the last plan: their block changes
    /// never reached this viewer, so every chunk whose revision moved on is
    /// sent again.
    pub skipped: bool,
    /// Free outbound world-queue frame slots, refreshed by the pump before
    /// each publish: the plan emits at most this many best-effort frames.
    pub world_credit: usize,
    /// Free outbound world-queue bytes, refreshed with `world_credit`
    /// ([`WorldCredit`] explains the soft byte bound).
    pub world_bytes: usize,
    /// A recorder connection ([`crate::recorder`]): every entity type is
    /// tracked to the full view distance and each publish is marked.
    pub recorder: bool,
    /// Entity removals, spawns and updates the last plan left without world
    /// credit ([`crate::recorder::RecorderTick`]); reset before every publish.
    pub deferred_updates: usize,
    /// Entity plans so far: the clock of [`TrackedEntity::updated`].
    pub(super) entity_round: u64,
    /// Entities whose frames the last plan queued; the pump reports a frame
    /// of that plan that the world queue refused ([`Self::world_frame_unsent`]).
    pub(super) world_entities: Vec<EntityId>,
    /// Entities whose removal the last plan queued.
    pub(super) world_removed: Vec<EntityId>,
    /// A same-dimension respawn is pending its forget pass: the vanilla
    /// client keeps its level across it (`ClientPacketListener.handleRespawn`
    /// builds a new `ClientLevel` only on a dimension change), so, like the
    /// vanilla server, everything held is removed/forgotten on the world
    pub(super) forget_all: bool,
    /// Connection-local settlement label image (virtual negative ids).
    pub(crate) labels: crate::settlement_labels::LabelBaseline,
    /// Presentation-only stair seats keyed by their tracked villager.
    pub(super) seats: HashMap<EntityId, (i32, mc_core::boundary::BlockPos)>,
    /// Snapshot dimension the label image was built from; a mismatch means
    /// the client's fresh level holds none of these virtual entities.
    pub(crate) label_dimension: Option<mc_core::dimensions::Dimension>,
    /// Battle-faction teams this client holds ([`crate::faction_teams`]);
    /// respawns keep them.
    pub(crate) teams: crate::faction_teams::TeamBaseline,
    /// Last sent `(tick, day_time, rate)`; None forces an authoritative sync.
    pub(super) clock_baseline: Option<(u64, i64, f32)>,
    pub(super) scratch: PlanScratch,
    pub(super) window: ViewWindow,
    /// What the chunk state was last proven against; `None` forces a full walk.
    pub(super) chunk_sync: Option<ChunkSync>,
    #[cfg(test)]
    pub(crate) chunk_stats: chunk_delta::ChunkPlanStats,
}

impl ViewerState {
    #[must_use]
    pub fn new(connection: ConnectionId, entity_id: EntityId, position: [f64; 3]) -> Self {
        Self {
            connection,
            entity_id,
            camera: Some(entity_id),
            position,
            dimension: mc_core::dimensions::Dimension::Overworld,
            view_distance: None,
            chunk_batches: Arc::default(),
            dimension_type_ids: [0; 3],
            advancements: Arc::default(),
            recipe_catalog: None,
            loaded_chunks: HashMap::default(),
            chunk_repairs: HashSet::default(),
            block_entity_updates: HashSet::default(),
            light_baseline: crate::light::LightBaseline::default(),
            weather_baseline: crate::light::WeatherBaseline::default(),
            boss_baseline: crate::boss_bars::BossBaseline::default(),
            border_baseline: None,
            tracked_entities: HashMap::default(),
            vehicles: HashMap::default(),
            player_profiles: HashMap::default(),
            plan_stamp: 0,
            entity_tick: None,
            entity_interest: entity_interest::EntityInterest::default(),
            pending_blocks: HashSet::default(),
            pending_block_entities: HashSet::default(),
            center_sent: None,
            radius_sent: None,
            skipped: false,
            world_credit: usize::MAX,
            world_bytes: usize::MAX,
            recorder: false,
            deferred_updates: 0,
            entity_round: 0,
            world_entities: Vec::new(),
            world_removed: Vec::new(),
            forget_all: false,
            labels: crate::settlement_labels::LabelBaseline::default(),
            seats: HashMap::default(),
            label_dimension: None,
            teams: crate::faction_teams::TeamBaseline::default(),
            clock_baseline: None,
            scratch: PlanScratch::default(),
            window: ViewWindow::default(),
            chunk_sync: None,
            #[cfg(test)]
            chunk_stats: chunk_delta::ChunkPlanStats::default(),
        }
    }

    /// The client never received this chunk's packet: plan it again, and let
    /// the next plan walk the whole window.
    pub(crate) fn chunk_unsent(&mut self, pos: (i32, i32)) {
        self.loaded_chunks.remove(&pos);
        self.chunk_sync = None;
    }

    /// Reconcile the complete held geometry after a planned world frame was not queued.
    pub(crate) fn geometry_unsent(&mut self) {
        self.chunk_repairs
            .extend(self.loaded_chunks.keys().copied());
        self.chunk_sync = None;
    }

    /// A planned world frame was not queued: the world queue filled after
    /// the plan read its credit, because another producer (an LOD tile) took
    /// a slot. The viewer recorded everything the last plan emitted as
    /// delivered, so held geometry and the client state of every entity that
    /// plan touched are repaired from the next plan.
    pub(crate) fn world_frame_unsent(&mut self) {
        self.camera = None;
        self.geometry_unsent();
        for id in std::mem::take(&mut self.world_removed) {
            // The client may still hold it: track it again to have it removed.
            self.tracked_entities
                .entry(id)
                .or_insert_with(TrackedEntity::undelivered);
            self.entity_interest.respawn_tracked(id);
        }
        for id in std::mem::take(&mut self.world_entities) {
            self.entity_unsent(id);
        }
    }

    /// The client may miss any frame planned for `id`. The own entity forgets
    /// its baseline and resends its data. The viewer's own vehicle keeps its
    /// place (removing it dismounts the player) and resends its state.
    /// Another tracked entity no longer matches its recorded identity, so the
    /// next plan removes and respawns it, with its passengers and leash; one
    /// that left view meanwhile is removed as usual.
    pub(super) fn entity_unsent(&mut self, id: EntityId) {
        if id == self.entity_id {
            self.untrack(id);
        } else if self.vehicles.get(&self.entity_id) == Some(&id) {
            if let Some(tracked) = self.tracked_entities.get_mut(&id) {
                tracked.resend = true;
            }
            self.entity_interest.revisit(id);
        } else if let Some(tracked) = self.tracked_entities.get_mut(&id) {
            tracked.type_id = UNDELIVERED_TYPE;
            self.entity_interest.respawn_tracked(id);
        }
    }

    /// All geometry already planned for this viewer precedes a world-queue fence.
    pub(crate) fn geometry_drained(&self, server_distance: i32, chunks: &[SnapshotChunk]) -> bool {
        let radius = self
            .view_distance
            .unwrap_or(server_distance)
            .min(server_distance);
        let center = (
            floor_i32(self.position[0]).div_euclid(16),
            floor_i32(self.position[2]).div_euclid(16),
        );
        !self.skipped
            && !self.forget_all
            && self.center_sent == Some(center)
            && self.pending_blocks.is_empty()
            && self.chunk_repairs.is_empty()
            && (center.0 - radius..=center.0 + radius).all(|x| {
                (center.1 - radius..=center.1 + radius).all(|z| {
                    self.loaded_chunks.get(&(x, z)).is_some_and(|held| {
                        find_chunk(chunks, (x, z)).is_some_and(|chunk| chunk.revision == *held)
                    })
                })
            })
    }

    /// Record `entity` as delivered with its current appearance.
    pub(super) fn track(&mut self, entity: &EntityView) {
        let old = self.tracked_entities.insert(
            entity.id,
            TrackedEntity::new(entity, self.plan_stamp, self.entity_round),
        );
        relink_passengers(
            &mut self.vehicles,
            entity.id,
            old.as_ref()
                .map_or(&[][..], |old| old.appearance.passengers.as_slice()),
            &entity.appearance.passengers,
        );
    }

    /// Forget a delivered entity and the passenger entries it owned; false
    /// when it was not tracked.
    pub(super) fn untrack(&mut self, id: EntityId) -> bool {
        let Some(old) = self.tracked_entities.remove(&id) else {
            return false;
        };
        relink_passengers(&mut self.vehicles, id, &old.appearance.passengers, &[]);
        true
    }

    /// The client holds no entity of this view any more.
    pub(super) fn untrack_all(&mut self) {
        self.tracked_entities.clear();
        self.vehicles.clear();
    }
}

/// Remove every held entity and forget every held chunk, within credit.
/// Returns true once the client holds nothing; the cached view is then
/// empty and the next plan sends the whole view (own entity data included).
pub(super) fn forget_everything(
    viewer: &mut ViewerState,
    out: &mut Vec<PlannedPacket>,
    credit: &mut WorldCredit,
    scratch: &mut PlanScratch,
) -> bool {
    if !camera::send(viewer, viewer.entity_id, out, credit) {
        return false;
    }
    if !clear_seats(viewer, out, credit) {
        return false;
    }
    let conn = viewer.connection;
    scratch.removed.clear();
    scratch.removed.extend(
        viewer
            .tracked_entities
            .keys()
            .filter(|id| **id != viewer.entity_id)
            .map(|id| id.0),
    );
    scratch.removed.sort_unstable();
    if !scratch.removed.is_empty() {
        let packet = RemoveEntities {
            entity_ids: std::mem::take(&mut scratch.removed),
        };
        let sent = try_push(out, credit, |out| push_best_effort(out, conn, &packet));
        scratch.removed = packet.entity_ids;
        if !sent {
            return false;
        }
    }
    viewer
        .world_removed
        .extend(scratch.removed.iter().map(|&id| EntityId(id)));
    viewer.untrack_all();
    viewer.entity_tick = None;
    viewer.entity_interest = entity_interest::EntityInterest::default();
    scratch.chunks.clear();
    scratch.chunks.extend(viewer.loaded_chunks.keys().copied());
    for &chunk in &scratch.chunks {
        if !try_push(out, credit, |out| {
            push_best_effort(
                out,
                conn,
                &ForgetLevelChunk {
                    chunk_x: chunk.0,
                    chunk_z: chunk.1,
                },
            );
        }) {
            return false;
        }
        viewer.loaded_chunks.remove(&chunk);
        viewer.chunk_repairs.remove(&chunk);
        viewer.light_baseline.forget(chunk);
    }
    viewer.pending_blocks.clear();
    viewer.pending_block_entities.clear();
    viewer.block_entity_updates.clear();
    viewer.center_sent = None;
    viewer.forget_all = false;
    crate::settlement_labels::forget_labels(viewer, out);
    true
}

impl ViewerState {
    pub(crate) fn previous_entities(&self) -> Option<&mc_core::boundary::EntityViews> {
        self.entity_interest.previous()
    }
}
