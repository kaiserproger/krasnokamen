//! Human-speed action clocks; formation pulses, not faster swings, pace the film.
//! Calibration: BattleVideo/research-combat-behavior.md §§4.4–4.7 (Sabin/Ilerda).
use rustc_hash::FxHashMap;

use mc_melee_range::MeleeReach;

use super::faction::{in_melee_range, melee_reach, reach_bound};
use super::formation::{DECISION_TICKS, EXHAUSTED_STAMINA, STANDING_RECOVERY, SURGE_FATIGUE};
use super::{BattleIntent, BattleState, Frame, Output};
use crate::boundary::{EntityEvent, EntityId, Hand};
use crate::entities::EntitySlot;
use crate::intent::{DamageSource, Footprint, IntentKind};

// The 21..25-tick action cycle skips one 20-tick decision after each attack.
// Count that recovery before deriving odds; do not shorten the action clocks.
const ATTACK_DECISION_TICKS: u64 = 20;
/// Human reaction: a man who first finds an enemy within his melee range
/// stands guard 4..=8 ticks (seeded per man) before his first decision.
const REACTION_TICKS_MIN: u64 = 4;
const REACTION_TICKS_SPAN: u64 = 5;
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
// The meshed press keeps several times the calibration's 46 files within reach
// of each other per regiment front, so a landed blow removes a man that many
// times less often: losses per minute and time to rout are to stay those of
// the G2 take (owner, 2026-11-13; peak 2 212 a minute, a side's quarter lost
// after 194 s) while far more men trade blows, parries and blocks. At five
// (rehearsal 7baa94d5: 5 500-6 400 fighting for a minute) the press alone
// still cost 2 000-2 450 a minute and the rout 3 423.
const PRESS_EXPOSURE: f32 = 10.0;
const REMOVAL_ROLLS: f32 = 10_000.0 * TARGET_REMOVALS
    / (FRONT_FILES
        * PRESS_EXPOSURE
        * STRIKES_PER_SECOND
        * ACTIVE_FIGHT_SECONDS
        * (1.0 - PARRY_FRACTION));
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
    /// Mace and axe blows stagger a parrying shield less than a sword or a spear.
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
    /// Melee reach of the held item (its vanilla `minecraft:attack_range`
    /// for a mob, or the mob default without it), classified with the weapon
    /// when the held item changes.
    pub(super) range: MeleeReach,
    equipment: Option<(Option<u32>, Option<u32>, u32)>,
    shield: bool,
    /// Last shield pose handed to presentation; `None` until first decided.
    raised: Option<bool>,
    block: Option<(EntityId, u64)>,
    reaction_checked: bool,
    pub(super) arrows: u16,
    pub(super) charge: super::cavalry::Charge,
    /// Frozen-pass tick of a reserved kill: a second attacker must not credit
    /// the same casualty before native intent merge removes the body.
    slain: Option<u64>,
    /// Last battle tick he stood guard, wound up or recovered from a blow:
    /// the fight window of the `fighting` telemetry.
    in_fight: Option<u64>,
    /// Tick he first found an enemy within his melee range in this
    /// engagement; cleared when a decision finds none.
    engaged_from: Option<u64>,
    /// Two main weapons (weapons.rs): the one not in hand and the swap clock.
    pub(super) arms: super::weapons::Arms,
}

