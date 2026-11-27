use crate::entities::{EntityKind, EntitySlot, EntityStore};
use mc_domain::EntityId;

use mc_melee_range::{Aabb, MeleeReach, attack_box};

use super::{EnemyTarget, Unit};

/// Allegiance is independent of whether battle AI owns a native row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Relation {
    Same,
    Enemy,
    Neutral,
}

pub(crate) fn relation(a: u16, b: u16) -> Relation {
    if a == 0 || b == 0 {
        Relation::Neutral
    } else if a == b {
        Relation::Same
    } else {
        Relation::Enemy
    }
}

/// Roster/AI ownership marker, not a relation to another faction.
/// Today nonzero factions enroll a row; keep that policy separate from
/// allegiance so settlement membership need not acquire battle AI.
pub(crate) fn battle_row(faction: u16) -> bool {
    faction != 0
}

/// Zero is unaffiliated, not an enemy army.
pub fn enemies(a: u16, b: u16) -> bool {
    relation(a, b) == Relation::Enemy
}

pub(crate) fn live_unit(entities: &EntityStore, slot: EntitySlot) -> Option<Unit> {
    let body = entities.body(slot);
    (battle_row(body.faction)
        && body.alive
        && matches!(body.kind, EntityKind::Mob { .. })
        && entities.row(slot).health > 0.0
        && !crate::axolotl::playing_dead(entities.row(slot)))
    .then(|| Unit {
        id: body.id,
        faction: body.faction,
        pos: entities.pos(slot),
    })
}

/// O(1) validation between sliced decisions. Consumers recheck after movement
/// and damage: dead, re-factioned and out-of-reach bodies are never attackable
/// from a stale frozen snapshot.
pub fn retained_enemy(
    entities: &EntityStore,
    actor: EntitySlot,
    target: EntityId,
    reach: f64,
) -> Option<EnemyTarget> {
    let unit = live_unit(entities, actor)?;
    let enemy = live_unit(entities, entities.slot_of(target)?)?;
    if !enemies(unit.faction, enemy.faction) {
        return None;
    }
    let distance = crate::target_grid::distance(unit.pos, enemy.pos);
    (distance <= reach).then_some(EnemyTarget {
        id: enemy.id,
        pos: enemy.pos,
        distance,
    })
}

/// Melee reach of a fighter holding an item with `range` (its vanilla
/// `minecraft:attack_range`), as vanilla 26.3 sets a mob's: see
/// [`mc_melee_range::MeleeReach::mob`].
pub fn melee_reach(range: Option<mc_data::item_components::AttackRangeFacts>) -> MeleeReach {
    MeleeReach::mob(range.map(|range| (range.min_reach, range.max_reach, range.mob_factor)))
}

/// Feet-to-feet distance beyond which no two battle boxes (horse-wide at
/// most, a rider above his horse) can touch within `reach`: the prefilter for
/// distance lookups before the box test.
pub fn reach_bound(reach: MeleeReach) -> f64 {
    (reach.max + 2.0 * MAX_BOX_HALF_WIDTH) * std::f64::consts::SQRT_2 + 2.5
}

/// Widest battle body (a horse) for [`reach_bound`].
const MAX_BOX_HALF_WIDTH: f64 = 0.75;

fn body_box(entities: &EntityStore, slot: EntitySlot) -> Aabb {
    let body = entities.body(slot);
    Aabb::at(entities.pos(slot), 2.0 * body.half_width, body.height)
}

/// Whether `target` is within the melee range of `actor`: vanilla
/// `Mob.isWithinMeleeAttackRange` ([`mc_melee_range::in_melee_range`]) on
/// his box joined with his mount's and the target's own box.
pub fn in_melee_range(
    entities: &EntityStore,
    actor: EntitySlot,
    target: EntitySlot,
    reach: MeleeReach,
) -> bool {
    let mount = entities
        .row(actor)
        .entity_life
        .mount
        .vehicle
        .and_then(|mount| entities.slot_of(mount))
        .map(|mount| body_box(entities, mount));
    mc_melee_range::in_melee_range(
        attack_box(body_box(entities, actor), mount),
        body_box(entities, target),
        reach,
    )
}

/// Battle actions own their own timers and output. No vanilla species-specific
/// targeting or bow pass runs alongside them.
pub(crate) fn special(
    faction: u16,
    special: Option<crate::hostile::Special>,
) -> Option<crate::hostile::Special> {
    special.filter(|_| !battle_row(faction))
}

#[cfg(test)]
#[path = "faction_tests.rs"]
mod tests;
