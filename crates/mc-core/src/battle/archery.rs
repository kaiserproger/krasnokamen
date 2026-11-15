//! Regiment volley clocks and drag-aware aimed arrows. Flight/hits stay in the
//! native projectile broadphase; there is no separate battle collision scan.
//! Each regiment draws together, then releases original-rank groups over five
//! ticks. The 220-tick cadence and ammunition are unchanged; spawn, journal and
//! view work no longer arrive as one whole-regiment release.
use super::combat::{Action, CombatRuntime, Weapon};
use super::{ArrowLaunch, BattleState, Frame, Output};
use crate::boundary::{EntityEvent, Hand};
use mc_physics::mth;
use rustc_hash::FxHashMap;

pub const SUSTAINED_TICKS: u64 = 220; // 5.45/minute
const DRAW_TICKS: u64 = 30;
const RIPPLE_TICKS: u64 = 5;
const MAX_SPEED: f64 = 4.5;
// Native flight deals ceil(speed * base_damage), not base_damage HP. At the
// 4.5-block/tick release cap this is one non-critical HP before mitigation;
// the old base 2.0 launched nine-HP hits and routed lines before sustained melee.
const BATTLE_ARROW_BASE_DAMAGE: f32 = 0.2;
// About one arrow in seventy finds a gap in the armour. Base 25 lands above
// twenty HP through iron armour on Easy scaling down to a 1.8-block/tick
// impact (a 90-tick flight), so such an arrow kills where it strikes a body.
// One in seventeen emptied whole regiments before the lines met.
pub(super) const LETHAL_ARROW_ROLLS: u32 = 150;
pub(super) const LETHAL_ARROW_BASE_DAMAGE: f32 = 25.0;

