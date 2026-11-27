use super::*;
use crate::mounts::tests::{slot, spawn};
use crate::tick::TickState;

pub(in crate::battle) fn fixture() -> (TickState, EntityId, [f64; 3]) {
    let (mut state, player, pos) = crate::mounts::tests::fixture();
    // Loading terrain does not activate its clocks. Kernel-only tests must
    // explicitly admit the same player-demand chunks as the native tick.
    let active = state.desired_chunks().into_keys().collect();
    state.world.set_active_chunks(active, state.tick);
    (state, player, pos)
}

pub(in crate::battle) fn prepare(state: &TickState, battle: &mut BattleState, tick: u64) {
    let frame = Frame {
        entities: &state.entities,
        registry: &state.registry,
        world: &state.world,
        tick,
    };
    battle.columns.prepare(
        &frame,
        &mut battle.formation,
        &mut battle.output.counters,
        &mut battle.population,
    );
    battle.formation.finish_preparation();
    battle.combat.prepare(&battle.columns.ids);
}

pub(in crate::battle) fn index(state: &TickState, battle: &BattleState, id: EntityId) -> usize {
    battle.columns.index(slot(state, id)).unwrap()
}

/// Sets a fighter's private shield block for battle tests outside combat.
pub(in crate::battle) fn set_block(fighter: &mut Fighter, block: Option<(EntityId, u64)>) {
    fighter.block = block;
}

fn pair() -> (TickState, BattleState, EntityId, EntityId) {
    let (mut state, _, pos) = fixture();
    let actor = spawn(&mut state, "husk", pos);
    // One block apart: inside the vanilla mob melee reach without an
    // attack_range component (boxes grown by sqrt(2.04) - 0.6).
    let victim = spawn(&mut state, "husk", [pos[0] + 1.0, pos[1], pos[2]]);
    let a = slot(&state, actor);
    let b = slot(&state, victim);
    state.entities.set_faction(a, 1);
    state.entities.set_faction(b, 2);
    let mut battle = BattleState::default();
    prepare(&state, &mut battle, 0);
    let bi = index(&state, &battle, actor);
    battle.columns.targets[bi] = Some(super::super::EnemyTarget {
        id: victim,
        pos: state.entities.pos(b),
        distance: 1.0,
    });
    (state, battle, actor, victim)
}

fn step_at(state: &TickState, battle: &mut BattleState, tick: u64) -> Output {
    prepare(state, battle, tick);
    let frame = Frame {
        entities: &state.entities,
        registry: &state.registry,
        world: &state.world,
        tick,
    };
    let mut out = Output::default();
    step(&frame, battle, &mut out);
    out
}

#[test]
fn weapon_cycles_emit_full_duration_and_only_strike_on_contact_tick() {
    for weapon in [Weapon::Sword, Weapon::Spear, Weapon::Mace] {
        let (mut state, mut battle, actor, victim) = pair();
        let name = match weapon {
            Weapon::Sword => "minecraft:iron_sword",
            Weapon::Spear => "minecraft:trident",
            Weapon::Mace => "minecraft:mace",
            Weapon::Bow => unreachable!(),
        };
        let item = state
            .registry
            .items
            .id_of(&mc_data::Identifier::parse(name).unwrap())
            .unwrap();
        let actor_slot = slot(&state, actor);
        std::sync::Arc::make_mut(&mut state.entities.row_mut(actor_slot).appearance).equipment[0] =
            Some(mc_data::ItemStack::new(item, 1));
        let (start, out) = (0..1000)
            .find_map(|tick| {
                let out = step_at(&state, &mut battle, tick);
                (!out.events.is_empty()).then_some((tick, out))
            })
            .expect("deterministic attacker starts a swing");
        assert!(out.events.contains(&EntityEvent::Swing {
            entity: actor,
            hand: Hand::Main,
            animation: if weapon == Weapon::Spear {
                mc_domain::SwingKind::Stab
            } else {
                mc_domain::SwingKind::Whack
            },
            duration_ticks: i32::from(weapon.cycle())
        }));
        assert_eq!(weapon.cycle(), 2 * weapon.windup() + 1);
        let impact = start + u64::from(weapon.windup());
        for tick in start + 1..start + u64::from(weapon.cycle()) {
            let _ = step_at(&state, &mut battle, tick);
            assert_eq!(
                battle.combat.work.strikes,
                u64::from(tick == impact),
                "{weapon:?} at {tick}"
            );
            assert!(battle.combat.work.visited <= 2);
            assert!(battle.combat.work.target_checks <= 2);
        }
        assert!(matches!(
            battle.combat.fighters[index(&state, &battle, actor)].action,
            Action::Recover { .. }
        ));
        assert_eq!(
            battle.columns.target(slot(&state, actor)).unwrap().id,
            victim
        );
    }
}

