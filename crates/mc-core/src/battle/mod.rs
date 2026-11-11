//! Ordered battle kernels over frozen native entity rows. Physics and intent
//! merge stay in the ordinary tick owners; battle policy lives in this module.

pub(crate) mod archery;
pub(crate) mod cavalry;
pub(crate) mod combat;
mod faction;
pub(crate) mod formation;
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
use rustc_hash::FxHashMap;

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
    pub targets: FxHashMap<EntityId, EnemyTarget>,
    /// Desired velocity in blocks/second, consumed by ordinary row physics.
    pub steering: FxHashMap<EntityId, [f64; 3]>,
    grid: TargetGrid,
    pub work: Work,
    output: Output,
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

/// Population observed at the battle hook, before motion and damage merge.
/// `units` includes resident faction mobs (also horses and dead retained rows).
/// Alive counts require both body liveness and positive health.
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
    fn sample(&mut self, entities: &EntityStore, tick: u64) {
        self.tick = tick;
        self.units = 0;
        self.alive_per_faction.clear();
        for slot in entities.battle_slots() {
            let body = entities.body(slot);
            if !matches!(body.kind, crate::entities::EntityKind::Mob { .. }) {
                continue;
            }
            self.units += 1;
            let alive = u64::from(body.alive && entities.row(slot).health > 0.0);
            if let Some(count) = self
                .alive_per_faction
                .iter_mut()
                .find(|c| c.faction == body.faction)
            {
                count.alive += alive;
            } else {
                self.alive_per_faction.push(FactionAlive {
                    faction: body.faction,
                    alive,
                });
            }
        }
        self.alive_per_faction
            .sort_unstable_by_key(|count| count.faction);
    }
}

/// Last pre-motion sample, keyed by global server tick, including paused battles.
/// A frozen global tick does not call the hook; the returned stamp stays old.
pub fn unit_counters(hostile: &crate::hostile::HostileState) -> &UnitCounters {
    &hostile.battle.output.counters
}

#[derive(Debug, Default)]
pub(crate) struct Output {
    pub intents: Vec<BattleIntent>,
    pub events: Vec<EntityEvent>,
    pub arrows: Vec<ArrowLaunch>,
    pub counters: UnitCounters,
}

/// The only battle tick call. Ordered kernels read a frozen world/entity store
/// and write only battle runtime + private output; native effects apply after
/// the frozen borrow ends. Spawned arrows enter ordinary projectile simulation.
pub(crate) fn step(state: &mut TickState) -> Vec<Intent> {
    let mut battle = std::mem::take(&mut state.hostile.battle);
    battle.work = Work::default();
    let mut output = std::mem::take(&mut battle.output);
    output.counters.sample(&state.entities, state.tick);
    if !battle.running {
        battle.steering.clear();
        battle.output = output;
        state.hostile.battle = battle;
        return Vec::new();
    }
    let frame = Frame {
        entities: &state.entities,
        registry: &state.registry,
        world: &state.world,
        tick: battle.elapsed_ticks,
    };
    formation::update(&frame, &mut battle);
    targeting::update(&frame, &mut battle);
    steering::step(&frame, &mut battle, &mut output);
    combat::step(&frame, &mut battle, &mut output);
    morale::step(&frame, &mut battle, &mut output);
    battle.elapsed_ticks += 1;
    state.hostile.events.append(&mut output.events);
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
    battle.output = output;
    state.hostile.battle = battle;
    intents
}

#[cfg(test)]
mod tests;
