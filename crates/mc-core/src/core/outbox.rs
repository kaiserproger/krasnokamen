//! Bounded tick→edge reliable handoff.
//!
//! The tick owner pushes one batch per tick that has reliable
//! [`PlayerChange`]s and must never block, while the edge pump drains those
//! batches first thing in its own loop. An unbounded channel satisfies the first
//! property and fails the second: a pump that stops draining grows the queue
//! forever, uncounted (audit B2).
//!
//! This module bounds it in *bytes*, like [`JournalBudget`](crate::journal_budget)
//! bounds the journal work queue, and keeps the failure visible:
//!
//! - the tick charges each batch before pushing ([`OutboxBudget::charge`], never
//!   blocks, no consumer wait) and [`ReliableOutbox::drain`] releases exactly
//!   that number when it consumes the batch;
//! - a batch that does not fit under the hard cap is refused, and the tick
//!   halts fail-closed ([`OutboxRefusal`] carries the reason); the halt's own
//!   terminal `Disconnect` batch is admitted past the cap
//!   ([`OutboxSender::push_terminal`]) so the reason still reaches an edge that
//!   is draining at all;
//! - a non-empty outbox that no drain touched for
//!   [`OUTBOX_STALL_LIMIT`] is a pump that is not keeping up — supervision of
//!   the pump task cannot see a wedged-but-alive pump, so [`StallWatch`] halts
//!   the tick the same way;
//! - crossing [`OUTBOX_WARN_BYTES`] only warns once (rate-limited by
//!   [`OutboxBudget::charge`] clearing the mark when the queue falls back below
//!   half of it) and records the peak: a mass reconnect burst must not halt a
//!   healthy server.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use crate::boundary::{ConnectionId, PlayerChange};

pub(crate) type OutboxBatch = Vec<(ConnectionId, Vec<PlayerChange>)>;
pub(crate) type TimedOutboxBatch = (u64, OutboxBatch);

/// The bounds live in `config` so the defaults have one home and `config` never
/// has to reference `core` (that edge pulled five modules into the largest
/// module cycle). `OUTBOX_QUEUE_BYTES` is used by `CoreConfig::new` there and by
/// tests only, so it is not imported here.
use crate::config::OUTBOX_STALL_LIMIT;

/// Soft mark: one warning and the peak counter, never a halt.
pub(crate) const OUTBOX_WARN_BYTES: usize = 8 << 20;

/// Flat per-connection allowance for the merged map entry the edge builds.
const CONNECTION_ALLOWANCE: usize = 64;
/// Per-element allowance for ordered map/set node bookkeeping.
const NODE_ALLOWANCE: usize = 64;

/// Element storage of a vector, by capacity (never under the allocation).
fn vec_heap<T>(items: &Vec<T>) -> usize {
    items.capacity() * std::mem::size_of::<T>()
}

/// Element storage of a slice-shaped allocation (`Arc<[T]>`).
fn slice_heap<T>(items: &[T]) -> usize {
    std::mem::size_of_val(items)
}

fn strings_heap(items: &Vec<String>) -> usize {
    items.capacity() * std::mem::size_of::<String>()
        + items.iter().map(String::capacity).sum::<usize>()
}

fn identifier_heap(id: &mc_data::Identifier) -> usize {
    id.heap_bytes()
}

/// NBT tags appear in joins (dialogs, titles, block-entity data) and can be large.
fn nbt_heap(tag: &mc_nbt::Tag) -> usize {
    match tag {
        mc_nbt::Tag::Byte(_)
        | mc_nbt::Tag::Short(_)
        | mc_nbt::Tag::Int(_)
        | mc_nbt::Tag::Long(_)
        | mc_nbt::Tag::Float(_)
        | mc_nbt::Tag::Double(_) => 0,
        mc_nbt::Tag::ByteArray(values) => values.capacity(),
        mc_nbt::Tag::String(text) => text.capacity(),
        mc_nbt::Tag::IntArray(values) => values.capacity() * std::mem::size_of::<i32>(),
        mc_nbt::Tag::LongArray(values) => values.capacity() * std::mem::size_of::<i64>(),
        mc_nbt::Tag::List(list) => {
            vec_heap(&list.elements) + list.elements.iter().map(nbt_heap).sum::<usize>()
        }
        mc_nbt::Tag::Compound(entries) => {
            vec_heap(entries)
                + entries
                    .iter()
                    .map(|(name, value)| name.capacity() + nbt_heap(value))
                    .sum::<usize>()
        }
    }
}

