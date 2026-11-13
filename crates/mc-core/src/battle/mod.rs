//! Ordered battle kernels over frozen native entity rows. Physics and intent
//! merge stay in the ordinary tick owners; battle policy lives in this module.

pub(crate) mod archery;
pub(crate) mod cavalry;
mod columns;
pub(crate) use columns::Columns;
pub(crate) mod combat;
mod faction;
pub(crate) mod formation;
mod ground;
pub(crate) mod live_players;
pub(crate) mod morale;
pub mod staging;
pub(crate) mod steering;
mod targeting;

pub(crate) use faction::special;
pub use faction::{enemies, retained_enemy};
pub use targeting::{
    CELL_BLOCKS, EnemyTarget, MELEE_ENGAGEMENT_REACH, Query, RETARGET_TICKS, TargetGrid, Unit,
    Work, engagement_reach, retarget_due,
};

use crate::boundary::{EntityEvent, EntityId};
use crate::entities::EntityStore;
use crate::intent::{Footprint, Intent, IntentKind};
use crate::registry::CoreRegistry;
use crate::tick::TickState;
use crate::world::ChunkWorld;

/// Frozen input shared by every sub-step. `tick` is battle-local and advances
/// only while running, so pause never skips windups, volleys or morale clocks.
pub(crate) struct Frame<'a> {
    pub entities: &'a EntityStore,
    pub registry: &'a CoreRegistry,
    pub world: &'a ChunkWorld,
    pub tick: u64,
}

#[derive(Debug, Default)]
pub(crate) struct BattleState {
    pub running: bool,
    pub elapsed_ticks: u64,
    pub formation: formation::FormationRuntime,
    pub combat: combat::CombatRuntime,
    pub columns: Columns,
    grid: TargetGrid,
    pub work: Work,
    pub timings: Timings,
    output: Output,
    population: PopulationCache,
    /// Staged bodies (foot soldiers and horses) not yet stood on their ground.
    pub ground_pending: Vec<EntityId>,
    /// Staged bodies with no dry, clear ground near their slot: start refuses.
    pub ground_refused: Vec<EntityId>,
    /// Network players watched, holding a front-rank slot or spectating.
    pub live: live_players::LiveState,
    #[cfg(test)]
    trace: Option<trace_tests::FrameTrace>,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Timings {
    pub total_us: u64,
    pub counters_us: u64,
    pub formation_us: u64,
    pub columns_us: u64,
    pub targeting_us: u64,
    pub steering_us: u64,
    pub combat_us: u64,
    pub morale_us: u64,
    pub projection_us: u64,
    pub emit_us: u64,
}

fn phase_us(mark: &mut Option<std::time::Instant>) -> u64 {
    let Some(previous) = mark else { return 0 };
    let now = std::time::Instant::now();
    let elapsed = now.duration_since(*previous).as_micros() as u64;
    *previous = now;
    elapsed
}

#[derive(Debug)]
pub(crate) struct BattleIntent {
    pub actor: EntityId,
    pub kind: IntentKind,
    pub footprint: Footprint,
}

#[derive(Debug)]
pub(crate) struct ArrowLaunch {
    pub owner: EntityId,
    pub from: [f64; 3],
    /// Native projectile convention: blocks/tick, unlike steering.
    pub velocity: [f64; 3],
    pub damage: f32,
}

/// Latest native faction population: sampled by the battle hook and refreshed
/// after merge at publication. Includes horses and dead retained rows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnitCounters {
    pub tick: u64,
    pub units: u64,
    pub alive_per_faction: Vec<FactionAlive>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FactionAlive {
    pub faction: u16,
    pub alive: u64,
}

impl UnitCounters {
    fn clear(&mut self) {
        self.units = 0;
        self.alive_per_faction.clear();
    }

    fn record(&mut self, faction: u16, alive: bool) {
        if faction == 0 {
            return;
        }
        self.units += 1;
        let alive = u64::from(alive);
        if let Some(count) = self
            .alive_per_faction
            .iter_mut()
            .find(|c| c.faction == faction)
        {
            count.alive += alive;
        } else {
            self.alive_per_faction.push(FactionAlive { faction, alive });
        }
    }

    fn finish(&mut self) {
        self.alive_per_faction
            .sort_unstable_by_key(|count| count.faction);
    }

    #[cfg(test)]
    fn sample(&mut self, entities: &EntityStore, tick: u64) {
        self.tick = tick;
        self.clear();
        for slot in entities.battle_slots() {
            let body = entities.body(slot);
            self.record(body.faction, body.alive && entities.row(slot).health > 0.0);
        }
        self.finish();
    }
}

#[derive(Debug)]
struct HealthFact {
    id: EntityId,
    health: f32,
    valid: bool,
}

/// Native-slot health memo, never a population owner. Every sample still folds
/// current hot membership/alive flags, so removal, repacking and corpses are exact.
#[derive(Debug, Default)]
pub(crate) struct PopulationCache {
    health: Vec<HealthFact>,
    #[cfg(test)]
    health_reads: u64,
}

impl PopulationCache {
    fn prepare(&mut self, entities: &EntityStore) {
        self.health.resize_with(entities.len(), || HealthFact {
            id: EntityId(0),
            health: 0.0,
            valid: false,
        });
    }