#[test]
fn shield_window_is_inclusive_and_parry_staggers_without_damage() {
    for remaining in 0..10 {
        assert_eq!(block_window(remaining), (2..=6).contains(&remaining));
    }
    for ready in [0, 8, 9] {
        let (mut state, mut battle, actor, victim) = pair();
        let shield = state
            .registry
            .items
            .id_of(&mc_data::Identifier::parse("minecraft:shield").unwrap())
            .unwrap();
        let v = slot(&state, victim);
        std::sync::Arc::make_mut(&mut state.entities.row_mut(v).appearance).equipment[1] =
            Some(mc_data::ItemStack::new(shield, 1));
        let start = (0..1000)
            .find(|s| roll(victim, *s, actor.0 as u64) < 6500)
            .unwrap();
        let a = index(&state, &battle, actor);
        let b = index(&state, &battle, victim);
        battle.combat.fighters[a] = Fighter {
            action: Action::Windup {
                target: victim,
                start,
                weapon: Weapon::Sword,
            },
            ..Default::default()
        };
        battle.combat.fighters[b] = Fighter {
            action: Action::Recover {
                until: start + ready,
            },
            ..Default::default()
        };
        let mut reactions = 0;
        for tick in start..start + 10 {
            let _ = step_at(&state, &mut battle, tick);
            reactions += battle.combat.work.block_checks;
        }
        let out = step_at(&state, &mut battle, start + 10);
        let blocks = u64::from(ready <= 8);
        assert_eq!(reactions, blocks);
        assert_eq!(battle.combat.work.parries, blocks);
        if blocks != 0 {
            assert!(out.intents.is_empty());
            assert!(out.events.contains(&EntityEvent::ShieldBlock {
                entity: victim,
                disabled: false
            }));
            assert!(out.events.contains(&EntityEvent::Swing {
                entity: actor,
                hand: Hand::Main,
                animation: mc_domain::SwingKind::None,
                duration_ticks: 18
            }));
            assert_eq!(
                battle.combat.fighters[a].action,
                Action::Stagger { until: start + 28 }
            );
        }
    }
}

fn living_flags(state: &TickState, id: EntityId) -> Option<i8> {
    state
        .entities
        .row(slot(state, id))
        .appearance
        .extra
        .iter()
        .find_map(|(index, value)| match (*index, value) {
            (8, mc_data::entity_data_value::EntityDataValue::Byte(flags)) => Some(*flags),
            _ => None,
        })
}

