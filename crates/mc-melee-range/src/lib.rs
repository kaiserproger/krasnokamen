//! Whether a mob can strike a target in melee: vanilla 26.3 `Mob.isWithinMeleeAttackRange` with
//! `Mob.getAttackBoundingBox` and the non-player branch of `AttackRange.effectiveMinRange` /
//! `effectiveMaxRange`. Pure geometry with no dependencies, so the server and the replay scanner
//! judge contact with the same code.
//!
//! Source: `javap -c -p -constants` of `net.minecraft.world.entity.Mob` and
//! `net.minecraft.world.item.component.AttackRange` from `minecraft-merged-deobf-26.3.jar`:
//!
//! * `Mob.isWithinMeleeAttackRange(target)`: with the held item's `minecraft:attack_range`
//!   (`getActiveItem().get(ATTACK_RANGE)`) null, `max = DEFAULT_ATTACK_REACH`, `min = 0`; else
//!   `max = effectiveMaxRange(this)`, `min = effectiveMinRange(this)`. The result is
//!   `getAttackBoundingBox(max).intersects(target.getHitbox())` and
//!   `(min <= 0 || !getAttackBoundingBox(min).intersects(target.getHitbox()))`.
//! * `Mob.getAttackBoundingBox(d)`: the own bounding box, joined with the vehicle's bounding box
//!   by `min`/`max` of x and z only (y stays the mob's own) when riding, then `inflate(d, 0, d)`.
//! * `AttackRange.effectiveMaxRange` / `effectiveMinRange` for a non-player: `maxReach * mobFactor`
//!   and `minReach * mobFactor`, `float` products (hitbox margin is not used by the mob check).
//! * `Mob.DEFAULT_ATTACK_REACH`: `Math.sqrt(2.0399999618530273d) - 0.6000000238418579d`
//!   (the float constants `2.04f` and `0.6f` widened to double).
//! * `AABB.intersects`: strict (`min < other.max && max > other.min` on every axis).

/// Axis-aligned box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl Aabb {
    /// Box of an entity standing at `position` (feet) with hitbox `width` x `height`.
    pub fn at(position: [f64; 3], width: f64, height: f64) -> Self {
        let half = width / 2.0;
        Self {
            min: [position[0] - half, position[1], position[2] - half],
            max: [position[0] + half, position[1] + height, position[2] + half],
        }
    }

    /// `AABB.intersects`, strict on every axis.
    fn intersects(&self, other: &Self) -> bool {
        (0..3).all(|a| self.min[a] < other.max[a] && self.max[a] > other.min[a])
    }

    /// `AABB.inflate(d, 0, d)`.
    fn grown_horizontally(&self, d: f64) -> Self {
        Self {
            min: [self.min[0] - d, self.min[1], self.min[2] - d],
            max: [self.max[0] + d, self.max[1], self.max[2] + d],
        }
    }
}

/// `Mob.DEFAULT_ATTACK_REACH`: the maximum reach of a mob whose held item has no `attack_range`.
pub const DEFAULT_ATTACK_REACH: f64 = 1.428_285_672_354_458_4 - 0.600_000_023_841_857_9;

/// `Mob.getAttackBoundingBox` before the inflation: `own`, joined in x and z with `mount` when
/// riding; y is the rider's own.
pub fn attack_box(own: Aabb, mount: Option<Aabb>) -> Aabb {
    let Some(mount) = mount else { return own };
    Aabb {
        min: [
            own.min[0].min(mount.min[0]),
            own.min[1],
            own.min[2].min(mount.min[2]),
        ],
        max: [
            own.max[0].max(mount.max[0]),
            own.max[1],
            own.max[2].max(mount.max[2]),
        ],
    }
}

/// Minimum and maximum melee reach in blocks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeleeReach {
    pub min: f64,
    pub max: f64,
}

impl MeleeReach {
    /// Reach of a mob holding an item with `range = (min_reach, max_reach, mob_factor)` from its
    /// `minecraft:attack_range`, or none: products in `f32` as `effectiveMinRange`/`MaxRange` do,
    /// and `[0, DEFAULT_ATTACK_REACH]` without the component.
    pub fn mob(range: Option<(f32, f32, f32)>) -> Self {
        match range {
            Some((min_reach, max_reach, mob_factor)) => Self {
                min: f64::from(min_reach * mob_factor),
                max: f64::from(max_reach * mob_factor),
            },
            None => Self {
                min: 0.0,
                max: DEFAULT_ATTACK_REACH,
            },
        }
    }
}

