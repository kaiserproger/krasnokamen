//! Two main weapons. A staged soldier carries a spear and a sword, an archer a
//! bow and a sword, and holds the one the distance to his enemy calls for: the
//! spear on the approach, against horsemen and from the second rank; the sword
//! in the press once his enemy is inside the spear's near bound; the bow at a
//! distance and the sword once an enemy closes.
//!
//! Mobs have no inventory: the weapon not in hand stays in the fighter's battle
//! state (staging hands it over with the item components of its kit), never a
//! persistence surface. The weapon in hand is the native main-hand equipment,
//! so a swap is an ordinary main-hand change: native appearance custody
//! journals it, clients get `SetEquipment`, and the row's wire change
//! re-derives the combat classification (weapon, melee range, archer) on the
//! next tick through `Columns::equipment_dirty`.

use rustc_hash::FxHashMap;

use super::combat::{Action, CombatRuntime, Weapon};
use super::faction::{in_melee_range, melee_reach, reach_bound};
use mc_melee_range::MeleeReach;
use super::{BattleState, Frame, Output, Unit};
use crate::boundary::{EntityEvent, EntityId, Hand, SwingKind};
use crate::tick::TickState;
use mc_data::ItemStack;

/// How far inside the spear's near bound (blocks of box growth) an enemy must
/// come before a soldier who fights spear first (second rank, or against a
/// horseman) draws the sword: five ticks of a pulsing soldier's 0.10-block
/// step, so the iron spear's 1.0 bound becomes 0.5, inside the sword's own
/// 0.83 reach.
pub(super) const BAND: f64 = 0.5;

/// An archer takes his sword once an enemy is this close (blocks, centre to
/// centre): the melee engagement reach, where a soldier acquires his man and
/// closes. A pursuer (0.28 block/tick) covers it in 21 ticks, inside one
/// 30-tick bow draw, so another draw would be cut down; the sword is up in 13.
pub(super) const ARCHER_SWORD: f64 = super::MELEE_ENGAGEMENT_REACH;

/// ...and the bow again once no enemy is within this: a pulsing foot soldier
/// (0.10 block/tick) covers 3.5 blocks in the 5-tick bow recovery plus the
/// 30-tick draw, so the shot looses before he is back inside `ARCHER_SWORD`.
pub(super) const ARCHER_BOW: f64 = 10.0;

/// What a kit pairs: the held weapon's class decides which of the two is in hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pair {
    SpearSword,
    BowSword,
}

/// Second weapons handed over by staging, claimed once by each fighter.
#[derive(Debug, Default)]
pub(crate) struct Arsenal(FxHashMap<EntityId, (ItemStack, Pair)>);

impl Arsenal {
    pub(crate) fn stow(&mut self, id: EntityId, stack: ItemStack, pair: Pair) {
        self.0.insert(id, (stack, pair));
    }

    #[cfg(test)]
    pub(super) fn get(&self, id: EntityId) -> Option<&(ItemStack, Pair)> {
        self.0.get(&id)
    }
}

/// One fighter's weapon not in hand and his swap clock.
#[derive(Debug)]
pub(super) struct Arms {
    claimed: bool,
    /// The weapon not in hand, with the kit it forms with the held one.
    stowed: Option<Box<(ItemStack, Pair)>>,
    /// The spear's melee reach (`faction::melee_reach`) of a spear-and-sword
    /// kit: its near bound draws the sword, its reach puts the sword away.
    spear: MeleeReach,
    /// First tick the drawn weapon may strike or loose.
    ready: u64,
}

impl Default for Arms {
    fn default() -> Self {
        Self {
            claimed: false,
            stowed: None,
            spear: MeleeReach::mob(None),
            ready: 0,
        }
    }
}

impl Arms {
    /// The drawn weapon is still coming up: no windup, blow or draw.
    pub(super) fn swapping(&self, tick: u64) -> bool {
        tick < self.ready
    }

    #[cfg(test)]
    pub(super) fn stowed(&self) -> Option<&ItemStack> {
        self.stowed.as_deref().map(|(stack, _)| stack)
    }
}

/// Whether a spear-and-sword soldier wants the sword, from what he holds and
/// `inside(grow)`: his enemy's box is within `grow` of his (the combat melee
/// range test). The sword comes out once the enemy is inside the spear's near
/// bound, where the spear cannot strike (spear first, from the second rank
/// past the front man or against a horseman: only `BAND` further in). It goes
/// back only once the enemy is beyond the spear's reach: the pulses of a melee
/// (surges of 2.25 blocks) stay inside that hysteresis and never flip it.
pub(super) fn wants_sword(
    holding_sword: bool,
    spear_first: bool,
    spear: MeleeReach,
    inside: impl Fn(f64) -> bool,
) -> bool {
    let bound = match (holding_sword, spear_first) {
        (true, _) => spear.max,
        (false, false) => spear.min,
        (false, true) => spear.min - BAND,
    };
    bound > 0.0 && inside(bound)
}

/// Vanilla resets attack strength when the main-hand item changes
/// (`Player.resetAttackStrengthTicker`); a full blow waits 20 / attack speed
/// ticks of the drawn item: 13 for an iron sword (1.6), 19 for an iron spear
/// (1.05; its `minimum_attack_charge` 1 makes that a hard wait), 5 for a bow
/// (base 4.0).
pub(super) fn recovery_ticks(frame: &Frame<'_>, stack: &ItemStack) -> u64 {
    let modifier = frame
        .registry
        .items
        .name_of(stack.item_id)
        .and_then(|name| frame.registry.item_facts.get(name))
        .and_then(|facts| facts.attack_speed_modifier)
        .unwrap_or(0.0);
    (20.0 / (4.0 + f64::from(modifier)).max(0.2)).ceil() as u64
}