/// Advancement progress is JSON, sized by the join handbook at every login.
fn json_heap(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => 0,
        serde_json::Value::String(text) => text.capacity(),
        serde_json::Value::Array(items) => {
            vec_heap(items) + items.iter().map(json_heap).sum::<usize>()
        }
        serde_json::Value::Object(fields) => {
            fields.len() * (std::mem::size_of::<(String, serde_json::Value)>() + NODE_ALLOWANCE)
                + fields
                    .iter()
                    .map(|(key, value)| key.capacity() + json_heap(value))
                    .sum::<usize>()
        }
    }
}

/// One item stack with its components: stacks carry join-time menu contents and
/// villager offers, so their nested text has to be counted.
fn item_stack_heap(stack: &mc_data::ItemStack) -> usize {
    let mut heap = 0;
    heap += vec_heap(&stack.enchantments);
    heap += stack
        .enchantments
        .iter()
        .map(|enchantment| identifier_heap(&enchantment.id))
        .sum::<usize>();
    heap += stack.custom_name.as_ref().map_or(0, |name| name.len());
    // `item_model` is a shared registry identifier, not a copy.
    heap += vec_heap(&stack.stew_effects);
    heap += stack
        .stew_effects
        .iter()
        .map(|effect| identifier_heap(&effect.id))
        .sum::<usize>();
    if let Some(components) = &stack.components {
        heap += std::mem::size_of::<mc_data::item_stack::ExtendedItemComponents>()
            + extended_components_heap(components);
    }
    heap
}

fn potion_effect_heap(effect: &mc_data::item_stack::PotionEffect) -> usize {
    identifier_heap(&effect.id)
        + effect.hidden.as_deref().map_or(0, |hidden| {
            std::mem::size_of::<mc_data::item_stack::PotionEffect>() + potion_effect_heap(hidden)
        })
}

fn extended_components_heap(components: &mc_data::item_stack::ExtendedItemComponents) -> usize {
    let mut heap = 0;
    heap += components.potion.as_ref().map_or(0, identifier_heap);
    heap += vec_heap(&components.potion_custom_effects)
        + components
            .potion_custom_effects
            .iter()
            .map(potion_effect_heap)
            .sum::<usize>();
    heap += components
        .potion_custom_name
        .as_ref()
        .map_or(0, String::capacity);
    if let Some(trim) = &components.trim {
        heap += identifier_heap(&trim.material) + identifier_heap(&trim.pattern);
    }
    heap += vec_heap(&components.banner_patterns)
        + components
            .banner_patterns
            .iter()
            .map(|pattern| identifier_heap(&pattern.pattern))
            .sum::<usize>();
    heap += vec_heap(&components.stored_enchantments)
        + components
            .stored_enchantments
            .iter()
            .map(|enchantment| identifier_heap(&enchantment.id))
            .sum::<usize>();
    heap += vec_heap(&components.charged_projectiles)
        + components
            .charged_projectiles
            .iter()
            .map(item_stack_heap)
            .sum::<usize>();
    if let Some(fireworks) = &components.fireworks {
        heap += vec_heap(&fireworks.explosions)
            + fireworks
                .explosions
                .iter()
                .map(|explosion| {
                    explosion.colors.capacity() * std::mem::size_of::<i32>()
                        + explosion.fade_colors.capacity() * std::mem::size_of::<i32>()
                })
                .sum::<usize>();
    }
    heap += components.custom_data.as_ref().map_or(0, Vec::capacity);
    heap += vec_heap(&components.preserved_components)
        + components
            .preserved_components
            .iter()
            .map(|component| {
                component.name.capacity()
                    + component.value.capacity()
                    + component.baseline.as_ref().map_or(0, Vec::capacity)
            })
            .sum::<usize>();
    heap
}

