//! Retaliation (owner 2026-11-14, G2L5): a battle soldier answers an enemy
//! who hurt him, a live player included, and friends take on an attacker he
//! cannot answer.
//!
//! Invariant: for [`GRUDGE_TICKS`] after an enemy's blow took health from him
//! (the damage merge reports it, [`hurt`]), a steady melee soldier's next
//! re-pick takes that attacker over the nearest enemy while the attacker
//! stands within [`MELEE_ENGAGEMENT_REACH`]. He keeps him as his target until
//! the grudge lapses or the attacker leaves that reach: he looks at him, steps
//! in to his own weapon's reach (`Member::close_on`) and strikes him like any
//! target, a live player too (`combat`). When the attacker is still out of his
//! reach [`STEP_IN_TICKS`] after the first blow, or two enemies within
//! [`HELP_RADIUS`] hurt him inside the window, each such attacker goes to the
//! idle friends nearest the attacker (no target, neither hurt nor helping),
//! at most [`HELPERS`] per attacker, who answer him the same way for a window.
//!
//! Between soldiers a landed blow wounds (the man withdraws) or kills, so a
//! soldier-versus-soldier fight keeps its nearest-enemy choice (`targeting`);
//! only a blow that leaves a man able to fight (a live player's, an arrow's)
//! makes him answer.
//!
//! Cost per tick: one pass over the soldiers hurt or helping within the
//! window, and per hand-off one lookup of the attacker's helpers plus the
//! target grid's cells within [`HELP_RADIUS`] of the attacker; never a scan
//! of the whole roster or of all grudges per hand-off.
use super::faction::{in_melee_range, live_unit};
use super::{BattleState, EnemyTarget, Frame, MELEE_ENGAGEMENT_REACH, Unit};
use crate::boundary::EntityId;
use crate::entities::{EntitySlot, EntityStore};
use crate::tick::TickState;
use rustc_hash::FxHashMap;

/// How long a blow is remembered: 3 s.
pub(crate) const GRUDGE_TICKS: u64 = 60;
/// Attackers this close to the victim are handed on; helpers are found this
/// close to the attacker and keep him while he stays this close.
pub(crate) const HELP_RADIUS: f64 = 8.0;
/// Most friends answering one attacker for others, so a player is not mobbed
/// by a whole rank.
pub(crate) const HELPERS: usize = 2;
/// Time from the first blow a victim has to step in on his attacker before
/// an attacker still out of his reach is handed on: 2 s, enough to close from
/// the edge of [`MELEE_ENGAGEMENT_REACH`] to a sword's reach at the
/// formation's closing step.
pub(crate) const STEP_IN_TICKS: u64 = 40;

#[derive(Debug, Clone, Copy)]
struct Grudge {
    attacker: EntityId,
    /// Battle tick of his last blow (a helper's: of the latest hand-off).
    at: u64,
    /// Battle tick of the first blow of this grudge.
    since: u64,
    /// The enemy who hurt him before `attacker` did, and when.
    other: Option<(EntityId, u64)>,
    /// Handed on by a hurt friend, not hurt himself: never hands on.
    helping: bool,
    /// He holds `attacker` as his target.
    answering: bool,
}

impl Grudge {
    fn fresh(&self, tick: u64) -> bool {
        tick.saturating_sub(self.at) <= GRUDGE_TICKS
    }
}

/// Helpers answering one attacker for hurt friends.
type Crew = [Option<EntityId>; HELPERS];

/// Scale counters of the last pass.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Work {
    /// Soldiers hurt or helping within the window (grudges kept).
    pub remembered: u64,
    /// Of those, soldiers holding their attacker as target.
    pub answering: u64,
    /// Hand-offs weighed (each one attacker of one victim).
    pub handoffs: u64,
    /// Friendly bodies the hand-offs' grid walks visited.
    pub friends_examined: u64,
}

