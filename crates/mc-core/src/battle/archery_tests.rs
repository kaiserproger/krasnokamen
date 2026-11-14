use super::super::combat::tests::{fixture, index, prepare};
use super::*;
use crate::mounts::tests::{slot, spawn};

#[test]
fn sustained_volley_is_five_to_six_per_minute() {
    let mut volley = Volley::new(0, (2, 0));
    let mut shots = Vec::new();
    for tick in 0..1200 {
        volley.update(tick);
        if volley.loose {
            shots.push(tick);
        }
    }
    assert_eq!(shots, [30, 250, 470, 690, 910, 1130]);
}

#[test]
fn twenty_regiments_stagger_release_without_splitting_regiment_or_cadence() {
    let mut volleys: Vec<_> = (1..=2)
        .flat_map(|faction| (0..10).map(move |regiment| Volley::new(0, (faction, regiment))))
        .collect();
    let mut releases = vec![Vec::new(); volleys.len()];
    let mut draws = vec![None; volleys.len()];
    let mut updates = 0;
    for tick in 0..1200 {
        let mut simultaneous = 0;
        for (i, volley) in volleys.iter_mut().enumerate() {
            volley.update(tick);
            updates += 1;
            if volley.draw {
                draws[i] = Some(tick);
            }
            if volley.loose {
                assert_eq!(draws[i], Some(tick - DRAW_TICKS));
                releases[i].push(tick);
                simultaneous += 1;
            }
        }
        assert!(simultaneous <= 1, "synchronized regiments at tick {tick}");
    }
    assert_eq!(
        updates,
        20 * 1200,
        "one clock update per regiment, not per soldier"
    );
    for shots in releases {
        assert!((5..=6).contains(&shots.len()));
        assert!(
            shots
                .windows(2)
                .all(|pair| pair[1] - pair[0] == SUSTAINED_TICKS)
        );
    }
}

#[test]
fn aimed_arc_matches_native_drag_at_long_and_short_ranges() {
    for (distance, height) in [(200.0, 0.0), (60.0, 3.0), (20.0, -2.0)] {
        let (mut velocity, iterations) = aimed_arc([0.0; 3], [distance, height, 0.0]).unwrap();
        assert_eq!(iterations, 8);
        let speed = velocity.iter().map(|v| v * v).sum::<f64>().sqrt();
        assert!(speed <= 4.5);
        if distance == 200.0 {
            let angle = mth::atan2(velocity[1], velocity[0]).to_degrees();
            assert!((23.0..25.0).contains(&angle), "{angle}");
            assert!(speed > 4.4);
        }
        let mut position = [0.0; 3];
        let mut nearest = f64::MAX;
        for _ in 0..90 {
            for axis in 0..3 {
                position[axis] += velocity[axis];
                velocity[axis] *= 0.99;
            }
            velocity[1] -= 0.05;
            nearest = nearest.min(mth::length2(position[0] - distance, position[1] - height));
        }
        assert!(nearest < 1e-8, "{distance} blocks missed by {nearest}");
    }
    assert!(aimed_arc([0.0; 3], [400.0, 0.0, 0.0]).is_none());
}

#[test]
fn actual_archers_share_volley_tick_and_exhaust_sixty_arrows_once() {
    let (mut state, _, pos) = fixture();
    let a = spawn(&mut state, "husk", pos);
    let b = spawn(&mut state, "husk", [pos[0], pos[1], pos[2] + 1.0]);
    let target = spawn(&mut state, "husk", [pos[0] + 40.0, pos[1], pos[2]]);
    let bow = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:bow").unwrap())
        .unwrap();
    let target_slot = slot(&state, target);
    state.entities.set_faction(target_slot, 2);
    let mut battle = BattleState::default();
    for id in [a, b] {
        let s = slot(&state, id);
        state.entities.set_faction(s, 1);
        state.entities.row_mut(s).attack_reach = 220.0;
        std::sync::Arc::make_mut(&mut state.entities.row_mut(s).appearance).equipment[0] =
            Some(mc_data::ItemStack::new(bow, 1));
    }
    prepare(&state, &mut battle, 0);
    for id in [a, b] {
        let bi = index(&state, &battle, id);
        battle.columns.targets[bi] = Some(super::super::EnemyTarget {
            id: target,
            pos: state.entities.pos(target_slot),
            distance: 40.0,
        });
    }
    let mut total = 0;
    for tick in 0..13_500 {
        let frame = Frame {
            entities: &state.entities,
            registry: &state.registry,
            world: &state.world,
            tick,
        };
        let mut out = Output::default();
        super::super::combat::step(&frame, &mut battle, &mut out);
        if tick == phase((1, 0)) {
            assert_eq!(out.events.len(), 2);
            assert!(out.events.iter().all(|event| matches!(
                event,
                crate::boundary::EntityEvent::Swing {
                    duration_ticks: 40,
                    ..
                }
            )));
        }
        if !out.arrows.is_empty() {
            assert_eq!(out.arrows.len(), 2);
            for arrow in &out.arrows {
                let speed = arrow.velocity.iter().map(|v| v * v).sum::<f64>().sqrt();
                // Flight uses ceil(speed * base_damage), not base_damage HP;
                // only the rare armour-gap arrow carries the lethal base.
                let hit = (speed * f64::from(arrow.damage)).ceil();
                assert!(
                    hit == 1.0 || arrow.damage == LETHAL_ARROW_BASE_DAMAGE,
                    "{hit}"
                );
            }
            assert_eq!((tick - DRAW_TICKS - phase((1, 0))) % SUSTAINED_TICKS, 0);
        }
        assert!(battle.combat.work.arc_iterations <= 16);
        total += out.arrows.len();
    }
    assert_eq!(total, 120);
    assert_eq!(battle.combat.fighters[index(&state, &battle, a)].arrows, 0);
    assert_eq!(battle.combat.fighters[index(&state, &battle, b)].arrows, 0);
}

