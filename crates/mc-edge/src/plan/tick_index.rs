//! One change index per dimension/publish, shared by every viewer.
use super::*;

#[derive(Default)]
struct SubjectEvents {
    chunk: Option<(i32, i32)>,
    events: Vec<usize>,
}

/// Publish-wide facts: neither membership lookup nor type inspection needs
/// repeating for every viewer tracking the same moving mob.
#[derive(Clone, Copy, Debug)]
pub(super) struct EntityDelta {
    pub(super) id: EntityId,
    pub(super) chunk: Option<(i32, i32)>,
    pub(super) membership_same: bool,
    pub(super) appearance_changed: bool,
    pub(super) motion_changed: bool,
    pub(super) range: i32,
}

/// Test-only: how many spectator indexes were built (one per publish).
#[cfg(test)]
pub(super) static SPECTATOR_INDEX_BUILDS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[derive(Default)]
pub(crate) struct TickPlanIndex {
    pub(super) blocks: HashMap<(i32, i32), Vec<usize>>,
    pub(super) explosions: HashMap<(i32, i32), Vec<usize>>,
    /// Recipient impulses are independent of the 64-block visual index.
    pub(super) knockback: HashMap<EntityId, (usize, [f64; 3])>,
    /// Roster entities whose game mode is spectator this publish: the answer
    /// `broadcast_to` needs, built once instead of scanning the roster for
    /// every visible player of every viewer.
    pub(super) spectators: std::collections::HashSet<EntityId>,
    pub(super) events: HashMap<(i32, i32), Vec<usize>>,
    pub(crate) entity_base: Option<u64>,
    /// Anti-xray: hideable ores a block change this tick just exposed, keyed by
    /// the ore's own chunk (the change may be in a neighbouring chunk).
    pub(super) reveals: HashMap<(i32, i32), Vec<mc_world::chunk::BlockPos>>,
    entity_chunks: HashMap<(i32, i32), Vec<EntityDelta>>,
    pub(super) moves: super::shared_moves::SharedMoves,
    // Events can refer to an entity already removed from the snapshot, or to
    // a known entity that crossed out of the viewer square before its event.
    event_subjects: HashMap<EntityId, SubjectEvents>,
    chunk_delta: Option<Arc<ChunkDelta>>,
    /// Faction memberships this continuous step changed (`None`: no step).
    factions: Option<crate::faction_teams::FactionDelta>,
    /// History the pump's chain holds behind this publish, for rescan reasons.
    chain: ChainInfo,
}

/// One drained tick output's entity history for one dimension: the revision the
/// core published and the ids whose wire image it changed. Kept instead of the
/// views, so a link pins no copy-on-write page, and merged into a net delta only
/// for the viewer base that actually needs it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct EntityLink {
    base: u64,
    revision: u64,
}

/// The entity steps of the outputs a pump drained, per dimension, oldest first:
/// what a viewer whose own publish was skipped needs to stay contiguous.
///
/// A link carries no views and no deltas; the changed ids of each step are held
/// beside it and the net delta against a viewer's delivered base is computed
/// once, at bridge time ([`TickPlanIndex::with_chain`]). Bounded by the tick
/// broadcast's capacity, and cut at any `Lagged`, so nothing is bridged across
/// outputs the pump never received.
#[derive(Default)]
pub(crate) struct PublicationChain {
    /// `(link, ids the step changed)`, oldest first, contiguous.
    links: std::collections::VecDeque<(EntityLink, Arc<Vec<EntityId>>)>,
    bound: usize,
    /// Newest revision a broadcast `Lagged` dropped before it could be drained.
    lost_through: Option<u64>,
}

impl PublicationChain {
    pub(crate) fn new(bound: usize) -> Self {
        Self {
            bound: bound.max(1),
            ..Self::default()
        }
    }

