use super::*;
use crate::mounts::tests::{fixture, slot, spawn};

#[test]
fn sustained_volley_is_five_to_six_per_minute_and_burst_expires() {
    let mut volley = Volley::new(0);
    let mut shots = Vec::new();
    for tick in 0..1200 {
        volley.update(tick);
        if volley.loose {
            shots.push(tick);
        }
    }
    assert_eq!(shots, [30, 250, 470, 690, 910, 1130]);
    let mut volley = Volley::new(0);
    volley.burst_until = MAX_BURST_TICKS;
    let mut shots = Vec::new();
    for tick in 0..1800 {
        volley.update(tick);
        if volley.loose {
            shots.push(tick);
        }
    }
    assert!(
        shots
            .windows(2)
            .filter(|s| s[0] < 1200)
            .all(|s| s[1] - s[0] == 110)
    );
    assert!(
        shots
            .windows(2)
            .filter(|s| s[0] >= 1200)
            .all(|s| s[1] - s[0] == 220)
    );
}

#[test]
fn aimed_arc_matches_native_drag_at_long_and_short_ranges() {
    for (distance, height) in [(200.0, 0.0), (60.0, 3.0), (20.0, -2.0)] {
        let (mut velocity, iterations) = aimed_arc([0.0; 3], [distance, height, 0.0]).unwrap();
        assert_eq!(iterations, 8);
        let speed = velocity.iter().map(|v| v * v).sum::<f64>().sqrt();
        assert!(speed <= 4.5);
        if distance == 200.0 {
            let angle = velocity[1].atan2(velocity[0]).to_degrees();
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
            nearest = nearest.min((position[0] - distance).hypot(position[1] - height));
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
        battle.targets.insert(
            id,
            super::super::EnemyTarget {
                id: target,
                pos: state.entities.pos(target_slot),
                distance: 40.0,
            },
        );
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
        if tick == 0 {
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
            assert_eq!((tick - 30) % 220, 0);
        }
        assert!(battle.combat.work.arc_iterations <= 16);
        total += out.arrows.len();
    }
    assert_eq!(total, 120);
    assert_eq!(battle.combat.fighters[&a].arrows, 0);
    assert_eq!(battle.combat.fighters[&b].arrows, 0);
}