fn active_effect_heap(effect: &crate::effects::ActiveEffect) -> usize {
    effect.hidden.as_deref().map_or(0, |hidden| {
        std::mem::size_of::<crate::effects::ActiveEffect>() + active_effect_heap(hidden)
    })
}

fn map_update_heap(update: &crate::maps::MapUpdate) -> usize {
    vec_heap(&update.markers)
        + update
            .markers
            .iter()
            .map(|marker| marker.name.as_ref().map_or(0, String::capacity))
            .sum::<usize>()
        + update.colors.capacity()
}

fn team_heap(team: &crate::scoreboard::Team) -> usize {
    team.display_name.capacity()
        + team.prefix.capacity()
        + team.suffix.capacity()
        + team.members.len() * (std::mem::size_of::<String>() + NODE_ALLOWANCE)
        + team.members.iter().map(String::capacity).sum::<usize>()
}

fn scoreboard_event_heap(event: &crate::scoreboard::ScoreboardEvent) -> usize {
    match event {
        crate::scoreboard::ScoreboardEvent::Team {
            name,
            team,
            members,
            ..
        } => name.capacity() + team.as_ref().map_or(0, team_heap) + strings_heap(members),
        crate::scoreboard::ScoreboardEvent::Objective {
            name, objective, ..
        } => {
            name.capacity()
                + objective.as_ref().map_or(0, |objective| {
                    objective.criteria.capacity() + objective.display_name.capacity()
                })
        }
        crate::scoreboard::ScoreboardEvent::Score {
            owner, objective, ..
        } => owner.capacity() + objective.capacity(),
        crate::scoreboard::ScoreboardEvent::Reset { owner, objective } => {
            owner.capacity() + objective.as_ref().map_or(0, String::capacity)
        }
        crate::scoreboard::ScoreboardEvent::Display { objective, .. } => objective.capacity(),
    }
}

fn villager_offer_heap(offer: &crate::villager::VillagerTradeOffer) -> usize {
    item_stack_heap(&offer.cost_a)
        + offer.cost_b.as_ref().map_or(0, item_stack_heap)
        + item_stack_heap(&offer.result)
}

fn attribute_heap(attribute: &crate::boundary::PlayerAttribute) -> usize {
    vec_heap(&attribute.modifiers)
        + attribute
            .modifiers
            .iter()
            .map(|modifier| {
                modifier.kind.capacity()
                    + modifier.id.capacity()
                    + modifier.operation.capacity()
                    + modifier.slot.as_ref().map_or(0, String::capacity)
            })
            .sum::<usize>()
}

fn title_heap(change: &crate::boundary::TitleChange) -> usize {
    match change {
        crate::boundary::TitleChange::Title(tag)
        | crate::boundary::TitleChange::Subtitle(tag)
        | crate::boundary::TitleChange::ActionBar(tag) => nbt_heap(tag),
        crate::boundary::TitleChange::Times { .. } | crate::boundary::TitleChange::Clear { .. } => {
            0
        }
    }
}

fn boss_bar_heap(change: &crate::boundary::BossBarChange) -> usize {
    match change {
        crate::boundary::BossBarChange::Add { name, .. }
        | crate::boundary::BossBarChange::Name(name) => nbt_heap(name),
        crate::boundary::BossBarChange::Remove
        | crate::boundary::BossBarChange::Progress(_)
        | crate::boundary::BossBarChange::Style { .. }
        | crate::boundary::BossBarChange::Properties(_) => 0,
    }
}

/// Allocations already charged in the batch being priced, so one snapshot
/// broadcast to every recipient is counted once per batch, not once per copy.
type Seen = std::collections::HashSet<usize>;

fn arc_key<T>(allocation: &Arc<[T]>) -> usize {
    Arc::as_ptr(allocation) as *const () as usize
}