#[derive(Debug, Default)]
pub(crate) struct Retaliation {
    /// (victim, attacker) of each blow on a battle unit the damage merge
    /// committed since the last battle tick.
    pending: Vec<(EntityId, EntityId)>,
    /// Per soldier hurt or helping within the window.
    grudges: FxHashMap<EntityId, Grudge>,
    /// Per attacker, the helpers answering him, rebuilt by each pass.
    crews: FxHashMap<EntityId, Crew>,
    /// Members whose `close_on` the last pass set.
    closing: Vec<usize>,
    /// (victim, attacker) hand-offs of this pass.
    handoffs: Vec<(EntityId, EntityId)>,
    pub(crate) work: Work,
}

/// The damage merge committed a blow of `attacker` that took health from
/// battle unit `victim` (`hostile::retaliate`); the next battle tick weighs it.
pub(crate) fn hurt(state: &mut TickState, victim: EntitySlot, attacker: EntityId) {
    let battle = &mut state.hostile.battle;
    if battle.running {
        let victim = state.entities.body(victim).id;
        battle.retaliation.pending.push((victim, attacker));
    }
}

/// `attacker` as an opponent of `faction` standing at `from`: a living enemy
/// battle unit or a living holder of the other side.
fn opponent(
    entities: &EntityStore,
    live: &super::live_players::LiveState,
    faction: u16,
    from: [f64; 3],
    attacker: EntityId,
) -> Option<(EntitySlot, EnemyTarget)> {
    let slot = entities.slot_of(attacker)?;
    let side = match live_unit(entities, slot) {
        Some(unit) => unit.faction,
        None => super::live_players::holder_side(live, attacker)?,
    };
    if !super::enemies(faction, side) {
        return None;
    }
    let pos = entities.pos(slot);
    let distance = crate::target_grid::distance(from, pos);
    Some((
        slot,
        EnemyTarget {
            id: attacker,
            pos,
            distance,
        },
    ))
}

/// Where a foot soldier stands to strike `enemy` with `range`: where he is
/// once in reach, else the point on the line to the enemy at the middle of
/// his reach. A rider is turned by his horse toward the target (`cavalry`).
fn close_on(
    entities: &EntityStore,
    slot: EntitySlot,
    enemy: EntitySlot,
    range: mc_melee_range::MeleeReach,
) -> Option<[f64; 3]> {
    if entities.row(slot).entity_life.mount.vehicle.is_some() {
        return None;
    }
    let own = entities.pos(slot);
    if in_melee_range(entities, slot, enemy, range) {
        return Some(own);
    }
    let at = entities.pos(enemy);
    let stand = entities.body(slot).half_width
        + entities.body(enemy).half_width
        + (range.min + range.max) * 0.5;
    let toward = super::steering::unit([at[0] - own[0], at[2] - own[2]], [0.0, 0.0]);
    Some([at[0] - toward[0] * stand, own[1], at[2] - toward[1] * stand])
}

