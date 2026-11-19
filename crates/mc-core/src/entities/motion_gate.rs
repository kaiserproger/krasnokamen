//! Native motion gate: which motion lanes one entity runs this tick.
//!
//! Derived, non-authoritative inputs for the entity phase. The store owns the
//! `flags`/`hints` columns and their custody; this owner owns the classification
//! policy that fills them from hot body state, cold retained state and the
//! blueprint registry. Bytes moved verbatim from `entities.rs`.

use super::{
    Body, ColdRow, EntityKind, EntitySlot, EntityStore, MOTION_ALIVE, MOTION_ARMORED,
    MOTION_EFFECTS, MOTION_EGG, MOTION_ITEM_COLLECTOR, MOTION_LIFE, MOTION_OWNED,
    MOTION_PHASE_SHIFT, MOTION_PLAYER, MOTION_QUIET_TIMERS, MOTION_REST, MOTION_SPECIAL,
    MOTION_TARGETLESS_WANDER_REST, MOTION_WANDER_REST, PICKUP_DELAY_NEVER,
};
use crate::blueprint::MovementClass;

/// Exact tick native-motion gate shared with the entity phase. Corpses move
/// only through `tick::dying` (vanilla dead bodies run no AI).
fn motion_owned(body: &Body, row: &ColdRow) -> bool {
    (body.alive || matches!(body.kind, EntityKind::Player { .. }))
        && !crate::special_mobs::raiders::owns(body.kind)
        && row.falling.is_none()
        && row.tnt.is_none()
        && row.entity_life.vehicle.is_none()
        && !row
            .villager
            .as_ref()
            .is_some_and(|v| !v.wandering && v.conversion_time.is_none())
        && !matches!(body.kind, EntityKind::ExperienceOrb { .. })
}

/// Blueprint special visible to motion gating: living mobs without NoAI only
/// (a corpse runs no AI).
pub(super) fn effective_special(
    registry: &crate::registry::CoreRegistry,
    body: &Body,
    row: &ColdRow,
) -> Option<crate::hostile::Special> {
    let EntityKind::Mob { type_id } = body.kind else {
        return None;
    };
    if !body.alive || row.controls.no_ai {
        return None;
    }
    registry
        .blueprint(type_id)
        .and_then(crate::hostile::special_of)
}

fn motion_rest_base(
    registry: &crate::registry::CoreRegistry,
    entities: &EntityStore,
    body: &Body,
    row: &ColdRow,
    impulse: [f64; 2],
    special: Option<crate::hostile::Special>,
) -> bool {
    if !body.alive
        || !matches!(body.kind, EntityKind::Mob { .. })
        || body.movement != MovementClass::Walker
        || impulse != [0.0; 2]
        || row.controls.no_ai
        || row.special_mob.is_some()
        || row.love != 0
        || body.burn_ticks != 0
        || crate::taming::suppress_ai(entities, row)
    {
        return false;
    }
    let EntityKind::Mob { type_id } = body.kind else {
        return false;
    };
    let Some(blueprint) = registry.blueprint(type_id) else {
        return false;
    };
    if blueprint.goals.is_empty()
        || blueprint.name == "minecraft:creaking"
        || !matches!(
            special,
            None | Some(crate::hostile::Special::Ranged) | Some(crate::hostile::Special::Swell)
        )
    {
        return false;
    }
    true
}

fn motion_effects(body: &Body, row: &ColdRow) -> bool {
    if matches!(body.kind, EntityKind::Player { .. }) {
        return false;
    }
    !row.active_effects.entries.is_empty()
        || !row.appearance.effect_particles.is_empty()
        || row.appearance.absorption > 0.0
        || row.appearance.shared_flags & 0x60 != 0
}

fn motion_life(body: &Body, row: &ColdRow, tadpole: Option<u32>) -> bool {
    let EntityKind::Mob { type_id } = body.kind else {
        return row.appearance.arrow_count > 0 || row.appearance.stinger_count > 0;
    };
    if !body.alive {
        return row.appearance.arrow_count > 0 || row.appearance.stinger_count > 0;
    }
    let mob_clock = row.breed_cooldown > 0
        || row.love > 0
        || (!row.entity_life.age_lock.locked
            && (row.entity_life.child_ticks > 0 || Some(type_id) == tadpole));
    mob_clock || row.appearance.arrow_count > 0 || row.appearance.stinger_count > 0
}

pub(super) fn motion_flags_for(
    registry: &crate::registry::CoreRegistry,
    entities: &EntityStore,
    slot: EntitySlot,
    special: Option<crate::hostile::Special>,
    chicken: Option<u32>,
    tadpole: Option<u32>,
) -> u16 {
    let body = entities.body(slot);
    let row = entities.row(slot);
    let impulse = entities.impulse[slot.0 as usize];
    let phase = ((body.id.0 as u64 & 3) as u16) << MOTION_PHASE_SHIFT;
    let mut flags: u16 = phase;
    if motion_owned(body, row) {
        flags |= MOTION_OWNED;
    }
    if matches!(body.kind, EntityKind::Player { .. }) {
        flags |= MOTION_PLAYER;
    }
    if body.alive {
        flags |= MOTION_ALIVE;
    }
    if special.is_some() {
        flags |= MOTION_SPECIAL;
    }
    if motion_rest_base(registry, entities, body, row, impulse, special) {
        flags |= MOTION_REST;
        // No alternate movement goal, standing override, breeding or panic.
        if let EntityKind::Mob { type_id } = body.kind
            && let Some(blueprint) = registry.blueprint(type_id)
            && !matches!(
                crate::piglin::name(type_id),
                "minecraft:hoglin" | "minecraft:piglin"
            )
            && !crate::dolphin::is_dolphin(type_id)
        {
            let chaser = blueprint.has_goal(crate::blueprint::GoalKind::MeleeChase)
                || matches!(
                    special,
                    Some(crate::hostile::Special::Ranged | crate::hostile::Special::Swell)
                );
            if !chaser {
                flags |= MOTION_WANDER_REST;
            } else if !blueprint.capabilities.hunts_monsters && !blueprint.capabilities.hunts_living
            {
                // Admission checks every applicable target table: players,
                // and also residents for zombie/zombie-villager chasers.
                flags |= MOTION_TARGETLESS_WANDER_REST;
            }
        }
    }
    if motion_effects(body, row) {
        flags |= MOTION_EFFECTS;
    }
    if motion_life(body, row, tadpole) {
        flags |= MOTION_LIFE;
    }
    if body.alive
        && matches!(body.kind, EntityKind::Mob { type_id } if Some(type_id) == chicken)
        && crate::entity_life::adult(row)
    {
        flags |= MOTION_EGG;
    }
    if body.alive
        && !row.controls.no_ai
        && row.loot.can_pick_up_loot
        && matches!(body.kind, EntityKind::Mob { type_id }
            if registry.blueprint(type_id).is_some_and(|bp| bp.capabilities.equipment_pickup))
    {
        flags |= MOTION_ITEM_COLLECTOR;
    }
    if !matches!(body.kind, EntityKind::Player { .. })
        && row.appearance.equipment[2..6].iter().any(Option::is_some)
    {
        flags |= MOTION_ARMORED;
    }
    if matches!(body.pickup_delay, 0 | PICKUP_DELAY_NEVER)
        && body.attack_cooldown == 0
        && impulse == [0.0; 2]
    {
        flags |= MOTION_QUIET_TIMERS;
    }
    flags
}