    /// Record one drained output's entity views, in drain order. A repeated
    /// revision (the core only advances it when the wire image changes) is a
    /// no-op: it must not break a chain the pump did receive.
    pub(crate) fn record(&mut self, entities: &mc_core::boundary::EntityViews) {
        let (base, revision) = (entities.base_revision(), entities.revision());
        if revision == base || entities.changed().is_empty() {
            return;
        }
        let Some(&(last, _)) = self.links.back() else {
            self.links.push_back((
                EntityLink { base, revision },
                Arc::clone(entities.changed_arc()),
            ));
            return;
        };
        if last.revision == revision {
            return;
        }
        if last.revision != base {
            // A step the pump never drained: nothing older can reach this
            // publish any more.
            self.links.clear();
        }
        self.links.push_back((
            EntityLink { base, revision },
            Arc::clone(entities.changed_arc()),
        ));
        while self.links.len() > self.bound {
            self.links.pop_front();
        }
    }

    /// A broadcast `Lagged` dropped outputs the pump can never see again.
    pub(crate) fn cut(&mut self) {
        let lost = self.links.back().map(|(link, _)| link.revision);
        self.lost_through = match (self.lost_through, lost) {
            (Some(old), Some(lost)) => Some(old.max(lost)),
            (old, lost) => old.or(lost),
        };
        self.links.clear();
    }

    /// The contiguous steps from `base` to `revision`, oldest first; `None`
    /// when any step between them is missing (tests and diagnostics).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn steps(&self, base: u64, revision: u64) -> Option<Vec<EntityLink>> {
        let mut steps = Vec::new();
        let mut want = base;
        for (link, _) in &self.links {
            if steps.is_empty() && link.base != base {
                continue;
            }
            if link.base != want {
                return None;
            }
            steps.push(*link);
            want = link.revision;
            if want == revision {
                return Some(steps);
            }
        }
        None
    }

    /// Whether every step from `base` to `revision` is held, without
    /// materializing the ids: the cheap half of [`Self::changed_between`],
    /// used to pick the base before paying for the union.
    pub(crate) fn covers(&self, base: u64, revision: u64) -> bool {
        let mut want = base;
        let mut seen = false;
        for (link, _) in &self.links {
            if !seen {
                if link.base != base {
                    continue;
                }
                seen = true;
            }
            if link.base != want {
                return false;
            }
            want = link.revision;
            if want == revision {
                return true;
            }
        }
        false
    }

    /// The changed ids of every step between `base` and `revision`, ascending
    /// and deduplicated: the rows a bridge has to re-examine.
    pub(crate) fn changed_between(&self, base: u64, revision: u64) -> Option<Vec<EntityId>> {
        let mut want = base;
        let mut seen = false;
        let mut changed: Vec<EntityId> = Vec::new();
        for (link, ids) in &self.links {
            if !seen {
                if link.base != base {
                    continue;
                }
                seen = true;
            }
            if link.base != want {
                return None;
            }
            changed.extend(ids.iter().copied());
            want = link.revision;
            if want == revision {
                changed.sort_unstable();
                changed.dedup();
                return Some(changed);
            }
        }
        None
    }

    pub(crate) fn info(&self) -> ChainInfo {
        ChainInfo {
            lost_through: self.lost_through,
        }
    }

    /// Drained steps the chain holds (tests and diagnostics).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn len(&self) -> usize {
        self.links.len()
    }
}

/// The drained outputs of every dimension: each dimension has its own bounded
/// chain, because a publish carries a root snapshot plus per-dimension
/// projections and a viewer only ever bridges inside its own dimension.
#[derive(Default)]
pub(crate) struct PublicationHistory {
    root: PublicationChain,
    root_dimension: Option<mc_core::dimensions::Dimension>,
    dimensions: HashMap<mc_core::dimensions::Dimension, PublicationChain>,
    bound: usize,
}

impl PublicationHistory {
    pub(crate) fn new(bound: usize) -> Self {
        let bound = bound.max(1);
        Self {
            root: PublicationChain::new(bound),
            bound,
            ..Self::default()
        }
    }

    /// Record one drained output: the root snapshot and every projection it
    /// carries.
    pub(crate) fn record(&mut self, output: &TickOutput) {
        self.root_dimension = Some(output.snapshot.dimension);
        self.root.record(&output.snapshot.entities);
        for (dimension, projection) in &output.snapshot.dimensions {
            self.dimensions
                .entry(*dimension)
                .or_insert_with(|| PublicationChain::new(self.bound))
                .record(&projection.snapshot.entities);
        }
    }