    fn invalidate(&mut self, id: EntityId, slot: crate::entities::EntitySlot) {
        if let Some(fact) = self.health.get_mut(slot.0 as usize)
            && fact.id == id
        {
            fact.valid = false;
        }
    }

    fn remember(&mut self, slot: crate::entities::EntitySlot, id: EntityId, health: f32) {
        self.health[slot.0 as usize] = HealthFact {
            id,
            health,
            valid: true,
        };
    }

    fn health(&mut self, entities: &EntityStore, slot: crate::entities::EntitySlot) -> f32 {
        let id = entities.body(slot).id;
        let fact = &mut self.health[slot.0 as usize];
        if !fact.valid || fact.id != id {
            fact.id = id;
            fact.health = entities.row(slot).health;
            fact.valid = true;
            #[cfg(test)]
            {
                self.health_reads += 1;
            }
        }
        fact.health
    }

    fn sample(&mut self, entities: &EntityStore, tick: u64, counts: &mut UnitCounters) {
        counts.tick = tick;
        counts.clear();
        if entities.battle_slots().next().is_none() {
            return;
        }
        self.prepare(entities);
        for (id, slot) in entities.pending_wire_changes() {
            self.invalidate(id, slot);
        }
        for slot in entities.battle_slots() {
            let body = entities.body(slot);
            counts.record(
                body.faction,
                body.alive && self.health(entities, slot) > 0.0,
            );
        }
        counts.finish();
    }
}

fn invalidate_facts(
    columns: &mut Columns,
    population: &mut PopulationCache,
    entities: &EntityStore,
) {
    if columns.ids.is_empty() && population.health.is_empty() {
        return;
    }
    for (id, slot) in entities.pending_wire_changes() {
        columns.invalidate(id, slot);
        population.invalidate(id, slot);
    }
}

/// Latest sample keyed by global server tick: pre-motion after `step`, and
/// post-merge after publication. Staged and paused populations remain visible.
pub fn unit_counters(hostile: &crate::hostile::HostileState) -> &UnitCounters {
    &hostile.battle.output.counters
}

pub(crate) fn publication_counts(
    state: &mut TickState,
) -> Option<std::sync::Arc<crate::boundary::BattleCounts>> {
    let battle = &mut state.hostile.battle;
    let counts = &mut battle.output.counters;
    battle
        .population
        .sample(&state.entities, state.tick, counts);
    if counts.units == 0 {
        return None;
    }
    let engaged = |count: u64| {
        if battle.running {
            u32::try_from(count).expect("battle fighters exceed native slot capacity")
        } else {
            0
        }
    };
    Some(std::sync::Arc::new(crate::boundary::BattleCounts {
        units: u32::try_from(counts.units).expect("battle population exceeds native slot capacity"),
        alive: counts
            .alive_per_faction
            .iter()
            .map(|count| {
                (
                    count.faction,
                    u32::try_from(count.alive)
                        .expect("battle faction exceeds native slot capacity"),
                )
            })
            .collect(),
        in_weapon_reach: engaged(battle.combat.work.in_weapon_reach),
        fighting: engaged(battle.combat.work.fighting),
        shields_raised: engaged(battle.combat.work.shields_raised),
        shield_blocks: engaged(battle.combat.work.shield_blocks),
    }))
}
/// Every `EntityStore::views()` drain consumes the pending-wire feed: tick
/// publication and the dimension-travel refresh both drain through here, so
/// late native writes (including deferred multi-dimension commands) reach the
/// battle's derived facts first. Native custody is unchanged.
pub(crate) fn drain_entity_views(state: &mut TickState) -> crate::boundary::EntityViews {
    invalidate_wire_facts(state);
    state.entities.views()
}

/// Only derived facts are invalidated; native custody is unchanged.
fn invalidate_wire_facts(state: &mut TickState) {
    let battle = &mut state.hostile.battle;
    invalidate_facts(&mut battle.columns, &mut battle.population, &state.entities);
}

#[derive(Debug, Default)]
pub(crate) struct Output {
    pub intents: Vec<BattleIntent>,
    pub events: Vec<EntityEvent>,
    pub arrows: Vec<ArrowLaunch>,
    /// Shield pose transitions: (bearer, raised), applied after the frozen pass.
    pub shields: Vec<(EntityId, bool)>,
    pub counters: UnitCounters,
}

/// The only battle tick call. Ordered kernels read a frozen world/entity store
/// and write only battle runtime + private output; native effects apply after
/// the frozen borrow ends. Spawned arrows enter ordinary projectile simulation.
pub(crate) fn step(state: &mut TickState) -> Vec<Intent> {
    // Chunks admitted this tick are settled before their first physics step.
    if !state.hostile.battle.ground_pending.is_empty() {
        staging::settle(state);
    }
    live_players::update(state);
    let start = std::env::var_os("MC_CORE_PHASE_LOG").map(|_| std::time::Instant::now());
    let mut mark = start;
    let mut battle = std::mem::take(&mut state.hostile.battle);
    battle.work = Work::default();
    let mut output = std::mem::take(&mut battle.output);
    if !battle.running {
        battle
            .population
            .sample(&state.entities, state.tick, &mut output.counters);
    } else {
        output.counters.tick = state.tick;
    }
    let mut timings = Timings {
        counters_us: phase_us(&mut mark),
        ..Default::default()
    };
    if !battle.running {
        battle.columns.drive_by_slot.clear();
        battle.columns.look_by_slot.clear();
        battle.output = output;
        timings.total_us = start.map_or(0, |start| start.elapsed().as_micros() as u64);
        battle.timings = timings;
        state.hostile.battle = battle;
        return Vec::new();
    }
    let frame = Frame {
        entities: &state.entities,
        registry: &state.registry,
        world: &state.world,
        tick: battle.elapsed_ticks,
    };
    if state.tickets.contains_key(staging::OWNER) {
        staging::update_orders(&mut battle.formation);
    }
    battle.columns.prepare(
        &frame,
        &mut battle.formation,
        &mut output.counters,
        &mut battle.population,
    );
    timings.columns_us = phase_us(&mut mark);
    battle.formation.finish_preparation();
    timings.formation_us = phase_us(&mut mark);
    targeting::update(&frame, &mut battle);
    timings.targeting_us = phase_us(&mut mark);
    steering::step(&frame, &mut battle, &mut output);
    timings.steering_us = phase_us(&mut mark);
    combat::step(&frame, &mut battle, &mut output);
    timings.combat_us = phase_us(&mut mark);
    morale::step(&frame, &mut battle, &mut output);
    timings.morale_us = phase_us(&mut mark);
    battle.columns.project();
    timings.projection_us = phase_us(&mut mark);
    #[cfg(test)]
    if battle.trace.is_some() {
        battle.trace = Some(trace_tests::frame_digest(&battle, &output));
    }
    battle.elapsed_ticks += 1;
    state.hostile.events.append(&mut output.events);
    for (id, raised) in output.shields.drain(..) {
        show_shield(state, id, raised);
    }
    for arrow in output.arrows.drain(..) {
        crate::hostile::launch_arrow(state, arrow.owner, arrow.from, arrow.velocity, arrow.damage);
    }
    let intents = output
        .intents
        .drain(..)
        .map(|raw| Intent {
            actor: raw.actor,
            seq: state.next_seq(raw.actor),
            kind: raw.kind,
            footprint: raw.footprint,
        })
        .collect();
    timings.emit_us = phase_us(&mut mark);
    timings.total_us = start.map_or(0, |start| start.elapsed().as_micros() as u64);
    battle.timings = timings;
    battle.output = output;
    state.hostile.battle = battle;
    intents
}

/// Vanilla `LivingEntity.DATA_LIVING_ENTITY_FLAGS` (javap 26.3): index 8, a
/// byte where 1 is `isUsingItem` and 2 selects the off hand. A raised battle
/// shield is the off-hand use a blocking mob or player shows; native
/// appearance custody journals and publishes it like any metadata change.
const LIVING_FLAGS: u8 = 8;
const USING_OFF_HAND: i8 = 0b11;

fn show_shield(state: &mut TickState, id: EntityId, raised: bool) {
    let Some(slot) = state.entities.slot_of(id) else {
        return;
    };
    let flags = if raised { USING_OFF_HAND } else { 0 };
    let shown = state
        .entities
        .row(slot)
        .appearance
        .extra
        .iter()
        .find_map(|(index, value)| match (*index, value) {
            (LIVING_FLAGS, crate::boundary::EntityDataValue::Byte(flags)) => Some(*flags),
            _ => None,
        })
        .unwrap_or(0);
    if shown == flags {
        return;
    }
    let extra = &mut std::sync::Arc::make_mut(&mut state.entities.row_mut(slot).appearance).extra;
    let value = crate::boundary::EntityDataValue::Byte(flags);
    match extra.iter_mut().find(|(index, _)| *index == LIVING_FLAGS) {
        Some((_, current)) => *current = value,
        None => extra.push((LIVING_FLAGS, value)),
    }
    state.appearance_changed.push(id);
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod trace_tests;

#[cfg(test)]
mod ranks_tests;
