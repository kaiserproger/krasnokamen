use super::*;
use crate::battle::formation::{FormationRuntime, Order, RegimentSpec, SoldierSample};
use crate::boundary::EntityId;

#[test]
fn steady_regiment_breaks_near_ten_percent_not_fifty() {
    let mut morale = Morale::default();
    let mut broke_at = None;
    for losses in 0..=15 {
        morale.update(losses as u64 * 10, 1000, losses * 10, 100.0, false);
        if morale.broken {
            broke_at = Some(losses);
            break;
        }
    }
    assert!(
        (8..=12).contains(&broke_at.expect("regiment must break")),
        "break={broke_at:?}"
    );
}

#[test]
fn rear_rank_flight_precedes_front_and_then_whole_regiment_breaks() {
    let mut runtime = FormationRuntime::default();
    runtime
        .add_regiment(
            RegimentSpec {
                frontage: 1.0,
                id: 0,
                faction: 1,
                origin: [0.0, 64.0, 0.0],
                facing: [0.0, 1.0],
                files: 10,
                order: Order::Hold,
            },
            &(1..=100).map(EntityId).collect::<Vec<_>>(),
        )
        .unwrap();
    for i in 0..8 {
        runtime.mark_wounded(EntityId(i + 1));
    }
    runtime.prepare(0, |id| {
        Some(SoldierSample {
            slot: None,
            pos: [f64::from(id.0), 64.0, 0.0],
            health: 20.0,
            faction: 1,
        })
    });
    runtime.update_morale(0);
    let front = runtime
        .members
        .iter()
        .filter(|m| m.original_rank < 5 && !m.wounded && m.routing)
        .count();
    let rear = runtime
        .members
        .iter()
        .filter(|m| m.original_rank >= 5 && m.routing)
        .count();
    assert!(rear > front, "front={front}, rear={rear}");
    assert!(!runtime.regiments[0].morale.broken);
    for i in 8..10 {
        runtime.mark_wounded(EntityId(i + 1));
    }
    runtime.prepare(10, |id| {
        Some(SoldierSample {
            slot: None,
            pos: [f64::from(id.0), 64.0, 0.0],
            health: 20.0,
            faction: 1,
        })
    });
    runtime.update_morale(10);
    assert!(runtime.regiments[0].morale.broken);
    assert!(runtime.members.iter().all(|m| m.routing));
}

#[test]
fn pressure_levels_do_not_accumulate_and_shields_reduce_missile_effect() {
    let mut plain = Morale::default();
    let mut shield = Morale::default();
    for tick in (0..100).step_by(10) {
        plain.pressure.missiles = true;
        shield.pressure = Pressure {
            missiles: true,
            shield_wall: true,
            ..Pressure::default()
        };
        plain.update(tick, 100, 0, 100.0, false);
        shield.update(tick, 100, 0, 100.0, false);
        assert_eq!(plain.value, 67.0);
        assert_eq!(shield.value, 71.0);
    }
    plain.pressure = Pressure {
        flank: true,
        rear: true,
        ..Pressure::default()
    };
    plain.update(100, 100, 0, 100.0, false);
    assert_eq!(plain.value, 55.0);
    plain.update(110, 100, 0, 100.0, false);
    assert_eq!(plain.value, 75.0);
}

#[test]
fn kills_raise_morale_and_panic_and_recent_casualties_reduce_it() {
    let mut morale = Morale::default();
    morale.kills = 1;
    morale.update(0, 100, 0, 100.0, false);
    assert_eq!(morale.value, 78.0);
    morale.pressure.panicking = 100;
    morale.update(10, 100, 0, 100.0, false);
    assert_eq!(morale.value, 68.0);
    morale.pressure.adjacent_rout = true;
    morale.update(20, 100, 0, 100.0, false);
    assert_eq!(morale.value, 58.0);
    morale.update(30, 100, 3, 100.0, false);
    assert_eq!(morale.value, 35.0);
    morale.update(240, 100, 3, 100.0, false);
    assert_eq!(
        morale.value, 40.0,
        "rate penalty must expire after 200 ticks"
    );
}

#[test]
fn missile_cause_survives_deferred_damage_and_is_not_counted_twice() {
    let mut melee = Morale::default();
    let mut missile = Morale::default();
    missile.missile_losses = 1;
    missile.update(0, 100, 1, 100.0, false);
    melee.update(0, 100, 1, 100.0, false);
    assert_eq!(missile.value - melee.value, 2.0);
    missile.update(10, 100, 2, 100.0, false);
    melee.update(10, 100, 2, 100.0, false);
    assert_eq!(
        missile.value - melee.value,
        2.0,
        "second melee loss must not become a missile loss"
    );
}

#[test]
fn symmetric_kill_rewards_do_not_prevent_historical_loss_break_with_panic_cascade() {
    let mut runtime = FormationRuntime::default();
    runtime
        .add_regiment(
            RegimentSpec {
                frontage: 1.0,
                id: 0,
                faction: 1,
                origin: [0.0, 64.0, 0.0],
                facing: [0.0, 1.0],
                files: 50,
                order: Order::Hold,
            },
            &(1..=1000).map(EntityId).collect::<Vec<_>>(),
        )
        .unwrap();
    let mut break_percent = None;
    for percent in 1..=15 {
        for id in (percent - 1) * 10 + 1..=percent * 10 {
            runtime.mark_wounded(EntityId(id));
        }
        runtime.regiments[0].morale.kills = 10;
        let tick = percent as u64 * 10;
        runtime.prepare(tick, |id| {
            Some(SoldierSample {
                slot: None,
                pos: [f64::from(id.0 % 50), 64.0, f64::from(id.0 / 50)],
                health: 20.0,
                faction: 1,
            })
        });
        runtime.update_morale(tick);
        if runtime.regiments[0].morale.broken {
            break_percent = Some(percent);
            break;
        }
    }
    assert!((8..=15).contains(&break_percent.expect("panic must break symmetric combat")));
}