    pub(crate) fn chain(&self, dimension: mc_core::dimensions::Dimension) -> &PublicationChain {
        if self.root_dimension == Some(dimension) {
            &self.root
        } else {
            self.dimensions.get(&dimension).unwrap_or(&EMPTY_CHAIN)
        }
    }

    /// A broadcast `Lagged` dropped outputs the pump can never see again.
    pub(crate) fn cut(&mut self) {
        self.root.cut();
        for chain in self.dimensions.values_mut() {
            chain.cut();
        }
    }
}

static EMPTY_CHAIN: std::sync::LazyLock<PublicationChain> =
    std::sync::LazyLock::new(|| PublicationChain::new(1));

/// What the pump's chain knows about history behind one publish, carried on the
/// index so a rescan can name its reason.
#[derive(Clone, Copy, Default)]
pub(crate) struct ChainInfo {
    /// Revision at or below which a broadcast `Lagged` lost outputs.
    pub(crate) lost_through: Option<u64>,
}

/// The per-id delta [`TickPlanIndex::with_previous`] derives from a pair of
/// entity revisions, shared by the single-step and the bridged path: both
/// compare a delivered base row against the final row.
fn entity_delta(
    previous: &mc_core::boundary::EntityViews,
    entities: &mc_core::boundary::EntityViews,
    id: EntityId,
    factions: &mut crate::faction_teams::FactionDelta,
) -> (Option<(i32, i32)>, EntityDelta) {
    let old = previous.get(id);
    let new = entities.get(id);
    factions.observe(old, new);
    let old_chunk = old.map(|entity| entity_chunk(entity.position));
    let chunk = new.map(|entity| entity_chunk(entity.position));
    let contract = new.and_then(|entity| {
        mc_data::entity_types::entity_type_contract_26_1_2_by_protocol_id(entity.type_id)
    });
    let delta = EntityDelta {
        id,
        chunk,
        membership_same: old
            .zip(new)
            .is_some_and(|(old, new)| old.type_id == new.type_id && old_chunk == chunk),
        appearance_changed: old
            .zip(new)
            .is_none_or(|(old, new)| !Arc::ptr_eq(&old.appearance, &new.appearance)),
        motion_changed: old.zip(new).is_none_or(|(old, new)| {
            old.position != new.position
                || old.yaw != new.yaw
                || old.pitch != new.pitch
                || old.head_yaw != new.head_yaw
        }),
        range: contract.map_or(0, |contract| i32::from(contract.tracking_range)),
    };
    (old_chunk, delta)
}

impl TickPlanIndex {
    /// (hit, miss) move frames this publish's cache served, for the plan debug line.
    pub(crate) fn move_tallies(&self) -> (u64, u64) {
        self.moves.move_tallies()
    }

    /// Attach the publish's chunk revision delta (`None`: every viewer walks
    /// its whole window).
    pub(crate) fn with_chunk_delta(mut self, delta: Option<Arc<ChunkDelta>>) -> Self {
        self.chunk_delta = delta;
        self
    }

    pub(super) fn chunk_delta(&self) -> Option<&ChunkDelta> {
        self.chunk_delta.as_deref()
    }

    pub(crate) fn faction_delta(&self) -> Option<&crate::faction_teams::FactionDelta> {
        self.factions.as_ref()
    }

    /// Add the anti-xray reveals of this tick's block changes (computed once
    /// per publish, shared by every viewer).
    pub(crate) fn with_reveals(
        mut self,
        output: &TickOutput,
        policy: Option<&crate::anti_xray::AntiXray>,
    ) -> Self {
        let Some(policy) = policy.filter(|policy| policy.enabled(output.snapshot.dimension)) else {
            return self;
        };
        let mut revealed = Vec::new();
        for change in &output.changes.blocks {
            policy.reveal_around(
                output.snapshot.dimension,
                &output.snapshot.chunks,
                change.position,
                &mut revealed,
            );
        }
        revealed.sort_unstable_by_key(|pos| (pos.x, pos.y, pos.z));
        revealed.dedup();
        for pos in revealed {
            self.reveals
                .entry((pos.x.div_euclid(16), pos.z.div_euclid(16)))
                .or_default()
                .push(pos);
        }
        self
    }