#[test]
fn shield_bearer_raises_on_guard_and_lowers_to_swing_or_when_wounded() {
    let (mut state, mut battle, actor, victim) = pair();
    let item = |state: &TickState, name: &str| {
        state
            .registry
            .items
            .id_of(&mc_data::Identifier::parse(name).unwrap())
            .unwrap()
    };
    let (sword, shield) = (
        item(&state, "minecraft:iron_sword"),
        item(&state, "minecraft:shield"),
    );
    let a = slot(&state, actor);
    let equipment =
        &mut std::sync::Arc::make_mut(&mut state.entities.row_mut(a).appearance).equipment;
    equipment[0] = Some(mc_data::ItemStack::new(sword, 1));
    equipment[1] = Some(mc_data::ItemStack::new(shield, 1));
    // First decision publishes every fighter once; only the bearer raises.
    let out = step_at(&state, &mut battle, 0);
    assert!(out.shields.contains(&(actor, true)), "{:?}", out.shields);
    assert!(out.shields.contains(&(victim, false)), "{:?}", out.shields);
    let mut swing = None;
    for tick in 1..1000 {
        let out = step_at(&state, &mut battle, tick);
        if out
            .events
            .iter()
            .any(|e| matches!(e, EntityEvent::Swing { entity, .. } if *entity == actor))
        {
            assert_eq!(out.shields, [(actor, false)], "the swing lowers the shield");
            swing = Some(tick);
            break;
        }
        assert!(
            out.shields.is_empty(),
            "steady guard republishes nothing at {tick}"
        );
    }
    let start = swing.expect("deterministic attacker starts a swing");
    let cycle = u64::from(Weapon::Sword.cycle());
    for tick in start + 1..start + cycle {
        assert!(
            step_at(&state, &mut battle, tick).shields.is_empty(),
            "lowered through {tick}"
        );
    }
    // Recovery ends in guard: the shield goes back up.
    assert_eq!(
        step_at(&state, &mut battle, start + cycle).shields,
        [(actor, true)]
    );
    let bi = index(&state, &battle, actor);
    battle.combat.fighters[bi].action = Action::Wounded;
    assert_eq!(
        step_at(&state, &mut battle, start + cycle + 1).shields,
        [(actor, false)]
    );

    // Native presentation: vanilla living flags byte (index 8), using + off hand.
    state.appearance_changed.clear();
    super::super::show_shield(&mut state, actor, true);
    assert_eq!(living_flags(&state, actor), Some(0b11));
    assert_eq!(state.appearance_changed, [actor]);
    super::super::show_shield(&mut state, actor, true);
    assert_eq!(
        state.appearance_changed,
        [actor],
        "an unchanged pose is not republished"
    );
    super::super::show_shield(&mut state, actor, false);
    assert_eq!(living_flags(&state, actor), Some(0));
    assert_eq!(state.appearance_changed, [actor, actor]);
    // A fighter without a shield that never raised one costs no row write.
    super::super::show_shield(&mut state, victim, false);
    assert_eq!(living_flags(&state, victim), None);
    assert_eq!(state.appearance_changed, [actor, actor]);
}

#[test]
fn shield_guard_lowers_for_stagger_and_raises_after_recovery() {
    let (mut state, mut battle, actor, _) = pair();
    let shield = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:shield").unwrap())
        .unwrap();
    let slot = slot(&state, actor);
    std::sync::Arc::make_mut(&mut state.entities.row_mut(slot).appearance).equipment[1] =
        Some(mc_data::ItemStack::new(shield, 1));
    let bi = index(&state, &battle, actor);
    battle.columns.targets[bi] = None;
    assert!(
        step_at(&state, &mut battle, 0)
            .shields
            .contains(&(actor, true))
    );
    battle.combat.fighters[bi].action = Action::Stagger { until: 100 };
    assert_eq!(step_at(&state, &mut battle, 1).shields, [(actor, false)]);
    assert!(step_at(&state, &mut battle, 2).shields.is_empty());
    assert_eq!(step_at(&state, &mut battle, 100).shields, [(actor, true)]);
}

#[test]
fn shield_guard_lowers_for_rout_and_raises_after_rally() {
    use super::super::formation::{Order, RegimentSpec};
    let (mut state, mut battle, actor, _) = pair();
    let shield = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:shield").unwrap())
        .unwrap();
    let slot = slot(&state, actor);
    std::sync::Arc::make_mut(&mut state.entities.row_mut(slot).appearance).equipment[1] =
        Some(mc_data::ItemStack::new(shield, 1));
    battle
        .formation
        .add_regiment(
            RegimentSpec {
                id: 0,
                faction: 1,
                origin: state.entities.pos(slot),
                facing: [1.0, 0.0],
                files: 1,
                frontage: 1.0,
                order: Order::Hold,
            },
            &[actor],
        )
        .unwrap();
    prepare(&state, &mut battle, 0);
    let bi = index(&state, &battle, actor);
    battle.columns.targets[bi] = None;
    assert!(
        step_at(&state, &mut battle, 0)
            .shields
            .contains(&(actor, true))
    );
    battle.formation.members[bi].routing = true;
    assert_eq!(step_at(&state, &mut battle, 1).shields, [(actor, false)]);
    assert!(step_at(&state, &mut battle, 2).shields.is_empty());
    battle.formation.members[bi].routing = false;
    assert_eq!(step_at(&state, &mut battle, 3).shields, [(actor, true)]);
}