impl PlayerChange {
    /// Heap bytes this change keeps alive on its own, without the batch-level
    /// snapshot dedupe: `batch_bytes` uses [`Self::heap_bytes_with`] so a
    /// snapshot every recipient receives is charged once. Tests compare this
    /// against a walk of the payload.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn heap_bytes(&self) -> usize {
        self.heap_bytes_with(&mut Seen::new())
    }

    /// Heap bytes this change keeps alive on its own, for the outbox byte bound.
    ///
    /// Exhaustive on purpose: a new variant cannot skip accounting. Only
    /// process-static or reload-scoped `Arc`s count as references, because a
    /// queued batch does not grow them:
    /// - `Advancements::definitions` and `RecipesUnlocked`/`RecipesReloaded`
    ///   registry handles live in the core registry and are replaced on a
    ///   datapack reload (operator action), not per tick;
    /// - `ItemStack::item_model` is an identifier owned by the item registry.
    /// Everything else — roster snapshots, scoreboard event slices, NBT, JSON,
    /// stacks, text — is counted in full, including the capacity of its
    /// containers, so a retained generation is never invisible. A snapshot one
    /// tick broadcasts to every recipient is charged once per batch
    /// (`seen` carries the allocations already priced in this batch), not once
    /// per copy.
    fn heap_bytes_with(&self, seen: &mut Seen) -> usize {
        match self {
            Self::Map { update } => map_update_heap(update),
            Self::Advancements { progress, .. } => json_heap(progress),
            Self::AdvancementsTab { tab } => tab.as_ref().map_or(0, String::capacity),
            // `RegistryReload` holds the shared registry.
            Self::RecipesReloaded { .. } => 0,
            Self::RecipesUnlocked { recipes, .. } => strings_heap(recipes),
            Self::MobEffect { effect, .. } => active_effect_heap(effect),
            Self::Scoreboard { events } => {
                if !seen.insert(arc_key(events)) {
                    return 0;
                }
                slice_heap(events) + events.iter().map(scoreboard_event_heap).sum::<usize>()
            }
            // A new roster snapshot per roster change: queued batches keep their
            // own generation alive, so each distinct snapshot is charged once.
            Self::Roster { players, removed } => {
                let mut heap = 0;
                if seen.insert(arc_key(players)) {
                    heap += slice_heap(players)
                        + players
                            .iter()
                            .map(|player| player.name.capacity())
                            .sum::<usize>();
                }
                if seen.insert(arc_key(removed)) {
                    heap += slice_heap(removed);
                }
                heap
            }
            Self::StyledChat { content } => nbt_heap(content),
            Self::TextBurst { lines } => {
                vec_heap(lines)
                    + lines
                        .iter()
                        .map(|line| line.heap_bytes_with(seen))
                        .sum::<usize>()
            }
            Self::ItemCooldown { group, .. } => group.capacity(),
            Self::Attributes { attributes } => {
                vec_heap(attributes) + attributes.iter().map(attribute_heap).sum::<usize>()
            }
            Self::BlockEntityData { nbt, .. } => nbt_heap(nbt),
            Self::CommandSound { sound, .. } => identifier_heap(sound),
            Self::MenuOpened { title, .. } => title.capacity(),
            Self::ShowDialog { dialog } => nbt_heap(dialog),
            Self::MerchantOffers { offers, .. } => {
                vec_heap(offers) + offers.iter().map(villager_offer_heap).sum::<usize>()
            }
            Self::MenuContents { slots, cursor, .. } => {
                vec_heap(slots)
                    + slots.iter().flatten().map(item_stack_heap).sum::<usize>()
                    + cursor.as_ref().map_or(0, item_stack_heap)
            }
            Self::MenuSlot { item, .. }
            | Self::PlayerInventorySlot { item, .. }
            | Self::Cursor { item } => item.as_ref().map_or(0, item_stack_heap),
            Self::PluginClientSound { plugin_id, sound } => {
                plugin_id.capacity() + sound.heap_bytes()
            }
            Self::PluginClientView { payload } => payload.capacity(),
            Self::Died { message } => nbt_heap(message),
            Self::Title(change) => title_heap(change),
            Self::BossBar { change, .. } => boss_bar_heap(change),
            Self::Chat { message } => message.capacity(),
            Self::Disconnect { reason } => reason.capacity(),
            Self::Statistics { stats } => vec_heap(stats),
            Self::VehicleCorrected { .. }
            | Self::PermissionLevel { .. }
            | Self::ProjectileStatus { .. }
            | Self::ProjectileLevelEvent { .. }
            | Self::RemoveMobEffect { .. }
            | Self::Absorption { .. }
            | Self::AirSupply { .. }
            | Self::ShieldCooldown { .. }
            | Self::Joined { .. }
            | Self::PositionCorrected { .. }
            | Self::Motion { .. }
            | Self::BlockResynced { .. }
            | Self::BlockActionsAcked { .. }
            | Self::BookOpened { .. }
            | Self::SignEditorOpened { .. }
            | Self::BlockEvent { .. }
            | Self::BellRung { .. }
            | Self::CommandParticle { .. }
            | Self::MountMenuOpened { .. }
            | Self::ClearDialog
            | Self::MenuClosed { .. }
            | Self::HeldSlot { .. }
            | Self::MenuProperty { .. }
            | Self::Experience { .. }
            | Self::GameModeChanged { .. }
            | Self::WeatherChanged { .. }
            | Self::DifficultyChanged { .. }
            | Self::TickingState { .. }
            | Self::TickingStep { .. }
            | Self::Health { .. }
            | Self::DimensionChanged { .. }
            | Self::Respawned { .. } => 0,
        }
    }
}

