//! Human-speed action clocks; formation pulses, not faster swings, pace the film.
//! Calibration: BattleVideo/research-combat-behavior.md §§4.4–4.7 (Sabin/Ilerda).
use rustc_hash::FxHashMap;

use super::formation::{DECISION_TICKS, EXHAUSTED_STAMINA, STANDING_RECOVERY, SURGE_FATIGUE};
use super::{BattleIntent, BattleState, Frame, Output};
use crate::boundary::{EntityEvent, EntityId, Hand};
use crate::entities::EntitySlot;
use crate::intent::{DamageSource, Footprint, IntentKind};

// The 21..25-tick action cycle skips one 20-tick decision after each attack.
// Count that recovery before deriving odds; do not shorten the action clocks.
const ATTACK_DECISION_TICKS: u64 = 20;
const AGGRESSIVE_FIGHTERS: u32 = 2500;
const AGGRESSIVE_ATTEMPTS: u32 = 8000;
const CAUTIOUS_ATTEMPTS: u32 = 2000;
const STRIKES_PER_SECOND: f32 = {
    let share = AGGRESSIVE_FIGHTERS as f32 / 10_000.0;
    let aggressive = AGGRESSIVE_ATTEMPTS as f32 / 10_000.0;
    let cautious = CAUTIOUS_ATTEMPTS as f32 / 10_000.0;
    (share * aggressive / (1.0 + aggressive) + (1.0 - share) * cautious / (1.0 + cautious))
        * (20.0 / ATTACK_DECISION_TICKS as f32)
};
// Cinematic calibration: 80 removals per 46-file regiment in 120 s of contact.
// Fatigue permits 65 fighting seconds, but the native three-minute run's
// busiest infantry front only accumulated 46.03 target-bearing pulse seconds.
// Bound the budget by that measured exposure, not just the stamina ceiling.
// The native 20k/6000-tick reference measured 2316 parries / 3873 strikes.
const FRONT_FILES: f32 = 46.0;
const OBSERVED_FRONT_FIGHT_SECONDS: f32 = 46.0;
const ACTIVE_FIGHT_SECONDS: f32 = {
    let decision_seconds = DECISION_TICKS as f32 / 20.0;
    let fatigue_ceiling = (100.0 - EXHAUSTED_STAMINA
        + 120.0 * STANDING_RECOVERY / decision_seconds)
        / ((SURGE_FATIGUE + STANDING_RECOVERY) / decision_seconds);
    if fatigue_ceiling < OBSERVED_FRONT_FIGHT_SECONDS {
        fatigue_ceiling
    } else {
        OBSERVED_FRONT_FIGHT_SECONDS
    }
};
const TARGET_REMOVALS: f32 = 80.0;
const PARRY_FRACTION: f32 = 2316.0 / 3873.0;
const REMOVAL_ROLLS: f32 = 10_000.0 * TARGET_REMOVALS
    / (FRONT_FILES * STRIKES_PER_SECOND * ACTIVE_FIGHT_SECONDS * (1.0 - PARRY_FRACTION));
// Film lethality: most removals fall on the spot; the wounded share withdraws
// through the ranks. Pursuit cuts down nearly everyone it reaches.
const FATAL_SHARE: f32 = 0.7;
const PURSUIT_REMOVAL_ROLLS: f32 = 3.0 * REMOVAL_ROLLS;
const PURSUIT_FATAL_SHARE: f32 = 0.9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weapon {
    Sword,
    Spear,
    Mace,
    Bow,
}