impl Default for Fighter {
    fn default() -> Self {
        Self {
            action: Action::Guard { until: 0 },
            weapon: Weapon::Sword,
            range: MeleeReach::mob(None),
            equipment: None,
            shield: false,
            raised: None,
            block: None,
            reaction_checked: false,
            arrows: 60,
            charge: Default::default(),
            slain: None,
            in_fight: None,
            engaged_from: None,
            arms: Default::default(),
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
    /// Melee fighters (no bows) with an enemy within their melee range.
    pub in_weapon_reach: u64,
    /// Of those, the ones in the fight loop: the decision pass may let them
    /// strike and they stand guard, wind up or recover from a blow.
    pub fighting: u64,
    /// Of those, the ones in action rather than standing guard: winding up,
    /// recovering from a blow, staggered by a parry or blocking one.
    pub acting: u64,
    /// Shield bearers holding the shield up after this tick's actions.
    pub shields_raised: u64,
    /// Shield blocks (`ShieldBlock` events) this tick.
    pub shield_blocks: u64,
    /// Weapon swaps started this tick (weapons.rs).
    pub weapon_swaps: u64,
    /// Active fighters holding a spear, a sword (or mace) and a bow.
    pub holding_spear: u64,
    pub holding_sword: u64,
    pub holding_bow: u64,
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
    /// Staged second weapons not yet claimed by a fighter (weapons.rs).
    pub(super) arsenal: super::weapons::Arsenal,
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
    let held = item.and_then(|id| frame.registry.items.name_of(id));
    let name = held.map_or("", |id| id.as_str());
    fighter.range = melee_reach(
        held.and_then(|name| frame.registry.item_facts.get(name))
            .and_then(|facts| facts.attack_range),
    );
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
    let events_before = out.events.len();
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
        if context(battle, bi).is_some_and(|c| c.routing) && !battle.formation.cornered_at(bi) {
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
        if super::live_players::is_holder(battle, target) {
            // As for a soldier target: wounded or parried since the windup, no blow.
            let range = frame
                .entities
                .slot_of(actor)
                .and_then(|slot| battle.columns.index(slot))
                .map(|bi| &combat.fighters[bi])
                .filter(|f| !matches!(f.action, Action::Wounded | Action::Stagger { .. }))
                .map(|f| f.range);
            if let Some(range) = range {
                super::live_players::blow(frame, battle, out, actor, target, range);
            }
            continue;
        }
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
    // Two main weapons: swaps start before this tick's volleys and windups.
    super::weapons::step(frame, battle, &mut combat, out);
    super::archery::step(frame, battle, &mut combat, out);
    for bi in 0..battle.columns.ids.len() {
        if !battle.columns.active[bi] || battle.columns.carried[bi] {
            continue;
        }
        let slot = battle.columns.slots[bi].expect("active column");
        let id = battle.columns.ids[bi];
        // Outside a pulse a steady front-rank man still trades blows with an
        // enemy already in weapon reach, at the cautious rate, as does a
        // fugitive at bay (broken, so also wavering): pulses add the surge,
        // they are not the only contact.
        let in_order = bi >= battle.formation.members.len() || battle.formation.can_attack_at(bi);
        if !in_order
            && !context(battle, bi)
                .is_some_and(|c| (!c.routing && !c.wavering) || battle.formation.cornered_at(bi))
        {
            continue;
        }
        let fighter = &mut combat.fighters[bi];
        let Action::Guard { until } = fighter.action else {
            continue;
        };
        // Two main weapons: no windup while the drawn weapon comes up.
        if fighter.arms.swapping(frame.tick) {
            continue;
        }
        if fighter.weapon == Weapon::Bow || frame.tick < until {
            continue;
        }
        if fighter
            .block
            .is_some_and(|(_, impact)| frame.tick <= impact)
        {
            continue;
        }
        let soldier = battle.columns.targets[bi].as_ref().map(|target| target.id);
        if soldier.is_some() {
            combat.work.target_checks += 1;
        }
        let range = fighter.range;
        let reaches = |enemy: EntityId| {
            frame
                .entities
                .slot_of(enemy)
                .is_some_and(|enemy| in_melee_range(frame.entities, slot, enemy, range))
        };
        let decision_tick = (frame.tick + id.0 as u64).is_multiple_of(ATTACK_DECISION_TICKS);
        let soldier = soldier
            .and_then(|target| {
                super::retained_enemy(frame.entities, slot, target, reach_bound(range))
            })
            .filter(|target| reaches(target.id))
            .or_else(|| {
                // Any other enemy in range will do; searched only when a
                // decision is due, never every tick.
                if !decision_tick {
                    return None;
                }
                let unit = super::Unit {
                    id,
                    faction: battle.columns.factions[bi],
                    pos: battle.columns.positions[bi],
                };
                let found = battle
                    .grid
                    .any_enemy(unit, reach_bound(range), |u| reaches(u.id))?;
                super::retained_enemy(frame.entities, slot, found.id, reach_bound(range))
            });
        // A living network player of the other side in reach is one more
        // candidate for this same swing; the nearer of the two is struck.
        let holder = super::live_players::in_reach(battle, frame.entities, slot, range, None);
        let target = match (soldier, holder) {
            (Some(soldier), Some(holder)) if holder.distance < soldier.distance => holder,
            (Some(soldier), _) => soldier,
            (None, Some(holder)) => holder,
            (None, None) => {
                // Out of reach at a decision: the next enemy in reach is a new
                // engagement with its own reaction.
                if decision_tick {
                    fighter.engaged_from = None;
                }
                continue;
            }
        };
        // A man who just found an enemy in reach stands guard through a human
        // reaction (4..=8 ticks, seeded per man) before his first decision.
        let engaged_from = *fighter.engaged_from.get_or_insert(frame.tick);
        let reaction = REACTION_TICKS_MIN + u64::from(roll(id, 0, 3)) % REACTION_TICKS_SPAN;
        if !decision_tick || frame.tick < engaged_from + reaction {
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
        let probability = if fleeing || (in_order && roll(id, 0, 1) < AGGRESSIVE_FIGHTERS) {
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
    count_fighting(frame, battle, &mut combat);
    raise_shields(battle, &mut combat, out);
    combat.work.shield_blocks = out.events[events_before..]
        .iter()
        .filter(|event| matches!(event, EntityEvent::ShieldBlock { .. }))
        .count() as u64;
    battle.combat = combat;
}

/// A fighter counts as fighting while his fight loop (guard, windup, blow,
/// recovery) ran within this many ticks: one 21..25-tick action cycle, so a
/// stagger after a parried blow stays inside the window.
const FIGHT_WINDOW_TICKS: u64 = 25;
const COUNT_CHUNK: usize = 256;

/// Telemetry of who is fighting this tick. In weapon reach: any enemy within
/// the fighter's weapon reach, from his retained target or else one query of
/// this tick's frozen target grid at weapon reach (no bows). Fighting: of
/// those, everyone not in flight (a wavering man standing guard with his
/// shield up counts) whose guard/windup/recovery ran within the window; a
/// fugitive's placeholder guard is not a fight. Parallel over fighter chunks;
/// no allocation.
pub(super) fn count_fighting(frame: &Frame<'_>, battle: &BattleState, combat: &mut CombatRuntime) {
    use rayon::prelude::*;
    let tick = frame.tick;
    let (columns, formation, grid) = (&battle.columns, &battle.formation, &battle.grid);
    let entities = frame.entities;
    let count = |(chunk, fighters): (usize, &mut [Fighter])| {
        let mut counts = (0_u64, 0_u64, 0_u64);
        for (offset, fighter) in fighters.iter_mut().enumerate() {
            let bi = chunk * COUNT_CHUNK + offset;
            if !columns.active[bi] || columns.carried[bi] {
                continue;
            }
            // A fugitive (unless at bay) is held in guard only as a placeholder.
            let fleeing = bi < formation.members.len()
                && formation.context_at(bi).routing
                && !formation.cornered_at(bi);
            if !fleeing
                && matches!(
                    fighter.action,
                    Action::Guard { .. } | Action::Windup { .. } | Action::Recover { .. }
                )
            {
                fighter.in_fight = Some(tick);
            }
            if fighter.weapon == Weapon::Bow {
                continue;
            }
            let range = fighter.range;
            let actor = columns.slots[bi].expect("active column");
            let reaches = |enemy: EntityId| {
                entities
                    .slot_of(enemy)
                    .is_some_and(|enemy| in_melee_range(entities, actor, enemy, range))
            };
            let in_reach = columns.targets[bi].is_some_and(|t| reaches(t.id))
                || grid
                    .any_enemy(
                        super::Unit {
                            id: columns.ids[bi],
                            faction: columns.factions[bi],
                            pos: columns.positions[bi],
                        },
                        reach_bound(range),
                        |u| reaches(u.id),
                    )
                    .is_some();
            if !in_reach {
                continue;
            }
            counts.0 += 1;
            let recent = !fleeing
                && !matches!(fighter.action, Action::Wounded)
                && fighter
                    .in_fight
                    .is_some_and(|at| tick.saturating_sub(at) <= FIGHT_WINDOW_TICKS);
            counts.1 += u64::from(recent);
            // In action rather than standing guard: winding up, recovering
            // from his blow, staggered by a parry or blocking an incoming one.
            let acting = recent
                && (matches!(
                    fighter.action,
                    Action::Windup { .. } | Action::Recover { .. } | Action::Stagger { .. }
                ) || fighter.block.is_some_and(|(_, impact)| tick <= impact));
            counts.2 += u64::from(acting);
        }
        counts
    };
    let add = |a: (u64, u64, u64), b: (u64, u64, u64)| (a.0 + b.0, a.1 + b.1, a.2 + b.2);
    let fighters = &mut combat.fighters[..battle.columns.ids.len()];
    let (in_reach, fighting, acting) =
        if fighters.len() >= COUNT_CHUNK * 2 && rayon::current_num_threads() > 1 {
            fighters
                .par_chunks_mut(COUNT_CHUNK)
                .enumerate()
                .map(count)
                .reduce(|| (0, 0, 0), add)
        } else {
            fighters
                .chunks_mut(COUNT_CHUNK)
                .enumerate()
                .map(count)
                .fold((0, 0, 0), add)
        };
    combat.work.in_weapon_reach = in_reach;
    combat.work.fighting = fighting;
    combat.work.acting = acting;
}

/// A shield bearer standing guard in order holds the shield up; a swing, a
/// parried stagger, a wound or a rout lowers it. Only transitions leave here;
/// the raised count is telemetry.
fn raise_shields(battle: &BattleState, combat: &mut CombatRuntime, out: &mut Output) {
    let mut shields_raised = 0;
    for bi in 0..battle.columns.ids.len() {
        if !battle.columns.active[bi] || battle.columns.carried[bi] {
            continue;
        }
        let fighter = &mut combat.fighters[bi];
        let raised = fighter.shield
            && matches!(fighter.action, Action::Guard { .. })
            && !context(battle, bi).is_some_and(|c| c.routing);
        shields_raised += u64::from(raised);
        if fighter.raised != Some(raised) {
            fighter.raised = Some(raised);
            out.shields.push((battle.columns.ids[bi], raised));
        }
    }
    combat.work.shields_raised = shields_raised;
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
    let range = combat.fighters[actor_bi].range;
    if super::retained_enemy(frame.entities, slot, target, reach_bound(range)).is_none()
        || !in_melee_range(frame.entities, slot, victim, range)
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