/// Estimated resident bytes of one batch: a conservative over-count, never exact.
///
/// The outer slots, the change array by capacity and each change's own heap
/// ([`PlayerChange::heap_bytes`]), plus a flat merged-map allowance. The same
/// batch is priced identically when the tick charges it and when the edge
/// releases it, because nothing mutates a queued batch.
pub(crate) fn batch_bytes(batch: &[(ConnectionId, Vec<PlayerChange>)]) -> usize {
    // One `seen` set for the whole batch: a snapshot every recipient receives is
    // charged once, while a batch that retains two generations pays for both.
    let mut seen = Seen::new();
    std::mem::size_of_val(batch)
        + batch
            .iter()
            .map(|(_, changes)| {
                changes.capacity() * std::mem::size_of::<PlayerChange>()
                    + changes
                        .iter()
                        .map(|change| change.heap_bytes_with(&mut seen))
                        .sum::<usize>()
                    + CONNECTION_ALLOWANCE
            })
            .sum::<usize>()
}

/// Accounting for the queued reliable batches. Shared by the tick's sender, the
/// edge's receiver and the core handle's counters.
#[derive(Debug)]
pub(crate) struct OutboxBudget {
    cap: usize,
    queued: AtomicUsize,
    peak: AtomicUsize,
    drains: AtomicU64,
    warned: AtomicBool,
}

impl OutboxBudget {
    pub(crate) fn new(cap: usize) -> Self {
        Self {
            cap: cap.max(1),
            queued: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            drains: AtomicU64::new(0),
            warned: AtomicBool::new(false),
        }
    }

    pub(crate) fn cap(&self) -> usize {
        self.cap
    }

    pub(crate) fn queued_bytes(&self) -> usize {
        self.queued.load(Ordering::Acquire)
    }

    pub(crate) fn peak_bytes(&self) -> usize {
        self.peak.load(Ordering::Acquire)
    }

    /// Drains that consumed at least one batch; the stall watchdog's progress
    /// counter. An empty drain, or one that holds back a newer `pending` batch,
    /// is not progress and must not reset the watchdog.
    pub(crate) fn drains(&self) -> u64 {
        self.drains.load(Ordering::Acquire)
    }