#[test]
fn calibration_spreads_wide_front_removals_over_the_press_and_seven_fatal_in_ten() {
    let mut counts = [0; 3];
    for sample in 0..10_000 {
        counts[match outcome(sample, false, 0.0) {
            Outcome::Glance => 0,
            Outcome::Wound => 1,
            Outcome::Kill => 2,
        }] += 1;
    }
    // A 21..25-tick cycle skips the next 20-tick attack decision.
    for weapon in [Weapon::Sword, Weapon::Spear, Weapon::Mace] {
        assert!((21..=40).contains(&weapon.cycle()));
    }
    let rate = 0.25 * (0.8 / 1.8) + 0.75 * (0.2 / 1.2);
    assert_eq!(counts, [9920, 24, 56]);
    let removal_probability = (counts[1] + counts[2]) as f64 / 10_000.0;
    // 80 removals per regiment in 120 s of contact, over fifty times the
    // 46-file front within reach.
    let expected = 46.0 * 50.0 * rate * 46.0 * (1.0 - 2316.0 / 3873.0) * removal_probability;
    // Within one roll in 10 000 of the ~80 removal rolls.
    assert!((expected - 80.0).abs() < 80.0 / 79.0, "{expected}");
    let fatal_share = counts[2] as f64 / f64::from(counts[1] + counts[2]);
    assert!((0.69..=0.71).contains(&fatal_share), "{fatal_share}");
    let lifetime = 1.0 / (rate * (1.0 - 2316.0 / 3873.0) * removal_probability);
    assert!(lifetime >= 8.0, "{lifetime}");
    assert_eq!(
        (0..10_000)
            .filter(|r| outcome(*r, true, 0.0) == Outcome::Kill)
            .count(),
        216
    );
}

#[test]
fn saturated_pursuit_kills_nine_casualties_in_ten() {
    // A charge bonus far past any real one (impact_bonus <= 2) saturates the
    // pursuit odds at the 0.8 % press lethality.
    let mut counts = [0; 3];
    for sample in 0..10_000 {
        counts[match outcome(sample, true, 50.0) {
            Outcome::Glance => 0,
            Outcome::Wound => 1,
            Outcome::Kill => 2,
        }] += 1;
    }
    assert_eq!(counts, [0, 1000, 9000]);
}

#[test]
fn stale_target_faction_change_cancels_damage_at_impact() {
    let (mut state, mut battle, actor, victim) = pair();
    let a = index(&state, &battle, actor);
    battle.combat.fighters[a] = Fighter {
        action: Action::Windup {
            target: victim,
            start: 0,
            weapon: Weapon::Sword,
        },
        ..Default::default()
    };
    let victim_slot = slot(&state, victim);
    state.entities.set_faction(victim_slot, 1);
    let out = step_at(&state, &mut battle, 10);
    assert!(out.intents.is_empty());
    assert_eq!(battle.combat.work.strikes, 0);
}

fn kill_intents(out: &Output, victim: EntityId) -> usize {
    out.intents
        .iter()
        .filter(|intent| {
            matches!(intent.kind, IntentKind::DamageEntity {
                target,
                kind: crate::survival::DamageKind::GenericKill,
                ..
            } if target == victim)
        })
        .count()
}

#[test]
fn landed_blow_finishes_a_withdrawing_wounded_man() {
    let (state, mut battle, actor, victim) = pair();
    // A swing whose own roll is only a wound: the wounded target still dies.
    let start = (0..10_000)
        .find(|&s| outcome(roll(actor, s, victim.0 as u64), false, 0.0) == Outcome::Wound)
        .unwrap();
    let a = index(&state, &battle, actor);
    let b = index(&state, &battle, victim);
    battle.combat.fighters[a].action = Action::Windup {
        target: victim,
        start,
        weapon: Weapon::Sword,
    };
    battle.combat.fighters[b].action = Action::Wounded;
    let out = step_at(
        &state,
        &mut battle,
        start + u64::from(Weapon::Sword.windup()),
    );
    assert_eq!(battle.combat.work.strikes, 1);
    assert_eq!(kill_intents(&out, victim), 1, "wounded man stayed immune");
    assert_eq!(battle.combat.work.killed, 1);
}