    pub(crate) fn new(output: &TickOutput) -> Self {
        // One `enabled` check per publish: the shared move cache counts hits/misses only when the
        // plan line will be written, so a server with that target off takes no counters at all
        // and the tally lives under the shard lock the lookup already holds.
        let mut index = Self {
            spectators: output
                .snapshot
                .player_roster
                .iter()
                .filter(|player| player.game_mode == 3)
                .map(|player| player.entity)
                .collect(),
            ..Self::default()
        };
        #[cfg(test)]
        SPECTATOR_INDEX_BUILDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if tracing::enabled!(target: "mc_edge::server::plan", tracing::Level::DEBUG) {
            index.moves = super::shared_moves::SharedMoves::counting();
        }
        for (i, change) in output.changes.blocks.iter().enumerate() {
            let pos = (
                change.position.x.div_euclid(16),
                change.position.z.div_euclid(16),
            );
            index.blocks.entry(pos).or_default().push(i);
        }
        for (i, explosion) in output.changes.explosions.iter().enumerate() {
            index
                .explosions
                .entry(entity_chunk(explosion.center))
                .or_default()
                .push(i);
            for &(recipient, push) in &explosion.knockback {
                if push != [0.0; 3] {
                    let (_, sum) = index.knockback.entry(recipient).or_insert((i, [0.0; 3]));
                    for axis in 0..3 {
                        sum[axis] += push[axis];
                    }
                }
            }
        }
        for (i, event) in output.changes.entity_events.iter().enumerate() {
            index
                .event_subjects
                .entry(event_subject(*event))
                .or_default()
                .events
                .push(i);
        }
        if !index.event_subjects.is_empty() {
            for (id, subject) in &mut index.event_subjects {
                subject.chunk = output
                    .snapshot
                    .entities
                    .get(*id)
                    .map(|entity| entity_chunk(entity.position));
            }
            for subject in index.event_subjects.values() {
                if let Some(pos) = subject.chunk {
                    index.events.entry(pos).or_default().extend(&subject.events);
                }
            }
        }
        index
    }

    pub(crate) fn with_previous(
        output: &TickOutput,
        previous: Option<&mc_core::boundary::EntityViews>,
    ) -> Self {
        let mut index = Self::new(output);
        let entities = &output.snapshot.entities;
        let Some(previous) = previous else {
            return index;
        };
        if previous.revision() == entities.revision() {
            index.entity_base = Some(previous.revision());
        } else if previous.revision() == entities.base_revision() {
            index.merge_delta_from(previous, entities, entities.changed());
        }
        index
    }

    /// The coalesced steps from a viewer's delivered base to this output,
    /// merged into one exact delta set: every id any step touched is compared
    /// once, base row against the final row, so spawn, removal, move, appearance
    /// and faction state are exactly what a full rescan of the same base would
    /// produce. Building this per viewer base costs one pass over the rows the
    /// skipped outputs changed, never a per-step replay.
    pub(crate) fn with_chain(
        output: &TickOutput,
        base: &mc_core::boundary::EntityViews,
        changed: &[EntityId],
    ) -> Self {
        debug_assert!(
            base.revision() < output.snapshot.entities.revision(),
            "a bridge starts at a base older than the published snapshot"
        );
        let mut index = Self::new(output);
        index.merge_delta_from(base, &output.snapshot.entities, changed);
        index
    }

    /// One delta pass from `base` to `entities` over `changed`, in the bucket
    /// shape every viewer path already reads.
    fn merge_delta_from(
        &mut self,
        base: &mc_core::boundary::EntityViews,
        entities: &mc_core::boundary::EntityViews,
        changed: &[EntityId],
    ) {
        self.entity_base = Some(base.revision());
        let mut factions = crate::faction_teams::FactionDelta::new(base.revision());
        for &id in changed {
            let (old_chunk, delta) = entity_delta(base, entities, id, &mut factions);
            if let Some(chunk) = old_chunk {
                self.entity_chunks.entry(chunk).or_default().push(delta);
            }
            if let Some(chunk) = delta.chunk.filter(|chunk| Some(*chunk) != old_chunk) {
                self.entity_chunks.entry(chunk).or_default().push(delta);
            }
        }
        self.factions = Some(factions.finish());
    }

