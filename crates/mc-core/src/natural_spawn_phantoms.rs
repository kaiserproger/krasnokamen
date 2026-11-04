//! Vanilla insomnia-driven phantom attempts (26.1.2 PhantomSpawner).
use super::{Rng, observe_slot};
use crate::boundary::{ChangeSet, EntityId};
use crate::tick::TickState;

pub(super) fn run(
    state: &mut TickState,
    observers: &[(EntityId, [f64; 3])],
    rng: &mut Rng,
    changes: &mut ChangeSet,
) {
    if state.dimension != crate::dimensions::Dimension::Overworld
        || !crate::command_rules::boolean(state, crate::command_rules::BoolRule::SPAWN_PHANTOMS)
        || state.tick < state.spawn_census.phantom_next_tick
    {
        return;
    }
    state.spawn_census.phantom_next_tick = state.tick + (60 + rng.below(60)) * 20;
    if !super::is_night(state.day_time) && state.rain_level * state.thunder_level <= 0.9 {
        return;
    }
    let Some(type_id) = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse("minecraft:phantom").expect("phantom identifier"))
    else {
        return;
    };
    let Some(blueprint) = state.registry.blueprint(type_id) else {
        return;
    };
    let health = blueprint.params.max_health;
    for &(id, pos) in observers {
        if pos[1].floor() < f64::from(state.dimension.sea_level())
            || !crate::conduit::sky_exposed(state, pos)
            || rng.unit() * 3.0 >= crate::mob_equipment::effective_at(state, pos)
        {
            continue;
        }
        let rest = state.players[&id]
            .statistics
            .get_custom(mc_data::statistics::CustomStat::TimeSinceRest)
            .max(1) as u64;
        if rng.below(rest) < 72_000 {
            continue;
        }
        let spawn = [
            pos[0].floor() + rng.below(21) as f64 - 10.0,
            pos[1].floor() + 20.0 + rng.below(15) as f64,
            pos[2].floor() + rng.below(21) as f64 - 10.0,
        ];
        let [x, y, z] = spawn.map(|v| v as i32);
        if !state.world.is_active(x.div_euclid(16), z.div_euclid(16))
            || !state.world.block_at(x, y, z).is_some_and(|block| {
                state.registry.materials.classify(block) == mc_physics::BlockMaterial::Air
            })
        {
            continue;
        }
        let count = 1 + rng.below(u64::from(state.difficulty.id()) + 1);
        for _ in 0..count {
            let entity = state.spawn_mob(type_id, spawn, health);
            let slot = state.entities.slot_of(entity).expect("new phantom");
            observe_slot(state, slot);
            changes.entities_spawned.push(entity);
        }
    }
}