#[test]
fn second_attacker_in_one_pass_cannot_kill_the_reserved_casualty() {
    let (mut state, mut battle, actor, victim) = pair();
    let pos = state.entities.pos(slot(&state, victim));
    let second = spawn(&mut state, "husk", [pos[0], pos[1], pos[2] + 2.0]);
    let second_slot = slot(&state, second);
    state.entities.set_faction(second_slot, 1);
    prepare(&state, &mut battle, 0);
    let start = (0..1_000_000)
        .find(|&s| {
            [actor, second]
                .iter()
                .all(|id| outcome(roll(*id, s, victim.0 as u64), false, 0.0) == Outcome::Kill)
        })
        .unwrap();
    for id in [actor, second] {
        let bi = index(&state, &battle, id);
        battle.combat.fighters[bi].action = Action::Windup {
            target: victim,
            start,
            weapon: Weapon::Sword,
        };
    }
    let out = step_at(
        &state,
        &mut battle,
        start + u64::from(Weapon::Sword.windup()),
    );
    assert_eq!(kill_intents(&out, victim), 1);
    assert_eq!(battle.combat.work.killed, 1);
}

#[test]
fn twenty_thousand_idle_fighters_do_no_pair_or_arc_searches() {
    let (mut state, _, pos) = fixture();
    for _ in 0..20_000 {
        let id = spawn(&mut state, "husk", pos);
        let s = slot(&state, id);
        state.entities.set_faction(s, 1);
    }
    let mut battle = BattleState::default();
    let out = step_at(&state, &mut battle, 0);
    assert!(out.events.is_empty() && out.intents.is_empty() && out.arrows.is_empty());
    assert_eq!(battle.combat.work.visited, 20_000);
    assert_eq!(battle.combat.work.target_checks, 0);
    assert_eq!(battle.combat.work.block_checks, 0);
    assert_eq!(battle.combat.work.arc_iterations, 0);
    assert_eq!(battle.combat.fighters.len(), 20_000);
}

#[test]
fn dense_fighter_layout_changes_preserve_action_and_ammunition_by_identity() {
    let mut combat = CombatRuntime::default();
    combat.prepare(&[EntityId(10), EntityId(20)]);
    combat.fighters[0].action = Action::Windup {
        target: EntityId(20),
        start: 7,
        weapon: Weapon::Spear,
    };
    combat.fighters[0].arrows = 17;
    combat.fighters[1].action = Action::Wounded;
    combat.prepare(&[EntityId(20), EntityId(30), EntityId(10)]);
    assert_eq!(combat.fighters[0].action, Action::Wounded);
    assert_eq!(combat.fighters[1].arrows, 60);
    assert_eq!(combat.fighters[2].arrows, 17);
    assert_eq!(
        combat.fighters[2].action,
        Action::Windup {
            target: EntityId(20),
            start: 7,
            weapon: Weapon::Spear,
        }
    );
    let storage = combat.fighters.as_ptr();
    combat.prepare(&[EntityId(20), EntityId(30), EntityId(10)]);
    assert_eq!(
        combat.fighters.as_ptr(),
        storage,
        "unchanged layout reuses dense storage"
    );
    combat.prepare(&[EntityId(10)]);
    assert_eq!(combat.fighters[0].arrows, 17);
    assert!(
        combat.remap.is_empty(),
        "removed identity cannot retain combat custody"
    );
}

