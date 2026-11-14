//! Regiment volley clocks and drag-aware aimed arrows. Flight/hits stay in the
//! native projectile broadphase; there is no separate battle collision scan.
use super::combat::{Action, CombatRuntime, Weapon};
use super::{ArrowLaunch, BattleState, Frame, Output};
use crate::boundary::{EntityEvent, Hand};
use mc_physics::mth;
use rustc_hash::FxHashMap;

pub const SUSTAINED_TICKS: u64 = 220; // 5.45/minute
const DRAW_TICKS: u64 = 30;
const MAX_SPEED: f64 = 4.5;

#[derive(Debug, Default)]
pub(super) struct Runtime {
    volleys: FxHashMap<(u16, u32), Volley>,
}

#[derive(Debug)]
struct Volley {
    next: u64,
    loose: bool,
    draw: bool,
    seen: u64,
}

/// Twenty regiment lanes, interleaved between opposing factions. This spreads
/// releases without scattering the soldiers of a regiment across different ticks.
fn phase((faction, regiment): (u16, u32)) -> u64 {
    u64::from(regiment % 20) * (SUSTAINED_TICKS / 20) + u64::from(faction % 2) * 5
}

impl Volley {
    fn new(now: u64, key: (u16, u32)) -> Self {
        // Rejoining after wavering must keep the regiment's absolute phase.
        let delay = (phase(key) + SUSTAINED_TICKS - now % SUSTAINED_TICKS) % SUSTAINED_TICKS;
        Self {
            next: now + DRAW_TICKS + delay,
            loose: false,
            draw: delay == 0,
            seen: now,
        }
    }
    fn update(&mut self, tick: u64) {
        self.draw = tick + DRAW_TICKS == self.next;
        self.loose = tick == self.next;
        if tick >= self.next {
            self.next = tick + SUSTAINED_TICKS;
        }
    }
}

/// Solve the discrete native air trajectory: p+=v; v*=.99; vy-=.05.
/// Eight bounded bisections on the low arc (<=90 ticks), not an expensive
/// per-tick integration or a no-drag ballistic approximation. At 200 blocks
/// this gives ~4.5 blocks/tick and ~24 degrees. Unreachable targets do not debit ammo.
pub fn aimed_arc(from: [f64; 3], to: [f64; 3]) -> Option<([f64; 3], u32)> {
    let delta = std::array::from_fn::<_, 3, _>(|i| to[i] - from[i]);
    let horizontal = mth::length2(delta[0], delta[2]);
    if !horizontal.is_finite() || !delta[1].is_finite() || horizontal < 1.0 {
        return None;
    }
    let velocity = |ticks: f64| {
        let sum = (1.0 - 0.99_f64.powf(ticks)) / 0.01;
        [
            delta[0] / sum,
            (delta[1] + 5.0 * (ticks - sum)) / sum,
            delta[2] / sum,
        ]
    };
    let speed_squared = |v: [f64; 3]| v.iter().map(|x| x * x).sum::<f64>();
    let mut high = 90.0;
    if speed_squared(velocity(high)) > MAX_SPEED * MAX_SPEED {
        return None;
    }
    let mut low = 1.0;
    for _ in 0..8 {
        let mid = (low + high) * 0.5;
        if speed_squared(velocity(mid)) > MAX_SPEED * MAX_SPEED {
            low = mid;
        } else {
            high = mid;
        }
    }
    // Integer arrival time makes the analytic position identical to flight's steps.
    Some((velocity(high.ceil()), 8))
}

pub(super) fn step(
    frame: &Frame<'_>,
    battle: &mut BattleState,
    combat: &mut CombatRuntime,
    out: &mut Output,
) {
    for volley in combat.archery.volleys.values_mut() {
        volley.update(frame.tick);
    }
    for bi in 0..battle.columns.ids.len() {
        if !battle.columns.active[bi] || battle.columns.carried[bi] {
            continue;
        }
        let slot = battle.columns.slots[bi].expect("active column");
        let body = frame.entities.body(slot);
        let fighter = &mut combat.fighters[bi];
        if fighter.weapon != Weapon::Bow
            || fighter.arrows == 0
            || matches!(fighter.action, Action::Wounded | Action::Stagger { .. })
        {
            continue;
        }
        let context = super::combat::context(battle, bi);
        if context.is_some_and(|c| c.routing || c.wavering) {
            continue;
        }
        let key = (body.faction, context.map_or(0, |c| c.regiment));
        let volley = combat
            .archery
            .volleys
            .entry(key)
            .or_insert_with(|| Volley::new(frame.tick, key));
        volley.seen = frame.tick;
        if !volley.loose && !volley.draw {
            continue;
        }
        let Some(target) = battle.columns.targets[bi].as_ref() else {
            continue;
        };
        combat.work.target_checks += 1;
        if super::retained_enemy(
            frame.entities,
            slot,
            target.id,
            f64::from(frame.entities.row(slot).attack_reach),
        )
        .is_none()
        {
            continue;
        }
        if volley.draw {
            let crossbow = frame.entities.row(slot).appearance.equipment[0]
                .as_ref()
                .and_then(|item| frame.registry.items.name_of(item.item_id))
                .is_some_and(|name| name.as_str().ends_with("crossbow"));
            out.events.push(EntityEvent::Swing {
                entity: body.id,
                hand: Hand::Main,
                animation: crate::boundary::SwingKind::Whack,
                duration_ticks: if crossbow { 75 } else { 40 },
            });
            continue;
        }
        let mut from = frame.entities.pos(slot);
        from[1] += frame.entities.body(slot).height * 0.85;
        let mut to = target.pos;
        to[1] += 0.8;
        combat.work.arc_solves += 1;
        let Some((velocity, iterations)) = aimed_arc(from, to) else {
            continue;
        };
        combat.work.arc_iterations += u64::from(iterations);
        fighter.arrows -= 1;
        combat.work.arrows += 1;
        out.arrows.push(ArrowLaunch {
            owner: body.id,
            from,
            velocity,
            damage: 2.0,
        });
        battle.formation.report_pressure(
            target.id,
            super::morale::Pressure {
                missiles: true,
                ..Default::default()
            },
        );
    }
    // One retained clock per active archer regiment, not one timer per arrow.
    combat
        .archery
        .volleys
        .retain(|_, volley| volley.seen == frame.tick);
}

#[cfg(test)]
#[path = "archery_tests.rs"]
mod tests;