/// After targeting, before steering: weigh the blows the last merge
/// committed, keep each answering soldier on his attacker (target, look,
/// step) and hand on attackers he cannot answer.
pub(super) fn step(frame: &Frame<'_>, battle: &mut BattleState) {
    let tick = frame.tick;
    let entities = frame.entities;
    let BattleState {
        columns,
        formation,
        combat,
        grid,
        live,
        retaliation: state,
        ..
    } = battle;
    state.work = Work::default();
    // Weapon reach is classified per fighter; align the fighters first.
    combat.prepare(&columns.ids);
    for index in state.closing.drain(..) {
        if let Some(member) = formation.members.get_mut(index) {
            member.close_on = None;
        }
    }
    for (victim, attacker) in state.pending.drain(..) {
        let Some(slot) = entities.slot_of(victim) else {
            continue;
        };
        if victim == attacker
            || opponent(
                entities,
                live,
                entities.body(slot).faction,
                [0.0; 3],
                attacker,
            )
            .is_none()
        {
            continue;
        }
        match state.grudges.get_mut(&victim) {
            Some(grudge) if !grudge.helping && grudge.fresh(tick) => {
                if grudge.attacker != attacker {
                    grudge.other = Some((grudge.attacker, grudge.at));
                    grudge.attacker = attacker;
                    grudge.since = tick;
                    grudge.answering = false;
                }
                grudge.at = tick;
            }
            _ => {
                state.grudges.insert(
                    victim,
                    Grudge {
                        attacker,
                        at: tick,
                        since: tick,
                        other: None,
                        helping: false,
                        answering: false,
                    },
                );
            }
        }
    }
    state.crews.clear();
    let Retaliation {
        grudges,
        crews,
        closing,
        handoffs,
        work,
        ..
    } = state;
    grudges.retain(|&victim, grudge| {
        if !grudge.fresh(tick) {
            return false;
        }
        let Some(bi) = entities
            .slot_of(victim)
            .and_then(|slot| columns.index(slot))
        else {
            return false;
        };
        let slot = columns.slots[bi].expect("indexed column");
        if grudge
            .other
            .is_some_and(|(_, at)| tick.saturating_sub(at) > GRUDGE_TICKS)
        {
            grudge.other = None;
        }
        let (faction, pos) = (columns.factions[bi], columns.positions[bi]);
        let able = bi < formation.members.len()
            && super::targeting::eligible(formation, columns, bi)
            && !columns.ranged[bi];
        let enemy = opponent(entities, live, faction, pos, grudge.attacker);
        let due = super::retarget_due(victim, tick);
        let reach = if grudge.helping {
            HELP_RADIUS
        } else {
            MELEE_ENGAGEMENT_REACH
        };
        let was = grudge.answering;
        if !able {
            grudge.answering = false;
        } else if grudge.answering || due {
            grudge.answering = enemy.is_some_and(|(_, enemy)| enemy.distance <= reach);
        }
        let range = combat.fighters[bi].range;
        if grudge.answering
            && let Some((enemy_slot, enemy)) = enemy
        {
            if !was && super::live_players::holder_side(live, enemy.id).is_some() {
                tracing::info!(
                    "Battle answer: soldier={} attacker={} tick={} distance={:.2} helping={}",
                    victim.0,
                    enemy.id.0,
                    tick,
                    enemy.distance,
                    grudge.helping
                );
            }
            work.answering += 1;
            columns.targets[bi] = Some(enemy);
            if let Some(at) = close_on(entities, slot, enemy_slot, range) {
                formation.members[bi].close_on = Some(at);
                closing.push(bi);
            }
        }
        if grudge.helping
            && let Some(free) = crews
                .entry(grudge.attacker)
                .or_default()
                .iter_mut()
                .find(|helper| helper.is_none())
        {
            *free = Some(victim);
        }
        if due && !grudge.helping {
            let near = |attacker| {
                opponent(entities, live, faction, pos, attacker)
                    .filter(|(_, enemy)| enemy.distance <= HELP_RADIUS)
            };
            let unanswered = grudge.answering
                && tick.saturating_sub(grudge.since) >= STEP_IN_TICKS
                && enemy.is_some_and(|(enemy_slot, _)| {
                    !in_melee_range(entities, slot, enemy_slot, range)
                });
            let second = grudge.other.and_then(|(other, _)| near(other));
            if (unanswered || second.is_some()) && near(grudge.attacker).is_some() {
                handoffs.push((victim, grudge.attacker));
            }
            if let Some((_, other)) = second {
                handoffs.push((victim, other.id));
            }
        }
        true
    });
    work.remembered = grudges.len() as u64;
    work.handoffs = handoffs.len() as u64;
    for index in 0..state.handoffs.len() {
        let (victim, attacker) = state.handoffs[index];
        hand_off(
            frame, columns, formation, combat, grid, live, state, victim, attacker,
        );
    }
    state.handoffs.clear();
}

