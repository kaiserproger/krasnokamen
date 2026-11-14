//! Ordered battle kernels over frozen native entity rows. Physics and intent
//! merge stay in the ordinary tick owners; battle policy lives in this module.

pub(crate) mod archery;
pub(crate) mod cavalry;
pub(crate) mod combat;
pub(crate) mod formation;
pub(crate) mod morale;
pub(crate) mod steering;
mod faction;
mod targeting;

pub use faction::{enemies, retained_enemy};
pub(crate) use faction::special;
pub use targeting::{
    CELL_BLOCKS, EnemyTarget, MELEE_ENGAGEMENT_REACH, Query, RETARGET_TICKS,
    TargetGrid, Unit, Work, engagement_reach, retarget_due,
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
}

pub(crate) struct BattleIntent {
    pub actor: EntityId,
    pub kind: IntentKind,
    pub footprint: Footprint,
}

pub(crate) struct ArrowLaunch {
    pub owner: EntityId,
    pub from: [f64; 3],
    /// Native projectile convention: blocks/tick, unlike steering.
    pub velocity: [f64; 3],
    pub damage: f32,
}

#[derive(Default)]
pub(crate) struct Output {
    pub intents: Vec<BattleIntent>,
    pub events: Vec<EntityEvent>,
    pub arrows: Vec<ArrowLaunch>,
}

/// The only battle tick call. Ordered kernels read a frozen world/entity store
/// and write only battle runtime + private output; native effects apply after
/// the frozen borrow ends. Spawned arrows enter ordinary projectile simulation.
pub(crate) fn step(state: &mut TickState) -> Vec<Intent> {
    let mut battle = std::mem::take(&mut state.hostile.battle);
    battle.work = Work::default();
    if !battle.running {
        battle.steering.clear();
        state.hostile.battle = battle;
        return Vec::new();
    }
    let frame = Frame {
        entities: &state.entities,
        registry: &state.registry,
        world: &state.world,
        tick: battle.elapsed_ticks,
    };
    let mut output = Output::default();
    formation::update(&frame, &mut battle);
    targeting::update(&frame, &mut battle);
    steering::step(&frame, &mut battle, &mut output);
    combat::step(&frame, &mut battle, &mut output);
    morale::step(&frame, &mut battle, &mut output);
    battle.elapsed_ticks += 1;
    state.hostile.battle = battle;
    state.hostile.events.extend(output.events);
    for arrow in output.arrows {
        crate::hostile::launch_arrow(state, arrow.owner, arrow.from, arrow.velocity, arrow.damage);
    }
    output.intents.into_iter().map(|raw| {
        let seq = state.seq_counter.entry(raw.actor).or_default();
        let intent = Intent { actor: raw.actor, seq: *seq, kind: raw.kind, footprint: raw.footprint };
        *seq += 1;
        intent
    }).collect()
}

#[cfg(test)]
mod tests;