    /// Charge one batch, non-blocking. `false` means it does not fit under the
    /// hard cap; an oversized single batch is admitted only into an empty queue
    /// (the [`JournalBudget`](crate::journal_budget) rule) and then stays over
    /// budget until it is drained.
    fn charge(&self, bytes: usize) -> bool {
        let mut queued = self.queued.load(Ordering::Acquire);
        loop {
            let admitted = queued == 0 || queued.saturating_add(bytes) <= self.cap;
            if !admitted {
                return false;
            }
            match self.queued.compare_exchange_weak(
                queued,
                queued.saturating_add(bytes),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(actual) => queued = actual,
            }
        }
        let total = queued.saturating_add(bytes);
        self.peak.fetch_max(total, Ordering::AcqRel);
        if total >= OUTBOX_WARN_BYTES && !self.warned.swap(true, Ordering::AcqRel) {
            tracing::warn!(
                queued_bytes = total,
                cap_bytes = self.cap,
                stall_limit_s = OUTBOX_STALL_LIMIT.as_secs(),
                "reliable outbox above its warning mark; the edge pump is behind"
            );
        }
        true
    }

    /// Release a batch the edge consumed.
    fn release(&self, bytes: usize) {
        let after = self.queued.fetch_sub(bytes, Ordering::AcqRel);
        let remaining = after.saturating_sub(bytes);
        // Re-arm the one-shot warning only after the queue falls back well
        // below the mark, so a hovering queue cannot log on every batch.
        if remaining < OUTBOX_WARN_BYTES / 2 {
            self.warned.store(false, Ordering::Release);
        }
    }

    /// One drain that consumed at least one batch: real progress for the
    /// stall watchdog. A pump that keeps calling `drain` without consuming
    /// anything must not be able to hold the watchdog off.
    fn drained(&self) {
        self.drains.fetch_add(1, Ordering::AcqRel);
    }
}

/// Why a push was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OutboxRefusal {
    /// The batch does not fit under the hard cap.
    Full { queued: usize, cap: usize },
    /// The core handle holding the receiver is gone.
    Closed,
}

impl OutboxRefusal {
    /// The fail-closed halt reason; the tick stops rather than dropping or
    /// blocking.
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::Full { queued, cap } => format!(
                "reliable outbox full: {queued} of {cap} bytes queued, edge pump not draining (fail closed)"
            ),
            Self::Closed => "reliable outbox receiver dropped (fail closed)".to_owned(),
        }
    }
}

/// Tick-side sender: never blocks, never drops a reliable batch silently.
#[derive(Debug, Clone)]
pub(crate) struct OutboxSender {
    tx: std::sync::mpsc::Sender<TimedOutboxBatch>,
    budget: Arc<OutboxBudget>,
}

impl OutboxSender {
    pub(crate) fn new(
        tx: std::sync::mpsc::Sender<TimedOutboxBatch>,
        budget: Arc<OutboxBudget>,
    ) -> Self {
        Self { tx, budget }
    }

    pub(crate) fn budget(&self) -> &Arc<OutboxBudget> {
        &self.budget
    }

    pub(crate) fn queued_bytes(&self) -> usize {
        self.budget.queued_bytes()
    }

    /// Charge and push one batch. `Err` is the tick's fail-closed decision: the
    /// batch was not queued and the server must stop, not drop it.
    pub(crate) fn push(&self, tick: u64, batch: OutboxBatch) -> Result<(), OutboxRefusal> {
        let bytes = batch_bytes(&batch);
        if !self.budget.charge(bytes) {
            return Err(OutboxRefusal::Full {
                queued: self.budget.queued_bytes(),
                cap: self.budget.cap(),
            });
        }
        if self.tx.send((tick, batch)).is_err() {
            self.budget.release(bytes);
            return Err(OutboxRefusal::Closed);
        }
        Ok(())
    }

