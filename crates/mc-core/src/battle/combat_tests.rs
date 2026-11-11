use super::*;
use crate::mounts::tests::{fixture, slot, spawn};
use crate::tick::TickState;

fn pair() -> (TickState, BattleState, EntityId, EntityId) {
    let (mut state, _, pos) = fixture();
    let actor = spawn(&mut state, "husk", pos);
    let victim = spawn(&mut state, "husk", [pos[0] + 2.0, pos[1], pos[2]]);
    let a = slot(&state, actor);
    let b = slot(&state, victim);
    state.entities.set_faction(a, 1);
    state.entities.set_faction(b, 2);
    let mut battle = BattleState::default();
    battle.targets.insert(
        actor,
        super::super::EnemyTarget {
            id: victim,
            pos: state.entities.pos(b),
            distance: 2.0,
        },
    );
    (state, battle, actor, victim)
}

fn step_at(state: &TickState, battle: &mut BattleState, tick: u64) -> Output {
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
                crate::boundary::SwingKind::Stab
            } else {
                crate::boundary::SwingKind::Whack
            },
            duration_ticks: weapon.cycle()
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
            battle.combat.fighters[&actor].action,
            Action::Recover { .. }
        ));
        assert!(battle.targets.contains_key(&actor));
        assert_eq!(battle.targets[&actor].id, victim);
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
        battle.combat.fighters.insert(
            actor,
            Fighter {
                action: Action::Windup {
                    target: victim,
                    start,
                    weapon: Weapon::Sword,
                },
                ..Default::default()
            },
        );
        battle.combat.fighters.insert(
            victim,
            Fighter {
                action: Action::Recover {
                    until: start + ready,
                },
                ..Default::default()
            },
        );
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
                animation: crate::boundary::SwingKind::None,
                duration_ticks: 18
            }));
            assert_eq!(
                battle.combat.fighters[&actor].action,
                Action::Stagger { until: start + 28 }
            );
        }
    }
}

#[test]
fn calibration_has_thirteen_per_thousand_removals_nine_wounded_per_death() {
    let mut counts = [0; 3];
    for sample in 0..10_000 {
        counts[match outcome(sample, false, 0.0) {
            Outcome::Glance => 0,
            Outcome::Wound => 1,
            Outcome::Kill => 2,
        }] += 1;
    }
    assert_eq!(counts, [9870, 117, 13]);
    assert_eq!(
        (0..10_000)
            .filter(|r| outcome(*r, true, 0.0) == Outcome::Kill)
            .count(),
        900
    );
}

#[test]
fn stale_target_faction_change_cancels_damage_at_impact() {
    let (mut state, mut battle, actor, victim) = pair();
    battle.combat.fighters.insert(
        actor,
        Fighter {
            action: Action::Windup {
                target: victim,
                start: 0,
                weapon: Weapon::Sword,
            },
            ..Default::default()
        },
    );
    let victim_slot = slot(&state, victim);
    state.entities.set_faction(victim_slot, 1);
    let out = step_at(&state, &mut battle, 10);
    assert!(out.intents.is_empty());
    assert_eq!(battle.combat.work.strikes, 0);
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