    /// The viewer base most of them hold that the chain can bridge to
    /// `revision`, with the union of the steps' changed ids; `None` when it
    /// reaches none. A viewer already at `revision` needs no bridge and never
    /// wins the selection, so it cannot preempt the chain for the viewers
    /// that are behind. Coverage is decided first and the union is built once,
    /// for the winner only.
    pub(crate) fn bridge<'a>(
        chain: &PublicationChain,
        revision: u64,
        bases: impl Iterator<Item = &'a mc_core::boundary::EntityViews>,
    ) -> Option<(&'a mc_core::boundary::EntityViews, u64, Vec<EntityId>)> {
        let mut held: HashMap<u64, (&'a mc_core::boundary::EntityViews, usize)> =
            HashMap::default();
        for base in bases {
            if base.revision() == revision {
                continue;
            }
            held.entry(base.revision()).or_insert((base, 0)).1 += 1;
        }
        let (base, _) = held
            .into_values()
            .filter(|(base, _)| chain.covers(base.revision(), revision))
            .max_by_key(|(base, viewers)| (*viewers, base.revision()))?;
        let changed = chain.changed_between(base.revision(), revision)?;
        Some((base, base.revision(), changed))
    }

    pub(crate) fn with_chain_info(mut self, info: ChainInfo) -> Self {
        self.chain = info;
        self
    }

    pub(super) fn chain_info(&self) -> ChainInfo {
        self.chain
    }

    pub(super) fn entity_changes_for<V>(
        &self,
        chunks: &HashMap<(i32, i32), V>,
        mut changed: impl FnMut(EntityDelta),
    ) {
        for chunk in chunks.keys() {
            if let Some(deltas) = self.entity_chunks.get(chunk) {
                for &delta in deltas {
                    // Cross-chunk rows occur in both buckets. Prefer their
                    // current held bucket; the old one still routes departures.
                    if delta.chunk != Some(*chunk)
                        && delta.chunk.is_some_and(|pos| chunks.contains_key(&pos))
                    {
                        continue;
                    }
                    changed(delta);
                }
            }
        }
    }

    pub(super) fn explosions_near(&self, position: [f64; 3], candidates: &mut Vec<usize>) {
        candidates.clear();
        if self.explosions.is_empty() {
            return;
        }
        let center = entity_chunk(position);
        for x in center.0 - 4..=center.0 + 4 {
            for z in center.1 - 4..=center.1 + 4 {
                if let Some(events) = self.explosions.get(&(x, z)) {
                    candidates.extend(events);
                }
            }
        }
        candidates.sort_unstable();
    }

    pub(super) fn events_for(
        &self,
        want: &HashSet<(i32, i32)>,
        subjects: impl Iterator<Item = EntityId>,
        candidates: &mut Vec<usize>,
    ) {
        candidates.clear();
        if self.event_subjects.is_empty() {
            return;
        }
        // Most events route via the held interest square. The subject index
        // handles deaths/pickups absent from the snapshot and events preceding
        // removal after a known entity moved out of that square.
        for pos in want {
            if let Some(events) = self.events.get(pos) {
                candidates.extend(events);
            }
        }
        for subject in subjects {
            if let Some(subject) = self.event_subjects.get(&subject)
                && subject.chunk.is_none_or(|pos| !want.contains(&pos))
            {
                candidates.extend(&subject.events);
            }
        }
        candidates.sort_unstable();
        candidates.dedup();
    }
}

pub(super) fn entity_chunk(position: [f64; 3]) -> (i32, i32) {
    (floor_i32(position[0] / 16.0), floor_i32(position[2] / 16.0))
}

pub(super) fn event_subject(event: EntityEvent) -> EntityId {
    match event {
        EntityEvent::Swing { entity, .. }
        | EntityEvent::Hurt { entity }
        | EntityEvent::Damage { entity, .. }
        | EntityEvent::ShieldBlock { entity, .. }
        | EntityEvent::GoldenDandelion { entity, .. }
        | EntityEvent::Bottle { entity, .. }
        | EntityEvent::SweepAttack { entity, .. }
        | EntityEvent::Death { entity }
        | EntityEvent::Status { entity, .. } => entity,
        EntityEvent::Pickup { item, .. } => item,
    }
}
