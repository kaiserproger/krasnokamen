//! Vanilla `Entity.checkFallDamage` for mobs: a falling body accumulates its
//! drop, and the landing move owes `ceil(distance - 3)` fall damage.
//!
//! Called from the serial movement writeback with the pre/post positions, so
//! walkers on level ground (no drop, no ground transition) never touch the
//! cold row. The accumulated distance lives in `Controls::fall_distance`
//! (already persisted as `FallDistance`); growth is written without a durable
//! mark because a transient mid-air value is not worth a WAL image per tick.
//! Clearing a non-zero distance takes journal custody: an image emitted
//! mid-air holds the distance, and replay must not keep it after a landing
//! (`Controls::journal_eq`).

use crate::blueprint::MovementClass;
use crate::effects::EffectId;
use crate::entities::{EntityKind, EntitySlot};
use crate::tick::TickState;

/// A pointed upper tip handles landing before the ordinary block fall path.
pub(crate) fn stalagmite_at(state: &TickState, position: [f64; 3]) -> bool {
    state
        .world
        .block_at(
            position[0].floor() as i32,
            (position[1] - 0.01).floor() as i32,
            position[2].floor() as i32,
        )
        .is_some_and(|id| state.registry.states.stalagmite_tips.contains(&id))
}

/// Account one mob move; returns the fall damage owed on a landing.
///
/// `was_ground` is the support flag before the move, `ground` after it.
pub(crate) fn after_move(
    state: &mut TickState,
    slot: EntitySlot,
    old: [f64; 3],
    new: [f64; 3],
    was_ground: bool,
    ground: bool,
    wall_climbing: bool,
) -> (f32, bool) {
    // Vanilla LivingEntity.handleOnClimbable resets fall distance. A clipped
    // climb still counts: otherwise tiny ceiling drops accumulate across rises.
    if wall_climbing {
        reset(state, slot);
        return (0.0, false);
    }
    // Vanilla LivingEntity.rideTick clears fall distance even without a drop.
    if state.entities.row(slot).entity_life.mount.vehicle.is_some() {
        reset(state, slot);
        return (0.0, false);
    }
    let dy = new[1] - old[1];
    if (was_ground || !ground) && dy >= 0.0 {
        return (0.0, false);
    }
    let body = state.entities.body(slot);
    let EntityKind::Mob { type_id } = body.kind else {
        return (0.0, false);
    };
    if matches!(
        body.movement,
        MovementClass::Flyer | MovementClass::Swimmer | MovementClass::Stationary
    ) || state
        .registry
        .blueprint(type_id)
        .is_some_and(|blueprint| blueprint.capabilities.fall_damage_immune)
    {
        return (0.0, false);
    }
    // Vanilla LivingEntity.rideTick resets a passenger's fall distance every
    // tick: the vehicle positions it, so a snap back to the seat is no fall.
    if state.entities.row(slot).entity_life.mount.vehicle.is_some() {
        reset(state, slot);
        return (0.0, false);
    }
    let feet = [
        new[0].floor() as i32,
        new[1].floor() as i32,
        new[2].floor() as i32,
    ];
    let cushioned = state
        .world
        .block_at(feet[0], feet[1], feet[2])
        .is_some_and(|id| {
            id == state.registry.states.water
                || state
                    .registry
                    .blocks
                    .by_id(mc_world::BlockStateId(id))
                    .is_some_and(|block| crate::statistics::climbable(state, block.block.raw_id))
        });
    let stalagmite = ground && stalagmite_at(state, new);
    let row = state.entities.row(slot);
    if cushioned
        || row.active_effects.has(EffectId::SlowFalling)
        || row.active_effects.has(EffectId::Levitation)
    {
        reset(state, slot);
        return (0.0, false);
    }
    let drop = if dy < 0.0 { f64::from(dy as f32) } else { 0.0 };
    if !ground {
        if drop != 0.0 {
            state
                .entities
                .row_mut_projected(slot)
                .controls
                .fall_distance -= drop;
        }
        return (0.0, false);
    }
    // The landing move's own drop counts, then the distance is spent.
    let distance = reset(state, slot) - drop;
    (
        if stalagmite {
            mc_survival::stalagmite_damage(distance)
        } else {
            mc_survival::fall_damage(distance)
        },
        stalagmite,
    )
}

/// Clear the fall distance; returns the distance it held. Only a non-zero
/// distance is written, through journaled custody (see the module docs).
fn reset(state: &mut TickState, slot: EntitySlot) -> f64 {
    let distance = state.entities.row(slot).controls.fall_distance;
    if distance != 0.0 {
        state.entities.row_mut(slot).controls.fall_distance = 0.0;
    }
    distance
}