/// `Mob.isWithinMeleeAttackRange`: `attack` (see [`attack_box`]) grown by `reach.max` in x and z
/// touches `target`, and with a positive `reach.min` the box grown by `reach.min` does not.
pub fn in_melee_range(attack: Aabb, target: Aabb, reach: MeleeReach) -> bool {
    attack.grown_horizontally(reach.max).intersects(&target)
        && (reach.min <= 0.0 || !attack.grown_horizontally(reach.min).intersects(&target))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(x: f64, y: f64) -> Aabb {
        Aabb::at([x, y, 0.0], 0.6, 1.95)
    }

    #[test]
    fn default_reach_is_the_javap_constant() {
        let expected = 2.039_999_961_853_027_3_f64.sqrt() - 0.600_000_023_841_857_9_f64;
        assert!(
            (DEFAULT_ATTACK_REACH - expected).abs() < 1e-15,
            "{DEFAULT_ATTACK_REACH} vs {expected}"
        );
        assert_eq!(
            MeleeReach::mob(None),
            MeleeReach {
                min: 0.0,
                max: DEFAULT_ATTACK_REACH
            }
        );
    }

    #[test]
    fn reach_is_the_attack_range_times_mob_factor_in_f32() {
        // Iron spear: min 2, max 4.5, mob factor 0.5.
        assert_eq!(
            MeleeReach::mob(Some((2.0, 4.5, 0.5))),
            MeleeReach {
                min: 1.0,
                max: 2.25
            }
        );
        // A float product, widened afterwards (not a double product).
        assert_eq!(
            MeleeReach::mob(Some((0.0, 0.1, 3.0))).max,
            f64::from(0.1_f32 * 3.0_f32)
        );
    }

    #[test]
    fn gap_between_the_boxes_decides_the_bare_hand_reach() {
        let reach = MeleeReach::mob(None);
        let me = body(0.0, 0.0);
        assert!(
            in_melee_range(me, body(1.4, 0.0), reach),
            "gap 0.8 <= 0.828"
        );
        assert!(!in_melee_range(me, body(1.5, 0.0), reach), "gap 0.9");
    }

    #[test]
    fn heights_must_overlap_and_are_not_grown() {
        let reach = MeleeReach::mob(None);
        let me = body(0.0, 0.0);
        assert!(in_melee_range(me, body(1.4, 1.9), reach), "overlap 0.05");
        assert!(
            !in_melee_range(me, body(1.4, 2.0), reach),
            "target starts above the head"
        );
        assert!(
            !in_melee_range(me, body(1.4, -1.95), reach),
            "target ends at the feet"
        );
    }

    #[test]
    fn spear_strikes_beyond_the_minimum_and_not_inside_it() {
        let spear = MeleeReach::mob(Some((2.0, 4.5, 0.5)));
        let me = body(0.0, 0.0);
        assert!(in_melee_range(me, body(2.6, 0.0), spear), "gap 2.0");
        assert!(!in_melee_range(me, body(3.0, 0.0), spear), "gap 2.4");
        assert!(
            !in_melee_range(me, body(1.1, 0.0), spear),
            "gap 0.5 is inside the minimum 1.0"
        );
    }

    #[test]
    fn a_mount_extends_the_strike_sideways_not_the_target() {
        let reach = MeleeReach::mob(None);
        let me = body(0.0, 0.0);
        let horse = Aabb::at([0.0, 0.0, 0.0], 1.4, 1.6);
        assert!(
            !in_melee_range(attack_box(me, None), body(1.8, 0.0), reach),
            "gap 1.2 on foot"
        );
        assert!(
            in_melee_range(attack_box(me, Some(horse)), body(1.8, 0.0), reach),
            "gap 0.8 from the flank"
        );
        // The target is judged by its own box, never joined with its horse.
        assert!(!in_melee_range(attack_box(body(1.8, 0.0), None), me, reach));
    }

    #[test]
    fn the_joined_box_keeps_the_riders_height() {
        let rider = Aabb::at([0.0, 1.2, 0.0], 0.6, 1.95);
        let horse = Aabb::at([0.0, 0.0, 0.0], 1.4, 1.6);
        let attack = attack_box(rider, Some(horse));
        assert_eq!(
            (attack.min, attack.max),
            ([-0.7, 1.2, -0.7], [0.7, 3.15, 0.7])
        );
        // A target on ground 0.8 lower (top 1.15) is beside a rider on foot but under a mounted one.
        let low = body(1.4, -0.8);
        assert!(in_melee_range(body(0.0, 0.0), low, MeleeReach::mob(None)));
        assert!(!in_melee_range(attack, low, MeleeReach::mob(None)));
    }
}