/// Decide and start swaps after classification and before any windup or
/// volley of this tick; count who holds what.
pub(super) fn step(
    frame: &Frame<'_>,
    battle: &BattleState,
    combat: &mut CombatRuntime,
    out: &mut Output,
) {
    let (mut swaps, mut spears, mut swords, mut bows) = (0, 0, 0, 0);
    for bi in 0..battle.columns.ids.len() {
        if !battle.columns.active[bi] || battle.columns.carried[bi] {
            continue;
        }
        let id = battle.columns.ids[bi];
        let slot = battle.columns.slots[bi].expect("active column");
        let fighter = &mut combat.fighters[bi];
        match fighter.weapon {
            Weapon::Spear => spears += 1,
            Weapon::Bow => bows += 1,
            Weapon::Sword | Weapon::Mace => swords += 1,
        }
        if !fighter.arms.claimed {
            fighter.arms.claimed = true;
            if let Some((stack, pair)) = combat.arsenal.0.remove(&id) {
                fighter.arms.spear = if fighter.weapon == Weapon::Spear {
                    fighter.range
                } else {
                    melee_reach(
                        frame
                            .registry
                            .items
                            .name_of(stack.item_id)
                            .and_then(|name| frame.registry.item_facts.get(name))
                            .and_then(|facts| facts.attack_range),
                    )
                };
                fighter.arms.stowed = Some(Box::new((stack, pair)));
            }
        }
        let Some(pair) = fighter.arms.stowed.as_deref().map(|&(_, pair)| pair) else {
            continue;
        };
        if fighter.arms.swapping(frame.tick) || !matches!(fighter.action, Action::Guard { .. }) {
            continue;
        }
        // A fugitive runs with what he holds.
        let context = super::combat::context(battle, bi);
        if context.is_some_and(|c| c.routing) && !battle.formation.cornered_at(bi) {
            continue;
        }
        let swap = match pair {
            Pair::SpearSword => {
                let holding_sword = fighter.weapon != Weapon::Spear;
                let second_rank =
                    context.is_some_and(|c| !c.front_rank) && !battle.formation.loose_order_at(bi);
                let spear = fighter.arms.spear;
                let unit = Unit {
                    id,
                    faction: battle.columns.factions[bi],
                    pos: battle.columns.positions[bi],
                };
                // His opponent as in the strike decision: the retained soldier
                // target, or a nearer live holder of the other side. A sword in
                // hand stays out while any enemy is within the spear's reach,
                // also between retargets when his own man has just fallen.
                let soldier = battle.columns.targets[bi].or_else(|| {
                    holding_sword
                        .then(|| {
                            let reach = MeleeReach {
                                min: 0.0,
                                max: spear.max,
                            };
                            battle.grid.nearest_enemy(unit, reach_bound(reach)).target
                        })
                        .flatten()
                });
                let holder = super::live_players::in_reach(
                    battle,
                    frame.entities,
                    slot,
                    MeleeReach {
                        min: 0.0,
                        max: spear.max,
                    },
                    None,
                )
                .filter(|holder| soldier.is_none_or(|soldier| holder.distance < soldier.distance));
                let wants = holder
                    .or(soldier)
                    .and_then(|opponent| frame.entities.slot_of(opponent.id))
                    .is_some_and(|enemy| {
                        let mounted = frame
                            .entities
                            .row(enemy)
                            .entity_life
                            .mount
                            .vehicle
                            .is_some();
                        wants_sword(holding_sword, second_rank || mounted, spear, |grow| {
                            in_melee_range(
                                frame.entities,
                                slot,
                                enemy,
                                MeleeReach {
                                    min: 0.0,
                                    max: grow,
                                },
                            )
                        })
                    });
                wants != holding_sword
            }
            // The nearest-enemy grid query runs on the retarget phase only.
            Pair::BowSword if super::retarget_due(id, frame.tick) => {
                let holding_bow = fighter.weapon == Weapon::Bow;
                let enemy_near = battle
                    .grid
                    .nearest_enemy(
                        Unit {
                            id,
                            faction: battle.columns.factions[bi],
                            pos: battle.columns.positions[bi],
                        },
                        if holding_bow {
                            ARCHER_SWORD
                        } else {
                            ARCHER_BOW
                        },
                    )
                    .target
                    .is_some();
                enemy_near == holding_bow
            }
            Pair::BowSword => false,
        };
        if !swap {
            continue;
        }
        let Some(held) = frame.entities.row(slot).appearance.equipment[0].clone() else {
            continue;
        };
        let stowed = fighter.arms.stowed.as_deref_mut().expect("kit checked");
        let drawn = std::mem::replace(&mut stowed.0, held);
        let pause = recovery_ticks(frame, &drawn);
        fighter.arms.ready = frame.tick + pause;
        out.events.push(EntityEvent::Swing {
            entity: id,
            hand: Hand::Main,
            animation: SwingKind::Whack,
            duration_ticks: pause as u16,
        });
        out.weapons.push((id, drawn));
        swaps += 1;
    }
    combat.work.weapon_swaps = swaps;
    combat.work.holding_spear = spears;
    combat.work.holding_sword = swords;
    combat.work.holding_bow = bows;
}

/// Put the drawn weapon in the main hand through native appearance custody:
/// journaled and published like any equipment change (trackers get
/// `SetEquipment`).
pub(super) fn hold(state: &mut TickState, id: EntityId, stack: ItemStack) {
    let Some(slot) = state.entities.slot_of(id) else {
        return;
    };
    std::sync::Arc::make_mut(&mut state.entities.row_mut(slot).appearance).equipment[0] =
        Some(stack);
    state.appearance_changed.push(id);
}

#[cfg(test)]
#[path = "weapons_tests.rs"]
mod tests;
