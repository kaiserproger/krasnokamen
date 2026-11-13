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

/// A fighter's melee reach as vanilla 26.3 judges a mob's (local `javap -c` of
/// server-26.3.jar, `Mob.isWithinMeleeAttackRange` and `AttackRange`): his box,
/// joined sideways with his mount's, grown horizontally by `reach` must touch
/// the target's box; with a positive `min`, the box grown by `min` must not.
/// The held item's `minecraft:attack_range` gives `reach = max_reach *
/// mob_factor` and `min = min_reach * mob_factor` (`effectiveMaxRange` and
/// `effectiveMinRange` for a non-player; the mob check uses no hitbox margin);
/// without the component the mob reach is `Mob.DEFAULT_ATTACK_REACH`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeleeRange {
    pub reach: f64,
    pub min: f64,
}

impl MeleeRange {
    /// `Mob.DEFAULT_ATTACK_REACH`: sqrt(2.04) - 0.6 with both float constants
    /// widened to double (2.0399999618530273, 0.6000000238418579).
    pub const DEFAULT: Self = Self {
        reach: 1.428_285_672_354_458_4 - 0.600_000_023_841_857_9,
        min: 0.0,
    };

    pub fn of_item(range: Option<mc_data::item_components::AttackRangeFacts>) -> Self {
        range.map_or(Self::DEFAULT, |range| Self {
            reach: f64::from(range.max_reach * range.mob_factor),
            min: f64::from(range.min_reach * range.mob_factor),
        })
    }

    /// Feet-to-feet distance beyond which no two battle boxes (horse-wide at
    /// most, a block apart in height) can touch: the prefilter for distance
    /// lookups before the box test.
    pub fn bound(self) -> f64 {
        (self.reach + 2.0 * MAX_BOX_HALF_WIDTH) * std::f64::consts::SQRT_2 + 2.5
    }
}

/// Widest battle body (a horse) for [`MeleeRange::bound`].
const MAX_BOX_HALF_WIDTH: f64 = 0.75;

fn body_box(entities: &EntityStore, slot: EntitySlot) -> ([f64; 3], [f64; 3]) {
    let body = entities.body(slot);
    let p = entities.pos(slot);
    (
        [p[0] - body.half_width, p[1], p[2] - body.half_width],
        [p[0] + body.half_width, p[1] + body.height, p[2] + body.half_width],
    )
}

/// Whether `target` is within the melee range of `actor` ([`MeleeRange`]).
pub fn in_melee_range(
    entities: &EntityStore,
    actor: EntitySlot,
    target: EntitySlot,
    range: MeleeRange,
) -> bool {
    let (mut lo, mut hi) = body_box(entities, actor);
    if let Some(mount) = entities.row(actor).entity_life.mount.vehicle
        && let Some(mount) = entities.slot_of(mount)
    {
        let (mount_lo, mount_hi) = body_box(entities, mount);
        for axis in [0, 2] {
            lo[axis] = lo[axis].min(mount_lo[axis]);
            hi[axis] = hi[axis].max(mount_hi[axis]);
        }
    }
    let (t_lo, t_hi) = body_box(entities, target);
    let touches = |grow: f64| {
        lo[0] - grow < t_hi[0]
            && hi[0] + grow > t_lo[0]
            && lo[1] < t_hi[1]
            && hi[1] > t_lo[1]
            && lo[2] - grow < t_hi[2]
            && hi[2] + grow > t_lo[2]
    };
    touches(range.reach) && (range.min <= 0.0 || !touches(range.min))
}

/// Battle actions own their own timers and output. No vanilla species-specific
/// targeting or bow pass runs alongside them.
pub(crate) fn special(
    faction: u16,
    special: Option<crate::hostile::Special>,
) -> Option<crate::hostile::Special> {
    special.filter(|_| faction == 0)
}
