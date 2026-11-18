//! Vanilla `LivingEntity.tickDeath` for non-player living bodies (26.3 javap):
//! the dead body stays for 20 simulated ticks, so clients play the death tilt,
//! then the server broadcasts entity event 60 (poof) and removes it. Loot and
//! experience drop once at death (`die` -> `dropAllDeathLoot`); removal drops
//! nothing. A corpse is `alive == false`: it runs no AI and takes no damage,
//! interaction, targeting, pushing or pickup; it moves only by the physics
//! below. Its removal is durable at death (`EntityStore::begin_dying`).

use super::*;

/// `LivingEntity.tickDeath` removes the body once `deathTime` reaches 20.
const DEATH_TICKS: u8 = 20;
/// `Level.broadcastEntityEvent(this, (byte) 60)` right before that removal.
const POOF_EVENT: i8 = 60;

/// Whether a dead non-player row keeps its body as a corpse. Non-living
/// entities (vehicles, frames, paintings, crystals) and armor stands, which
/// vanilla discards when broken, leave the store at once.
pub(super) fn keeps_corpse(state: &TickState, slot: EntitySlot) -> bool {
    let EntityKind::Mob { type_id } = state.entities.body(slot).kind else {
        return false;
    };
    !crate::armor_stand::is_stand(state, slot)
        && mc_data::entity_types::entity_type_contract_26_1_2_by_protocol_id(
            crate::blueprint::wire_type_id(type_id),
        )
        .is_some_and(|contract| {
            contract.behavior.archetype == mc_data::entity_types::EntityArchetype::Living
        })
}

/// One simulated tick of every corpse: expired bodies poof and leave the
/// store; the rest fall and slide without AI input.
pub(super) fn tick(state: &mut TickState, changes: &mut ChangeSet) {
    let mut after = None;
    while let Some(id) = state.entities.next_dying(after) {
        after = Some(id);
        let slot = state
            .entities
            .slot_of_any(id)
            .expect("corpse clock follows its row");
        if state.entities.tick_dying(id) >= DEATH_TICKS {
            changes.entity_events.push(EntityEvent::Status {
                entity: id,
                event: POOF_EVENT,
            });
            changes.entities_removed.push(id);
            state.remove_entity(slot);
        } else {
            fall(state, slot);
        }
    }
}

/// Vanilla `isImmobile` zeroes the dead body's inputs, but `travel` still
/// applies gravity, fluids and the lethal knockback. Facing is kept.
fn fall(state: &mut TickState, slot: EntitySlot) {
    let body = state.entities.body(slot);
    let row = state.entities.row(slot);
    let pos = state.entities.pos(slot);
    let vel = state.entities.vel(slot);
    let rotation = state.entities.rot(slot);
    let aquatic_control = match body.kind {
        EntityKind::Mob { type_id } => state
            .registry
            .blueprint(type_id)
            .map_or(crate::species::AquaticControl::Other, |blueprint| {
                blueprint.capabilities.aquatic
            }),
        _ => crate::species::AquaticControl::Other,
    };
    let reader = state.world.physics_reader();
    let sampler = WorkerSampler {
        block_at: &|x, y, z| reader.block_at(x, y, z),
        registry: &state.registry,
    };
    let result = crate::entity_motion::step(
        crate::entity_motion::MotionInput {
            body: mc_physics::EntityBody {
                position: mc_physics::Vec3::new(pos[0], pos[1], pos[2]),
                velocity: mc_physics::Vec3::new(vel[0], vel[1], vel[2]),
                aabb: mc_physics::Aabb {
                    half_width: body.half_width,
                    height: body.height,
                },
                on_ground: state.entities.ground(slot),
            },
            movement: body.movement,
            drive: [0.0; 3],
            impulse: state.entities.horizontal_impulse(slot),
            rotation,
            look_target: None,
            battle_facing: false,
            legacy_facing: false,
            head_follows_body: false,
            climbs_walls: false,
            aquatic_control,
            air_supply: body.air_supply,
            levitation: row
                .active_effects
                .level(crate::effects::EffectId::Levitation),
            slow_falling: row
                .active_effects
                .has(crate::effects::EffectId::SlowFalling),
            no_gravity: row.controls.no_gravity,
            id: body.id.0,
            tick: state.tick,
        },
        &sampler,
        |x, z| state.world.has_chunk(x.div_euclid(16), z.div_euclid(16)),
    );
    let motion = crate::entities::MotionResult {
        pos: [
            result.body.position.x,
            result.body.position.y,
            result.body.position.z,
        ],
        vel: [
            result.body.velocity.x,
            result.body.velocity.y,
            result.body.velocity.z,
        ],
        yaw: rotation[0],
        pitch: rotation[1],
        head_yaw: rotation[2],
        ground: result.body.on_ground,
        horizontal_impulse: result.impulse,
        air_supply: body.air_supply,
        attack_cooldown: None,
    };
    state.entities.apply_motion_result(slot, &motion);
}

#[cfg(test)]
#[path = "dying_tests.rs"]
mod tests;
