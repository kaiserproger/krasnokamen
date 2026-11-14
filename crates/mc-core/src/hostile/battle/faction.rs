use crate::boundary::EntityId;
use crate::entities::{EntityKind, EntitySlot, EntityStore};

use super::{EnemyTarget, Unit};

/// Zero is unaffiliated, not an enemy army.
pub fn enemies(a: u16, b: u16) -> bool {
    a != 0 && b != 0 && a != b
}

pub(crate) fn live_unit(entities: &EntityStore, slot: EntitySlot) -> Option<Unit> {
    let body = entities.body(slot);
    (body.faction != 0
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

/// Battle actions own their own timers and output. No vanilla species-specific
/// targeting or bow pass runs alongside them.
pub(crate) fn special(
    faction: u16,
    special: Option<crate::hostile::Special>,
) -> Option<crate::hostile::Special> {
    special.filter(|_| faction == 0)
}