/// A column that leaves at a layout change (a horse extra that died) is gone
/// from the next image: a Windup aimed at it ends as its no-op strike at
/// impact leaves it and a block against it lapses. A Windup on an identity
/// that never was a column (a live-role holder) is untouched.
#[test]
fn a_departed_column_ends_windups_and_blocks_aimed_at_it() {
    let (horse, holder) = (EntityId(30), EntityId(99));
    let mut combat = CombatRuntime::default();
    combat.prepare(&[EntityId(10), EntityId(20), horse]);
    let windup = |target| Action::Windup {
        target,
        start: 7,
        weapon: Weapon::Spear,
    };
    combat.fighters[0].action = windup(horse);
    combat.fighters[0].block = Some((horse, 12));
    combat.fighters[1].action = windup(holder);
    combat.fighters[1].block = Some((EntityId(10), 12));
    combat.prepare(&[EntityId(10), EntityId(20)]);
    assert_eq!(
        combat.fighters[0].action,
        Action::Recover {
            until: 7 + u64::from(Weapon::Spear.cycle()),
        }
    );
    assert_eq!(combat.fighters[0].block, None);
    assert_eq!(combat.fighters[1].action, windup(holder));
    assert_eq!(combat.fighters[1].block, Some((EntityId(10), 12)));
}

#[test]
fn front_rank_outside_a_pulse_still_strikes_an_enemy_in_reach() {
    use super::super::formation::{Order, RegimentSpec};
    let (state, mut battle, actor, victim) = pair();
    let origin = state.entities.pos(slot(&state, actor));
    battle
        .formation
        .add_regiment(
            RegimentSpec {
                id: 0,
                faction: 1,
                origin,
                facing: [1.0, 0.0],
                files: 1,
                frontage: 1.0,
                order: Order::Hold,
            },
            &[actor],
        )
        .unwrap();
    prepare(&state, &mut battle, 0);
    let a = index(&state, &battle, actor);
    assert!(
        !battle.formation.can_attack_at(a),
        "no regiment in contact, so no pulse"
    );
    let target = super::super::EnemyTarget {
        id: victim,
        pos: state.entities.pos(slot(&state, victim)),
        distance: 2.0,
    };
    let swings = (0..1000)
        .filter(|&tick| {
            battle.columns.targets[a] = Some(target);
            let out = step_at(&state, &mut battle, tick);
            out.events.iter().any(|event| {
                matches!(event, EntityEvent::Swing { entity, hand: Hand::Main, .. } if *entity == actor)
            })
        })
        .count();
    // Fifty cautious decisions: about one in six swings, never an aggressive surge.
    assert!((1..=20).contains(&swings), "swings={swings}");
}

#[test]
fn a_broken_fugitive_at_bay_still_strikes_an_enemy_in_reach() {
    use super::super::formation::{Order, RegimentSpec};
    let (state, mut battle, actor, victim) = pair();
    let origin = state.entities.pos(slot(&state, actor));
    battle
        .formation
        .add_regiment(
            RegimentSpec {
                id: 0,
                faction: 1,
                origin,
                facing: [1.0, 0.0],
                files: 1,
                frontage: 1.0,
                order: Order::Hold,
            },
            &[actor],
        )
        .unwrap();
    // The bound sits one block behind the fugitive: he can run no further.
    battle
        .formation
        .set_bounds(
            [origin[0] - 1.0, origin[2] - 50.0],
            [origin[0] + 50.0, origin[2] + 50.0],
        )
        .unwrap();
    prepare(&state, &mut battle, 0);
    let a = index(&state, &battle, actor);
    battle.formation.members[a].routing = true;
    battle.formation.members[a].wavering = true;
    let target = super::super::EnemyTarget {
        id: victim,
        pos: state.entities.pos(slot(&state, victim)),
        distance: 2.0,
    };
    let swings = (0..1000)
        .filter(|&tick| {
            battle.columns.targets[a] = Some(target);
            let out = step_at(&state, &mut battle, tick);
            out.events.iter().any(|event| {
                matches!(event, EntityEvent::Swing { entity, hand: Hand::Main, .. } if *entity == actor)
            })
        })
        .count();
    assert!(swings > 0, "a cornered fugitive never turned at bay");
}