/// Hand `attacker` of `victim` to the idle friends nearest the attacker:
/// those already helping against him keep on for another window, and new
/// ones fill his crew up to [`HELPERS`].
#[allow(clippy::too_many_arguments)]
fn hand_off(
    frame: &Frame<'_>,
    columns: &mut super::Columns,
    formation: &mut super::formation::FormationRuntime,
    combat: &super::combat::CombatRuntime,
    grid: &super::TargetGrid,
    live: &super::live_players::LiveState,
    state: &mut Retaliation,
    victim: EntityId,
    attacker: EntityId,
) {
    let tick = frame.tick;
    let entities = frame.entities;
    let crew = state.crews.entry(attacker).or_default();
    let mut helping = 0;
    for helper in crew.iter().flatten() {
        if let Some(grudge) = state.grudges.get_mut(helper) {
            grudge.at = tick;
            helping += 1;
        }
    }
    if helping >= HELPERS {
        return;
    }
    let (Some(victim_bi), Some(attacker_slot)) = (
        entities
            .slot_of(victim)
            .and_then(|slot| columns.index(slot)),
        entities.slot_of(attacker),
    ) else {
        return;
    };
    let faction = columns.factions[victim_bi];
    let around = Unit {
        id: attacker,
        faction,
        pos: entities.pos(attacker_slot),
    };
    // The nearest idle friends: (squared distance, id, column), nearest first.
    let mut nearest: [Option<(f64, EntityId, usize)>; HELPERS] = [None; HELPERS];
    let grudges = &state.grudges;
    let mut examined = 0;
    grid.friends_within(around, HELP_RADIUS, |unit, d2| {
        examined += 1;
        let Some(bi) = entities
            .slot_of(unit.id)
            .and_then(|slot| columns.index(slot))
        else {
            return;
        };
        if bi >= formation.members.len()
            || columns.ranged[bi]
            || columns.targets[bi].is_some()
            || grudges.contains_key(&unit.id)
            || !super::targeting::eligible(formation, columns, bi)
        {
            return;
        }
        let mut candidate = Some((d2, unit.id, bi));
        for place in &mut nearest {
            let better = match (*place, candidate) {
                (_, None) => false,
                (None, Some(_)) => true,
                (Some((d, id, _)), Some((cd, cid, _))) => cd < d || (cd == d && cid < id),
            };
            if better {
                candidate = std::mem::replace(place, candidate);
            }
        }
    });
    state.work.friends_examined += examined;
    let mut added = 0;
    for (_, helper, bi) in nearest.into_iter().flatten().take(HELPERS - helping) {
        let slot = columns.slots[bi].expect("indexed column");
        let Some((enemy_slot, enemy)) =
            opponent(entities, live, faction, columns.positions[bi], attacker)
        else {
            continue;
        };
        state.grudges.insert(
            helper,
            Grudge {
                attacker,
                at: tick,
                since: tick,
                other: None,
                helping: true,
                answering: true,
            },
        );
        if let Some(free) = crew.iter_mut().find(|place| place.is_none()) {
            *free = Some(helper);
        }
        columns.targets[bi] = Some(enemy);
        if let Some(at) = close_on(entities, slot, enemy_slot, combat.fighters[bi].range) {
            formation.members[bi].close_on = Some(at);
            state.closing.push(bi);
        }
        added += 1;
    }
    if added > 0 {
        tracing::info!(
            "Battle help: attacker={} victim={} tick={} helpers={}/{} crew={:?}",
            attacker.0,
            victim.0,
            tick,
            helping + added,
            HELPERS,
            crew.map(|helper| helper.map(|id| id.0))
        );
    }
}

#[cfg(test)]
impl Retaliation {
    /// The attacker `soldier` answers, and whether he only helps.
    pub(crate) fn answering(&self, soldier: EntityId) -> Option<(EntityId, bool)> {
        self.grudges
            .get(&soldier)
            .filter(|grudge| grudge.answering)
            .map(|grudge| (grudge.attacker, grudge.helping))
    }
}

#[cfg(test)]
#[path = "retaliation_tests.rs"]
mod tests;