/// Stateless per-shot choice: stable across worker count and hash order.
pub(super) fn arrow_damage(owner: crate::boundary::EntityId, tick: u64) -> f32 {
    if super::combat::roll(owner, tick, 0x4b49_4c4c) < LETHAL_ARROW_ROLLS {
        LETHAL_ARROW_BASE_DAMAGE
    } else {
        BATTLE_ARROW_BASE_DAMAGE
    }
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Runtime {
    #[serde(with = "super::persistence::volley_map")]
    volleys: FxHashMap<(u16, u32), Volley>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Volley {
    next: u64,
    loose: bool,
    ripple: Option<u64>,
    draw: bool,
    seen: u64,
}

/// Twenty regiment lanes, interleaved between opposing factions. Each lane
/// leaves room for a five-tick rank ripple before the next regiment releases.
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
            ripple: None,
            draw: delay == 0,
            seen: now,
        }
    }
    fn update(&mut self, tick: u64) {
        self.draw = tick + DRAW_TICKS == self.next;
        self.loose = tick == self.next;
        // `next` advances at the first release. Only an already-started ripple
        // may use its previous deadline; a newly admitted regiment must draw first.
        // Absolute ticks also avoid replaying missed ranks after a skipped tick.
        self.ripple = if self.loose {
            Some(0)
        } else {
            self.ripple
                .and(self.next.checked_sub(SUSTAINED_TICKS))
                .and_then(|released| tick.checked_sub(released))
                .filter(|&offset| offset < RIPPLE_TICKS)
        };
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
        // Two main weapons (weapons.rs): no draw while the bow comes up.
        if fighter.arms.swapping(frame.tick) {
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
        if volley.ripple.is_none() && !volley.draw {
            continue;
        }
        if !volley.draw {
            // Dense formation prefix: one indexed read, no per-soldier timer or
            // hash lookup. Original rank prevents double shots during front relief.
            let rank = battle
                .formation
                .members
                .get(bi)
                .map_or(0, |m| m.original_rank);
            if volley.ripple != Some(u64::from(rank) % RIPPLE_TICKS) {
                continue;
            }
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
            damage: arrow_damage(body.id, frame.tick),
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

#[cfg(test)]
mod ripple_tests {
    use super::super::combat::tests::{fixture, index, prepare};
    use super::super::formation::{Order, RegimentSpec};
    use super::*;
    use crate::mounts::tests::{slot, spawn};

    #[test]
    fn ripple_clock_requires_first_draw_and_does_not_replay_skipped_ticks() {
        let mut volley = Volley::new(743, (1, 19));
        let first = volley.next;
        for tick in 743..first {
            volley.update(tick);
            assert_eq!(volley.ripple, None, "premature release at {tick}");
        }
        volley.update(first);
        assert_eq!(volley.ripple, Some(0));
        volley.update(first + 3);
        assert_eq!(volley.ripple, Some(3), "missed ranks are not replayed");
        volley.update(first + 5);
        assert_eq!(volley.ripple, None);
        volley.update(first + SUSTAINED_TICKS);
        assert_eq!(volley.ripple, Some(0));
    }

    #[test]
    fn registered_ranks_ripple_over_five_ticks_without_duplicate_or_missing_shots() {
        let (mut state, _, pos) = fixture();
        let target = spawn(&mut state, "husk", [pos[0] + 40.0, pos[1], pos[2]]);
        let target_slot = slot(&state, target);
        state.entities.set_faction(target_slot, 2);
        let bow = state
            .registry
            .items
            .id_of(&mc_data::Identifier::parse("minecraft:bow").unwrap())
            .unwrap();
        let mut battle = BattleState::default();
        let mut archers = Vec::new();
        for _ in 0..40 {
            let id = spawn(&mut state, "husk", pos);
            let s = slot(&state, id);
            state.entities.set_faction(s, 1);
            state.entities.row_mut(s).attack_reach = 220.0;
            std::sync::Arc::make_mut(&mut state.entities.row_mut(s).appearance).equipment[0] =
                Some(mc_data::ItemStack::new(bow, 1));
            archers.push(id);
        }
        battle
            .formation
            .add_regiment(
                RegimentSpec {
                    id: 0,
                    faction: 1,
                    origin: pos,
                    facing: [1.0, 0.0],
                    files: 4,
                    frontage: 1.0,
                    order: Order::Hold,
                },
                &archers,
            )
            .unwrap();
        prepare(&state, &mut battle, 0);
        for &id in &archers {
            let bi = index(&state, &battle, id);
            battle.columns.targets[bi] = Some(super::super::EnemyTarget {
                id: target,
                pos: state.entities.pos(target_slot),
                distance: 40.0,
            });
        }
        let first = phase((1, 0)) + DRAW_TICKS;
        let mut released = vec![Vec::new(); archers.len()];
        let mut release_ticks = 0;
        for tick in 0..first + SUSTAINED_TICKS + 5 {
            if tick == first + 1 {
                // Front relief can change current ranks during a ripple.
                // Scheduling must keep original ranks or a soldier can fire twice.
                for member in &mut battle.formation.members {
                    member.rank = (member.original_rank + 2) % 10;
                }
            }
            let frame = Frame {
                entities: &state.entities,
                registry: &state.registry,
                world: &state.world,
                tick,
            };
            let mut out = Output::default();
            super::super::combat::step(&frame, &mut battle, &mut out);
            if !out.arrows.is_empty() {
                release_ticks += 1;
                assert_eq!(out.arrows.len(), 8, "unbounded volley at tick {tick}");
            }
            assert!(battle.combat.work.arc_solves <= 8);
            assert!(battle.combat.work.arc_iterations <= 64);
            for arrow in out.arrows {
                let archer = archers.iter().position(|&id| id == arrow.owner).unwrap();
                released[archer].push(tick);
            }
        }
        assert_eq!(release_ticks, 10, "two five-tick volleys");
        for (i, shots) in released.iter().enumerate() {
            let expected = first + (i / 4 % 5) as u64;
            assert_eq!(*shots, [expected, expected + SUSTAINED_TICKS]);
            assert_eq!(
                battle.combat.fighters[index(&state, &battle, archers[i])].arrows,
                58
            );
        }
    }

    /// Same flat-world staging and warmup as battle_phase_micro, with the real
    /// native journal producer included. No network, checkpoint writer or TPS claim.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "20k in-process volley diagnostic; frozen release executable in mem-run -p"]
    fn twenty_thousand_volley_journal_views_micro() {
        use crate::battle::staging::{BattleCommand, execute};
        fn cpu_ticks() -> u64 {
            let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
            let mut fields = stat.rsplit_once(')').unwrap().1.split_whitespace().skip(11);
            fields.next().unwrap().parse::<u64>().unwrap()
                + fields.next().unwrap().parse::<u64>().unwrap()
        }
        assert!(std::env::var_os("MC_CORE_PHASE_LOG").is_some());
        let registry = std::sync::Arc::new(crate::registry::CoreRegistry::load(None).unwrap());
        let world = crate::horde_bench::flat_world(&registry);
        let mut state = crate::tick::TickState::new(registry, world, 2, 512);
        let mut changes = crate::boundary::ChangeSet::default();
        let origin = [8.5, 64.0, 8.5];
        execute(
            &mut state,
            &mut changes,
            origin,
            &BattleCommand::Setup {
                seed: 42,
                count: 10_000,
            },
        )
        .unwrap();
        state.world.ensure_blocking(&state.desired_chunks());
        state.refresh_active_chunks(state.tick);
        for _ in 0..208 {
            let (output, _, _) = crate::tick::run_tick(&mut state, &[]);
            crate::core::journal_entries_for_tick(&mut state, &output).unwrap();
        }
        execute(&mut state, &mut changes, origin, &BattleCommand::Start).unwrap();
        let mut samples = Vec::new();
        let mut total_arrows = 0;
        for _ in 0..220 {
            let cpu_before = cpu_ticks();
            let (output, _, times) = crate::tick::run_tick(&mut state, &[]);
            let start = std::time::Instant::now();
            let entries = crate::core::journal_entries_for_tick(&mut state, &output).unwrap();
            let journal_us = start.elapsed().as_micros() as u64;
            let cpu_used = cpu_ticks() - cpu_before;
            let arrows = state.hostile.battle.combat.work.arrows;
            if arrows > 0 {
                let emit_us = state.hostile.battle.timings.emit_us;
                eprintln!(
                    "VOLLEY_SAMPLE tick={} arrows={arrows} journal_build_us={journal_us} views_us={} emit_us={emit_us} cpu_ticks={cpu_used} journal_entries={}",
                    output.tick,
                    times.views_us,
                    entries.len()
                );
                samples.push((arrows, journal_us, times.views_us, emit_us));
                total_arrows += arrows;
            }
        }
        assert!(
            !samples.is_empty(),
            "measurement must observe actual native arrow launches"
        );
        eprintln!(
            "VOLLEY_SUMMARY side=10000 seed=42 ticks=220 volley_ticks={} arrows={total_arrows} max_arrows={} max_journal_build_us={} max_views_us={} max_emit_us={}",
            samples.len(),
            samples.iter().map(|s| s.0).max().unwrap(),
            samples.iter().map(|s| s.1).max().unwrap(),
            samples.iter().map(|s| s.2).max().unwrap(),
            samples.iter().map(|s| s.3).max().unwrap()
        );
    }
}