#[test]
fn resumed_regiments_keep_absolute_volley_phase_and_full_draw_time() {
    for now in [0, 17, 219, 220, 743] {
        for regiment in 0..20 {
            let volley = Volley::new(now, (1, regiment));
            assert!(volley.next >= now + DRAW_TICKS);
            assert_eq!(
                (volley.next - DRAW_TICKS) % SUSTAINED_TICKS,
                phase((1, regiment))
            );
        }
    }
}

#[test]
fn twenty_registered_archer_regiments_do_not_launch_in_one_kernel_tick() {
    use super::super::formation::{Order, RegimentSpec};
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
    for regiment in 0..20 {
        let origin = [pos[0], pos[1], pos[2] + f64::from(regiment)];
        let id = spawn(&mut state, "husk", origin);
        let slot = slot(&state, id);
        state.entities.set_faction(slot, 1);
        state.entities.row_mut(slot).attack_reach = 220.0;
        std::sync::Arc::make_mut(&mut state.entities.row_mut(slot).appearance).equipment[0] =
            Some(mc_data::ItemStack::new(bow, 1));
        battle
            .formation
            .add_regiment(
                RegimentSpec {
                    id: regiment,
                    faction: 1,
                    origin,
                    facing: [1.0, 0.0],
                    files: 1,
                    frontage: 1.0,
                    order: Order::Hold,
                },
                &[id],
            )
            .unwrap();
        archers.push(id);
    }
    prepare(&state, &mut battle, 0);
    for &id in &archers {
        let bi = index(&state, &battle, id);
        battle.columns.targets[bi] = Some(super::super::EnemyTarget {
            id: target,
            pos: state.entities.pos(target_slot),
            distance: 40.0,
        });
    }
    let mut released = 0;
    for tick in 0..465 {
        let frame = Frame {
            entities: &state.entities,
            registry: &state.registry,
            world: &state.world,
            tick,
        };
        let mut out = Output::default();
        super::super::combat::step(&frame, &mut battle, &mut out);
        assert!(out.arrows.len() <= 1, "synchronized launch at tick {tick}");
        assert!(battle.combat.work.arc_solves <= 1 && battle.combat.work.arc_iterations <= 8);
        assert_eq!(battle.combat.work.visited, 21);
        released += out.arrows.len();
    }
    assert_eq!(released, 40, "all twenty regiments must release twice");
    for id in archers {
        assert_eq!(
            battle.combat.fighters[index(&state, &battle, id)].arrows,
            58
        );
    }
}

#[test]
fn about_one_arrow_in_seventy_kills_through_iron_on_easy() {
    let lethal = (0..20_000_u64)
        .filter(|&tick| {
            arrow_damage(crate::boundary::EntityId(7), tick) == LETHAL_ARROW_BASE_DAMAGE
        })
        .count();
    assert!((200..=400).contains(&lethal), "lethal={lethal}");
    // Slowest aimed impact (90-tick flight) through 8 armour points on Easy.
    let speed = MAX_SPEED * 0.99_f64.powi(90);
    let raw = (speed * f64::from(LETHAL_ARROW_BASE_DAMAGE)).ceil() as f32;
    let easy = (raw / 2.0 + 1.0).min(raw);
    let landed = easy * (1.0 - (8.0_f32 / 5.0).max(8.0 - easy / 2.0) / 25.0);
    assert!(landed > 20.0, "landed={landed}");
    let common = (MAX_SPEED * f64::from(BATTLE_ARROW_BASE_DAMAGE)).ceil();
    assert!(common <= 1.0, "ordinary arrows stay one-HP hurts");
}