/// One combat pass with the actor's column forced out of the active set, which
/// `prepare` cannot produce on demand.
fn step_off_chunk(
    state: &TickState,
    battle: &mut BattleState,
    tick: u64,
    bi: usize,
    off: bool,
) -> Output {
    prepare(state, battle, tick);
    if off {
        battle.columns.active[bi] = false;
    }
    let frame = Frame {
        entities: &state.entities,
        registry: &state.registry,
        world: &state.world,
        tick,
    };
    let mut out = Output::default();
    step(&frame, battle, &mut out);
    out
}

/// A flight stamps its hold once. The two paths that can read it must see the
/// deadline the old per-tick re-stamp left, or a man would be held longer than
/// base holds him:
/// - a pass that cannot visit him (chunk out of the active set) freezes the
///   marker at `tick + 19`, base's value for the tick before, once;
/// - a pass that can read it (pinned at bay) resolves the marker to the same
///   value, so the hold does not slide forward.
#[test]
fn a_flight_hold_is_stamped_once_and_never_slides_forward() {
    use super::super::formation::{Order, RegimentSpec};
    let enter = |actor: EntityId, state: &TickState, battle: &mut BattleState| {
        battle
            .formation
            .add_regiment(
                RegimentSpec {
                    id: 0,
                    faction: 1,
                    origin: state.entities.pos(slot(state, actor)),
                    facing: [1.0, 0.0],
                    files: 1,
                    frontage: 1.0,
                    order: Order::Hold,
                },
                &[actor],
            )
            .unwrap();
        prepare(state, battle, 0);
        let bi = index(state, battle, actor);
        battle.formation.members[bi].routing = true;
        battle.columns.targets[bi] = None;
        bi
    };

    // Off the active set: base wrote 100 + 20 on tick 100 and nothing on tick
    // 101, so the frozen value is exactly 120, however long the gap.
    let (state, mut battle, actor, _victim) = pair();
    let bi = enter(actor, &state, &mut battle);
    step_off_chunk(&state, &mut battle, 100, bi, false);
    let Action::Guard { until } = battle.combat.fighters[bi].action else {
        panic!("a fugitive holds a guard");
    };
    assert!(
        until > 120,
        "a flight must not hold a deadline that expires on the next pass: {until}"
    );
    step_off_chunk(&state, &mut battle, 101, bi, true);
    assert_eq!(
        battle.combat.fighters[bi].action,
        Action::Guard { until: 120 }
    );
    for tick in 102..=140 {
        step_off_chunk(&state, &mut battle, tick, bi, true);
    }
    assert_eq!(
        battle.combat.fighters[bi].action,
        Action::Guard { until: 120 },
        "an off-chunk fighter is not re-stamped every tick"
    );
    // He rallies and his chunk comes back. Base's guard expired at 120, so the
    // return must not mint a fresh hold. The pass is pinned to one with no
    // attack decision, so the guard is the only thing it could rewrite.
    battle.formation.members[bi].routing = false;
    let mut rally = 141;
    while (rally + actor.0 as u64).is_multiple_of(ATTACK_DECISION_TICKS) {
        rally += 1;
    }
    step_off_chunk(&state, &mut battle, rally, bi, false);
    match battle.combat.fighters[bi].action {
        Action::Guard { until } => assert_eq!(
            until, 120,
            "a rally after an off-chunk flight mints no fresh hold"
        ),
        other => panic!("a rally pass with no decision keeps the guard: {other:?}"),
    }

    // Pinned at bay on the very next pass: the marker resolves to 120, not to
    // 101 + 19 + 19, and the pass stops on the hold before any decision.
    let (state, mut battle, actor, _victim) = pair();
    let bi = enter(actor, &state, &mut battle);
    step_off_chunk(&state, &mut battle, 100, bi, false);
    let at = state.entities.pos(slot(&state, actor));
    battle
        .formation
        .set_bounds([at[0] - 1.0, at[2] - 50.0], [at[0] + 50.0, at[2] + 50.0])
        .unwrap();
    assert!(
        battle.formation.cornered_at(bi),
        "the bounds pin the fugitive"
    );
    step_off_chunk(&state, &mut battle, 101, bi, false);
    assert_eq!(
        battle.combat.fighters[bi].action,
        Action::Guard { until: 120 },
        "the hold resolves to base's deadline and does not slide forward"
    );
}
