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

/// One continuous entity-revision step the pump drained or skipped, kept as
/// the deltas themselves: a link outlives the publishes it describes without
/// holding an [`mc_core::boundary::EntityViews`], whose copy-on-write pages
/// would stay resident for as long as the chain is.
#[derive(Debug, Default)]
pub(crate) struct EntityLink {
    base: u64,
    revision: u64,
    chunks: HashMap<(i32, i32), Vec<EntityDelta>>,
    factions: Option<crate::faction_teams::FactionDelta>,
}

impl EntityLink {
    /// The step from `previous` to `entities`, or `None` when they are not one
    /// continuous step (a rebuilt snapshot, or a step whose predecessor was not
    /// at hand): nothing may bridge across that.
    pub(crate) fn between(
        previous: &mc_core::boundary::EntityViews,
        entities: &mc_core::boundary::EntityViews,
    ) -> Option<Self> {
        if previous.revision() == entities.revision()
            || previous.revision() != entities.base_revision()
        {
            return None;
        }
        let mut link = Self {
            base: previous.revision(),
            revision: entities.revision(),
            ..Self::default()
        };
        let mut factions = crate::faction_teams::FactionDelta::new(previous.revision());
        for &id in entities.changed() {
            let (old_chunk, delta) = entity_delta(previous, entities, id, &mut factions);
            if let Some(chunk) = old_chunk {
                link.chunks.entry(chunk).or_default().push(delta);
            }
            if let Some(chunk) = delta.chunk.filter(|chunk| Some(*chunk) != old_chunk) {
                link.chunks.entry(chunk).or_default().push(delta);
            }
        }
        link.factions = Some(factions.finish());
        Some(link)
    }

    pub(crate) fn base(&self) -> u64 {
        self.base
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// Whether any step of this link carries `id` (a spawn, removal or move).
    pub(crate) fn tracks(&self, id: EntityId) -> bool {
        self.chunks
            .values()
            .any(|deltas| deltas.iter().any(|delta| delta.id == id))
    }

    /// Bytes this link's buckets, deltas and faction rows hold.
    pub(crate) fn bytes(&self) -> usize {
        let buckets =
            self.chunks.capacity() * std::mem::size_of::<((i32, i32), Vec<EntityDelta>)>();
        let rows = self
            .chunks
            .values()
            .map(|deltas| deltas.capacity())
            .sum::<usize>()
            * std::mem::size_of::<EntityDelta>();
        let factions = self.factions.as_ref().map_or(0, |delta| delta.bytes());
        buckets + rows + factions
    }
}

/// The per-id delta [`TickPlanIndex::with_previous`] derives from a pair of
/// entity revisions, computed once for both a direct step and a chain link.
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

/// One entity id across a coalesced chain, merged from the steps' deltas.
struct MergedDelta {
    /// The bucket the row departs from: the earliest step that shows it leaving
    /// that chunk, or its removal. `None`: the row did not exist at the base.
    old_chunk: Option<(i32, i32)>,
    chunk: Option<(i32, i32)>,
    membership_same: bool,
    appearance_changed: bool,
    motion_changed: bool,
    range: i32,
    /// Latest step that carried this id, so its current chunk and range win.
    step: usize,
}

impl MergedDelta {
    fn new(step: usize, bucket: (i32, i32), delta: EntityDelta) -> Self {
        Self {
            old_chunk: (delta.chunk != Some(bucket)).then_some(bucket),
            chunk: delta.chunk,
            membership_same: delta.membership_same,
            appearance_changed: delta.appearance_changed,
            motion_changed: delta.motion_changed,
            range: delta.range,
            step,
        }
    }

    fn absorb(&mut self, step: usize, bucket: (i32, i32), delta: EntityDelta) {
        if self.old_chunk.is_none() && delta.chunk != Some(bucket) {
            self.old_chunk = Some(bucket);
        }
        self.membership_same &= delta.membership_same;
        self.appearance_changed |= delta.appearance_changed;
        self.motion_changed |= delta.motion_changed;
        if step > self.step {
            self.step = step;
            self.chunk = delta.chunk;
            self.range = delta.range;
        }
    }
}

/// The steps a pump coalesced over, oldest first: what a viewer whose publish
/// was skipped needs to stay contiguous. Bounded by the tick broadcast's
/// capacity and cut at any `Lagged`, so no viewer's history is bridged across
/// outputs the pump never received.
#[derive(Default)]
pub(crate) struct PublicationChain {
    links: std::collections::VecDeque<EntityLink>,
    bound: usize,
    /// Newest revision a broadcast `Lagged` dropped before it could be drained.
    lost_through: Option<u64>,
    /// The newest drained views, kept only until the next drain derives its
    /// step from them; never part of a link.
    head: Option<mc_core::boundary::EntityViews>,
}

impl PublicationChain {
    pub(crate) fn new(bound: usize) -> Self {
        Self {
            bound: bound.max(1),
            ..Self::default()
        }
    }

    /// Record an output the pump drained or skipped, in drain order.
    pub(crate) fn record(&mut self, entities: &mc_core::boundary::EntityViews) {
        let Some(previous) = self.head.replace(entities.clone()) else {
            self.links.clear();
            return;
        };
        match EntityLink::between(&previous, entities) {
            Some(link) => {
                self.links.push_back(link);
                while self.links.len() > self.bound {
                    self.links.pop_front();
                }
            }
            // Not one continuous step (a rebuilt snapshot or a gap): older
            // links can no longer reach this publish.
            None => self.links.clear(),
        }
    }