    /// The halt path's terminal batch: admitted past the hard cap so every live
    /// connection still receives its `Disconnect` reason from a draining edge.
    pub(crate) fn push_terminal(&self, tick: u64, batch: OutboxBatch) -> Result<(), OutboxRefusal> {
        let bytes = batch_bytes(&batch);
        self.budget.queued.fetch_add(bytes, Ordering::AcqRel);
        self.budget
            .peak
            .fetch_max(self.budget.queued_bytes(), Ordering::AcqRel);
        if self.tx.send((tick, batch)).is_err() {
            self.budget.release(bytes);
            return Err(OutboxRefusal::Closed);
        }
        Ok(())
    }
}

/// Tick-side stall detector: a non-empty outbox whose drain epoch has not moved
/// for the whole limit. Instant-driven and instant-checked: no sleeps, and no
/// state beyond the last observation.
#[derive(Debug)]
pub(crate) struct StallWatch {
    limit: std::time::Duration,
    epoch: u64,
    since: Option<std::time::Instant>,
}

impl StallWatch {
    pub(crate) fn new(limit: std::time::Duration) -> Self {
        Self {
            limit,
            epoch: 0,
            since: None,
        }
    }

    /// `Some(waited)` when the outbox has been non-empty and undrained for the
    /// whole limit; `None` (and the clock reset) otherwise.
    pub(crate) fn stalled_at(
        &mut self,
        budget: &OutboxBudget,
        now: std::time::Instant,
    ) -> Option<std::time::Duration> {
        if budget.queued_bytes() == 0 {
            self.since = None;
            return None;
        }
        let epoch = budget.drains();
        match self.since {
            Some(since) if self.epoch == epoch => {
                let waited = now.saturating_duration_since(since);
                (waited >= self.limit).then_some(waited)
            }
            _ => {
                self.epoch = epoch;
                self.since = Some(now);
                None
            }
        }
    }
}

/// Only edge consumers share this receiver; the tick owns its sender.
#[derive(Debug)]
pub(crate) struct ReliableOutbox {
    inner: parking_lot::Mutex<OutboxReceiver>,
    budget: Arc<OutboxBudget>,
}

#[derive(Debug)]
pub(crate) struct OutboxReceiver {
    pub(crate) receiver: std::sync::mpsc::Receiver<TimedOutboxBatch>,
    pub(crate) pending: Option<TimedOutboxBatch>,
}

impl ReliableOutbox {
    pub(crate) fn new(
        receiver: std::sync::mpsc::Receiver<TimedOutboxBatch>,
        budget: Arc<OutboxBudget>,
    ) -> Self {
        Self {
            inner: parking_lot::Mutex::new(OutboxReceiver {
                receiver,
                pending: None,
            }),
            budget,
        }
    }

    pub(crate) fn budget(&self) -> &Arc<OutboxBudget> {
        &self.budget
    }

    /// Drain every batch through `through`, coalescing per connection in FIFO
    /// order and holding back newer batches untouched.
    pub(crate) fn drain(&self, through: u64) -> OutboxBatch {
        let mut receiver = self.inner.lock();
        let mut store: HashMap<ConnectionId, Vec<PlayerChange>> = HashMap::new();
        let mut consumed = 0usize;
        while let Some((tick, batch)) = receiver
            .pending
            .take()
            .or_else(|| receiver.receiver.try_recv().ok())
        {
            if tick > through {
                receiver.pending = Some((tick, batch));
                break;
            }
            let charged = batch_bytes(&batch);
            for (connection, queue) in batch {
                let slot = store.entry(connection).or_default();
                if !slot
                    .last()
                    .is_some_and(|change| matches!(change, PlayerChange::Disconnect { .. }))
                {
                    slot.extend(queue);
                }
            }
            self.budget.release(charged);
            consumed += 1;
        }
        // Only a consumed batch is progress: a pump that keeps calling `drain`
        // while holding a newer batch back must not reset the stall watchdog.
        if consumed > 0 {
            self.budget.drained();
        }
        store
            .into_iter()
            .filter(|(_, queue)| !queue.is_empty())
            .collect()
    }
}

#[cfg(test)]
#[path = "outbox_tests.rs"]
mod tests;