impl Weapon {
    pub const fn windup(self) -> u16 {
        match self {
            Self::Sword | Self::Bow => 10,
            Self::Spear => 11,
            Self::Mace => 12,
        }
    }
    /// Strike occupies one tick; recovery has the same length as windup.
    pub const fn cycle(self) -> u16 {
        2 * self.windup() + 1
    }
    pub const fn reach(self) -> f64 {
        match self {
            Self::Spear => 3.5,
            _ => 2.5,
        }
    }
    const fn stagger(self) -> u64 {
        if matches!(self, Self::Mace) { 12 } else { 18 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Action {
    Guard {
        until: u64,
    },
    Windup {
        target: EntityId,
        start: u64,
        weapon: Weapon,
    },
    Recover {
        until: u64,
    },
    Stagger {
        until: u64,
    },
    Wounded,
}

#[derive(Debug)]
pub(super) struct Fighter {
    pub(super) action: Action,
    pub(super) weapon: Weapon,
    equipment: Option<(Option<u32>, Option<u32>, u32)>,
    shield: bool,
    block: Option<(EntityId, u64)>,
    reaction_checked: bool,
    pub(super) arrows: u16,
    pub(super) charge: super::cavalry::Charge,
    /// Frozen-pass tick of a reserved kill: a second attacker must not credit
    /// the same casualty before native intent merge removes the body.
    slain: Option<u64>,
}

impl Default for Fighter {
    fn default() -> Self {
        Self {
            action: Action::Guard { until: 0 },
            weapon: Weapon::Sword,
            equipment: None,
            shield: false,
            block: None,
            reaction_checked: false,
            arrows: 60,
            charge: Default::default(),
            slain: None,
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Work {
    pub visited: u64,
    pub target_checks: u64,
    pub block_checks: u64,
    pub swings: u64,
    pub strikes: u64,
    pub parries: u64,
    pub wounded: u64,
    pub killed: u64,
    pub arrows: u64,
    pub arc_solves: u64,
    pub arc_iterations: u64,
    pub charge_checks: u64,
}

#[derive(Debug, Default)]
pub(crate) struct CombatRuntime {
    pub(super) fighters: Vec<Fighter>,
    ids: Vec<EntityId>,
    // Used only on roster layout changes, never in per-fighter action passes.
    remap: FxHashMap<EntityId, Fighter>,
    pub(super) archery: super::archery::Runtime,
    strikes: Vec<(EntityId, EntityId, Weapon, u64)>,
    /// Reusable per-regiment cavalry sweep plans, rebuilt every tick.
    pub(super) sweeps: Vec<Option<super::cavalry::Sweep>>,
    pub(crate) work: Work,
}

impl CombatRuntime {
    pub(super) fn prepare(&mut self, ids: &[EntityId]) {
        if self.ids == ids {
            return;
        }
        self.remap
            .extend(self.ids.drain(..).zip(self.fighters.drain(..)));
        self.ids.extend_from_slice(ids);
        self.fighters.extend(
            ids.iter()
                .map(|id| self.remap.remove(id).unwrap_or_default()),
        );
        self.remap.clear();
    }
}

pub(super) fn context(battle: &BattleState, bi: usize) -> Option<super::formation::CombatContext> {
    (bi < battle.formation.members.len()).then(|| battle.formation.context_at(bi))
}

/// Stateless seed mixing: stable across hash iteration order and worker count.
pub(super) fn roll(id: EntityId, counter: u64, salt: u64) -> u32 {
    let mut n = (id.0 as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ counter ^ salt;
    n = (n ^ (n >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    n = (n ^ (n >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    ((n ^ (n >> 31)) % 10_000) as u32
}

pub fn block_window(ticks_to_strike: u64) -> bool {
    (2..=6).contains(&ticks_to_strike)
}

pub(super) fn is_archer(frame: &Frame<'_>, slot: EntitySlot) -> bool {
    frame.entities.row(slot).appearance.equipment[0]
        .as_ref()
        .and_then(|held| frame.registry.items.name_of(held.item_id))
        .is_some_and(|name| name.as_str().ends_with("bow"))
        || matches!(frame.entities.body(slot).kind,
            crate::entities::EntityKind::Mob { type_id }
            if frame.registry.blueprint(type_id).is_some_and(|bp|
                bp.has_goal(crate::blueprint::GoalKind::RangedAttack)))
}

fn classify(frame: &Frame<'_>, slot: EntitySlot, fighter: &mut Fighter, ranged: bool) {
    let row = frame.entities.row(slot);
    let crate::entities::EntityKind::Mob { type_id } = frame.entities.body(slot).kind else {
        return;
    };
    let item = row.appearance.equipment[0]
        .as_ref()
        .filter(|s| !s.is_empty())
        .map(|s| s.item_id);
    let offhand = row.appearance.equipment[1]
        .as_ref()
        .filter(|s| !s.is_empty())
        .map(|s| s.item_id);
    let key = (item, offhand, type_id);
    if fighter.equipment == Some(key) {
        return;
    }
    fighter.equipment = Some(key);
    let name = item
        .and_then(|id| frame.registry.items.name_of(id))
        .map_or("", |id| id.as_str());
    fighter.weapon = if ranged {
        Weapon::Bow
    } else if name.ends_with("spear") || name.ends_with("trident") {
        Weapon::Spear
    } else if name.ends_with("mace") || name.ends_with("_axe") {
        Weapon::Mace
    } else {
        Weapon::Sword
    };
    fighter.shield = offhand
        .and_then(|id| frame.registry.items.name_of(id))
        .and_then(|name| frame.registry.item_facts.get(name))
        .is_some_and(|facts| facts.blocks_attacks_disable_cooldown_scale.is_some());
}

pub(crate) fn step(frame: &Frame<'_>, battle: &mut BattleState, out: &mut Output) {
    let mut combat = std::mem::take(&mut battle.combat);
    combat.work = Work::default();
    combat.strikes.clear();
    combat.prepare(&battle.columns.ids);
    // Ordered execution columns avoid per-fighter hash lookups.
    for bi in 0..battle.columns.ids.len() {
        if !battle.columns.active[bi] || battle.columns.carried[bi] {
            continue;
        }
        let slot = battle.columns.slots[bi].expect("active column");
        let id = battle.columns.ids[bi];
        combat.work.visited += 1;
        let fighter = &mut combat.fighters[bi];
        if battle.columns.equipment_dirty[bi] || fighter.equipment.is_none() {
            classify(frame, slot, fighter, battle.columns.ranged[bi]);
            battle.columns.equipment_dirty[bi] = false;
        }
        if matches!(fighter.action, Action::Wounded) {
            continue;
        }
        if context(battle, bi).is_some_and(|c| c.routing) {
            fighter.action = Action::Guard {
                until: frame.tick + 20,
            };
            fighter.block = None;
            continue;
        }
        match fighter.action {
            Action::Windup {
                target,
                start,
                weapon,
            } => {
                let impact = start + u64::from(weapon.windup());
                if frame.tick == impact {
                    combat.strikes.push((id, target, weapon, start));
                    fighter.action = Action::Recover {
                        until: start + u64::from(weapon.cycle()),
                    };
                } else if frame.tick > impact {
                    // Inactive chunks cannot accumulate delayed damage on return.
                    fighter.action = Action::Guard { until: frame.tick };
                }
            }
            Action::Recover { until } | Action::Stagger { until } if frame.tick >= until => {
                fighter.action = Action::Guard { until: frame.tick };
            }
            _ => {}
        }
    }
    react_blocks(frame, battle, &mut combat, out);
    super::cavalry::step(frame, battle, &mut combat);
    // Take reusable strike scratch so resolution can mutate both opponents.
    let mut strikes = std::mem::take(&mut combat.strikes);
    for &(actor, target, weapon, start) in &strikes {
        strike(
            frame,
            battle,
            &mut combat,
            out,
            actor,
            target,
            weapon,
            start,
        );
    }
    strikes.clear();
    combat.strikes = strikes;
    super::archery::step(frame, battle, &mut combat, out);
    for bi in 0..battle.columns.ids.len() {
        if !battle.columns.active[bi] || battle.columns.carried[bi] {
            continue;
        }
        let slot = battle.columns.slots[bi].expect("active column");
        let id = battle.columns.ids[bi];
        if bi < battle.formation.members.len() && !battle.formation.can_attack_at(bi) {
            continue;
        }
        let fighter = &mut combat.fighters[bi];
        let Action::Guard { until } = fighter.action else {
            continue;
        };
        if fighter.weapon == Weapon::Bow || frame.tick < until {
            continue;
        }
        if fighter
            .block
            .is_some_and(|(_, impact)| frame.tick <= impact)
        {
            continue;
        }
        let Some(target) = battle.columns.targets[bi].as_ref() else {
            continue;
        };
        combat.work.target_checks += 1;
        if super::retained_enemy(frame.entities, slot, target.id, fighter.weapon.reach()).is_none()
        {
            continue;
        }
        // Three cautious soldiers in four; decisions once/20t, not per frame.
        if !(frame.tick + id.0 as u64).is_multiple_of(ATTACK_DECISION_TICKS) {
            continue;
        }
        fighter.action = Action::Guard {
            until: frame.tick + ATTACK_DECISION_TICKS,
        };
        // Anyone may cut at a fleeing back; the cautious majority only parries
        // and probes a steady front.
        let fleeing = frame
            .entities
            .slot_of(target.id)
            .and_then(|slot| battle.columns.index(slot))
            .and_then(|target_bi| context(battle, target_bi))
            .is_some_and(|c| c.routing);
        let probability = if fleeing || roll(id, 0, 1) < AGGRESSIVE_FIGHTERS {
            AGGRESSIVE_ATTEMPTS
        } else {
            CAUTIOUS_ATTEMPTS
        };
        if roll(id, frame.tick / ATTACK_DECISION_TICKS, 2) >= probability {
            continue;
        }
        fighter.action = Action::Windup {
            target: target.id,
            start: frame.tick,
            weapon: fighter.weapon,
        };
        fighter.block = None;
        fighter.reaction_checked = false;
        out.events.push(EntityEvent::Swing {
            entity: id,
            hand: Hand::Main,
            animation: if fighter.weapon == Weapon::Spear {
                crate::boundary::SwingKind::Stab
            } else {
                crate::boundary::SwingKind::Whack
            },
            duration_ticks: fighter.weapon.cycle(),
        });
        combat.work.swings += 1;
    }
    battle.combat = combat;
}

fn react_blocks(
    frame: &Frame<'_>,
    battle: &BattleState,
    combat: &mut CombatRuntime,
    out: &mut Output,
) {
    for bi in 0..battle.columns.ids.len() {
        if !battle.columns.active[bi] || battle.columns.carried[bi] {
            continue;
        }
        let actor = battle.columns.ids[bi];
        let attacker = &combat.fighters[bi];
        let Action::Windup {
            target,
            start,
            weapon,
        } = attacker.action
        else {
            continue;
        };
        let impact = start + u64::from(weapon.windup());
        if attacker.reaction_checked || !block_window(impact.saturating_sub(frame.tick)) {
            continue;
        }
        let Some(target_bi) = frame
            .entities
            .slot_of(target)
            .and_then(|slot| battle.columns.index(slot))
        else {
            continue;
        };
        if context(battle, target_bi).is_some_and(|c| c.routing)
            || !battle.columns.active[target_bi]
            || battle.columns.carried[target_bi]
        {
            continue;
        }
        let defender = &mut combat.fighters[target_bi];
        if !defender.shield || !matches!(defender.action, Action::Guard { .. }) {
            continue;
        }
        combat.work.block_checks += 1;
        // One roll at the first eligible instant in the 6..2 window, including
        // a defender finishing recovery late; never five independent retries.
        if roll(target, start, actor.0 as u64) < 6500 {
            defender.block = Some((actor, impact));
            out.events.push(EntityEvent::Swing {
                entity: target,
                hand: Hand::Off,
                animation: crate::boundary::SwingKind::Whack,
                duration_ticks: 10,
            });
        }
        combat.fighters[bi].reaction_checked = true;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Outcome {
    Glance,
    Wound,
    Kill,
}

/// Frontage/block-adjusted film outcomes; nominal pulse exposure ~26 s.
/// Pursuit shifts casualties to the broken side; action clocks are unchanged.
pub(super) fn outcome(roll: u32, routing: bool, charge: f32) -> Outcome {
    let (removal, fatal_share) = if routing {
        (PURSUIT_REMOVAL_ROLLS, PURSUIT_FATAL_SHARE)
    } else {
        (REMOVAL_ROLLS, FATAL_SHARE)
    };
    // Saturate total odds first; overflow must not turn the wounded share fatal.
    let removal = (removal * (1.0 + charge)).min(10_000.0);
    if (roll as f32) < removal * fatal_share {
        Outcome::Kill
    } else if (roll as f32) < removal {
        Outcome::Wound
    } else {
        Outcome::Glance
    }
}

#[allow(clippy::too_many_arguments)]
fn strike(
    frame: &Frame<'_>,
    battle: &mut BattleState,
    combat: &mut CombatRuntime,
    out: &mut Output,
    actor: EntityId,
    target: EntityId,
    weapon: Weapon,
    start: u64,
) {
    let Some(slot) = frame.entities.slot_of(actor) else {
        return;
    };
    let Some(victim) = frame.entities.slot_of(target) else {
        return;
    };
    let Some(actor_bi) = battle.columns.index(slot) else {
        return;
    };
    let Some(target_bi) = battle.columns.index(victim) else {
        return;
    };
    combat.work.target_checks += 1;
    if super::retained_enemy(frame.entities, slot, target, weapon.reach()).is_none()
        || matches!(
            combat.fighters[actor_bi].action,
            Action::Wounded | Action::Stagger { .. }
        )
        || combat.fighters[target_bi].slain == Some(frame.tick)
    {
        return;
    }
    combat.work.strikes += 1;
    if combat.fighters[target_bi].block == Some((actor, frame.tick)) {
        combat.fighters[actor_bi].action = Action::Stagger {
            until: frame.tick + weapon.stagger(),
        };
        out.events.push(EntityEvent::ShieldBlock {
            entity: target,
            disabled: false,
        });
        out.events.push(EntityEvent::Swing {
            entity: actor,
            hand: Hand::Main,
            animation: crate::boundary::SwingKind::None,
            duration_ticks: weapon.stagger() as u16,
        });
        combat.work.parries += 1;
        return;
    }
    let routing = context(battle, target_bi).is_some_and(|c| c.routing);
    let charge = combat.fighters[actor_bi].charge.bonus(frame.tick);
    let result = outcome(roll(actor, start, target.0 as u64), routing, charge);
    if result == Outcome::Glance {
        return;
    }
    // A withdrawing wounded man is not immune: any landed blow finishes him.
    // Otherwise he stays the nearest target forever and freezes the front.
    let result = if matches!(combat.fighters[target_bi].action, Action::Wounded) {
        Outcome::Kill
    } else {
        result
    };
    combat.fighters[target_bi].action = Action::Wounded;
    if result == Outcome::Kill {
        combat.fighters[target_bi].slain = Some(frame.tick);
    }
    let amount = if result == Outcome::Kill {
        combat.work.killed += 1;
        // Bypass armour through the native generic kill damage kind below.
        frame.entities.row(victim).health + 1.0
    } else {
        combat.work.wounded += 1;
        battle.formation.mark_wounded(target);
        1.0_f32.min(frame.entities.row(victim).health * 0.5)
    };
    battle.formation.report_kill(actor, target, false);
    let position = frame.entities.pos(slot);
    out.intents.push(BattleIntent {
        actor,
        kind: IntentKind::DamageEntity {
            target,
            amount,
            kind: if result == Outcome::Kill {
                crate::survival::DamageKind::GenericKill
            } else {
                crate::survival::DamageKind::Melee
            },
            source: Some(DamageSource::Melee {
                attacker: actor,
                position,
                yaw: frame.entities.rot(slot)[0],
                bonus_knockback: f64::from(charge),
                sweep: false,
                fire_ticks: 0,
                bane_ticks: 0,
            }),
        },
        footprint: Footprint {
            entity_reads: vec![actor, target],
            entity_writes: vec![target],
            ..Default::default()
        },
    });
}

#[cfg(test)]
#[path = "combat_tests.rs"]
pub(super) mod tests;