    /// A broadcast `Lagged` dropped outputs the pump can never see again.
    pub(crate) fn cut(&mut self) {
        let lost = self
            .head
            .as_ref()
            .map(mc_core::boundary::EntityViews::revision);
        self.lost_through = match (self.lost_through, lost) {
            (Some(old), Some(lost)) => Some(old.max(lost)),
            (old, lost) => old.or(lost),
        };
        self.links.clear();
        self.head = None;
    }

    /// The contiguous steps from `base` to `revision`, oldest first; `None`
    /// when any step between them is missing.
    pub(crate) fn steps(&self, base: u64, revision: u64) -> Option<Vec<&EntityLink>> {
        let mut steps = Vec::new();
        let mut want = base;
        for link in &self.links {
            if steps.is_empty() && link.base != base {
                continue;
            }
            if link.base != want {
                return None;
            }
            steps.push(link);
            want = link.revision;
            if want == revision {
                return Some(steps);
            }
        }
        None
    }

    pub(crate) fn info(&self) -> ChainInfo {
        ChainInfo {
            lost_through: self.lost_through,
            floor: self.links.front().map(EntityLink::base),
        }
    }

    /// Bytes the chain holds at its current depth; the transient head views are
    /// not part of it.
    pub(crate) fn bytes(&self) -> usize {
        self.links.iter().map(EntityLink::bytes).sum()
    }

    pub(crate) fn len(&self) -> usize {
        self.links.len()
    }
}

/// What the pump's chain knows about history behind one publish, carried on the
/// index so a rescan can name its reason.
#[derive(Clone, Copy, Default)]
pub(crate) struct ChainInfo {
    /// Revision at or below which a broadcast `Lagged` lost outputs.
    pub(crate) lost_through: Option<u64>,
    /// Base of the oldest contiguous step the pump still holds.
    pub(crate) floor: Option<u64>,
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
        } else if let Some(link) = EntityLink::between(previous, entities) {
            let base = link.base;
            index.apply_link(&link, base);
        }
        index
    }

    /// One continuous step as this index's base: the shape [`Self::with_previous`]
    /// always had for a pair of consecutive entity revisions.
    fn apply_link(&mut self, link: &EntityLink, base: u64) {
        self.entity_base = Some(base);
        for (chunk, deltas) in &link.chunks {
            self.entity_chunks
                .entry(*chunk)
                .or_default()
                .extend_from_slice(deltas);
        }
        self.factions.clone_from(&link.factions);
    }

    /// A coalesced chain of steps from `base` to this output's revision,
    /// merged into one delta set, so a viewer whose publish the pump skipped is
    /// still contiguous. An entity's earliest step supplies the bucket it
    /// departs from, its latest the current chunk and range; a change at any
    /// step counts as a change from `base`. Over-reporting costs that entity a
    /// full revisit, never a missed spawn, removal or move.
    pub(crate) fn with_chain(output: &TickOutput, chain: &[&EntityLink], base: u64) -> Self {
        debug_assert!(chain.first().is_some_and(|link| link.base == base));
        debug_assert!(
            chain
                .last()
                .is_some_and(|link| link.revision == output.snapshot.entities.revision())
        );
        let mut index = Self::new(output);
        let mut merged: HashMap<EntityId, MergedDelta> = HashMap::default();
        let mut factions = crate::faction_teams::FactionDelta::new(base);
        for (step, link) in chain.iter().enumerate() {
            if let Some(later) = link.factions.as_ref() {
                factions.merge(later);
            }
            for (chunk, deltas) in &link.chunks {
                for delta in deltas {
                    merged
                        .entry(delta.id)
                        .and_modify(|merged| merged.absorb(step, *chunk, *delta))
                        .or_insert_with(|| MergedDelta::new(step, *chunk, *delta));
                }
            }
        }
        for (id, merged) in merged {
            let delta = EntityDelta {
                id,
                chunk: merged.chunk,
                membership_same: merged.membership_same,
                appearance_changed: merged.appearance_changed,
                motion_changed: merged.motion_changed,
                range: merged.range,
            };
            if let Some(chunk) = merged.old_chunk {
                index.entity_chunks.entry(chunk).or_default().push(delta);
            }
            if let Some(chunk) = merged
                .chunk
                .filter(|chunk| Some(*chunk) != merged.old_chunk)
            {
                index.entity_chunks.entry(chunk).or_default().push(delta);
            }
        }
        index.entity_base = Some(base);
        index.factions = Some(factions.finish());
        index
    }

    /// The base most viewers hold that the chain can bridge to `revision`,
    /// with every step between; `None` when the chain reaches none of them.
    pub(crate) fn bridge<'a>(
        chain: &'a PublicationChain,
        revision: u64,
        bases: impl Iterator<Item = u64>,
    ) -> Option<(u64, Vec<&'a EntityLink>)> {
        let mut held: HashMap<u64, usize> = HashMap::default();
        for base in bases {
            *held.entry(base).or_default() += 1;
        }
        held.into_iter()
            .filter_map(|(base, viewers)| {
                chain
                    .steps(base, revision)
                    .map(|steps| (base, viewers, steps))
            })
            .max_by_key(|(base, viewers, _)| (*viewers, *base))
            .map(|(base, _, steps)| (base, steps))
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
