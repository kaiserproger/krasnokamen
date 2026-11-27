use super::*;
use crate::battle::formation::{
    FormationRuntime, Order, PULSE_MORALE_FLOOR, RegimentSpec, SoldierSample,
};
use mc_domain::EntityId;

#[test]
fn film_nerve_keeps_a_rear_trickle_above_the_regiment_break_threshold() {
    let mut rear_top = 0.0_f32;
    for id in 1..=1000 {
        let front = nerve(id, 0, 20);
        let rear = nerve(id, 19, 20);
        assert!((3.0..=18.0).contains(&front), "front={front}");
        assert!((13.0..=28.0).contains(&rear), "rear={rear}");
        assert_eq!(rear - front, 10.0);
        rear_top = rear_top.max(rear);
    }
    // Sections stop pulsing below the floor, so losses stall there: a rear
    // top at or under it would leave a worn regiment unable to start flight.
    assert!(rear_top > PULSE_MORALE_FLOOR, "rear_top={rear_top}");
}

/// G2 fade: with a 25..40 rear nerve the regiment losing its exchange broke at
/// 11-18 % removed, its rear half (which does not fight) running first. A
/// regiment 13 % down in one exchange (morale 31) now keeps every man.
#[test]
fn a_regiment_thirteen_percent_down_keeps_its_rear_ranks() {
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
    for i in 0..13 {
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
    let morale = &runtime.regiments[0].morale;
    assert!(!morale.broken);
    assert!(
        (30.0..32.0).contains(&morale.value),
        "morale={}",
        morale.value
    );
    let routing = runtime
        .members
        .iter()
        .filter(|m| !m.wounded && m.routing)
        .count();
    assert_eq!(routing, 0);
}

#[test]
fn steady_regiment_breaks_near_twenty_percent_not_fifty() {
    let mut morale = Morale::default();
    let mut broke_at = None;
    for losses in 0..=30 {
        morale.update(losses as u64 * 10, 1000, losses * 10, 100.0, false);
        if morale.broken {
            broke_at = Some(losses);
            break;
        }
    }
    assert!(
        (16..=20).contains(&broke_at.expect("regiment must break")),
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
    for i in 0..16 {
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
    for i in 16..18 {
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

/// Riders who fled to the edge of the field rally there and ride back until
/// they fall, even when their regiment has broken: mounted regiments broke at
/// 25-40 % losses in the G1 rehearsals and their fled riders (groups of
/// 100-175) stood idle on the bounds from t 150 to the end. Riders short of
/// the edge and foot fugitives at the edge keep fleeing.
#[test]
fn riders_who_fled_to_the_edge_rally_and_ride_back() {
    let mut runtime = FormationRuntime::default();
    runtime.set_bounds([-50.0, -50.0], [50.0, 50.0]).unwrap();
    for (id, frontage, first) in [(0, 2.0, 1), (1, 1.0, 101)] {
        runtime
            .add_regiment(
                RegimentSpec {
                    frontage,
                    id,
                    faction: 1,
                    origin: [0.0, 64.0, 0.0],
                    facing: [0.0, 1.0],
                    files: 10,
                    order: Order::Hold,
                },
                &(first..first + 100).map(EntityId).collect::<Vec<_>>(),
            )
            .unwrap();
    }
    // Both regiments have broken: every soldier runs.
    for r in &mut runtime.regiments {
        r.morale.broken = true;
    }
    // Even ids have fled to the +x edge, odd ids are still on their way.
    let sample = |id: EntityId| {
        Some(SoldierSample {
            slot: None,
            pos: [if id.0 % 2 == 0 { 49.5 } else { 0.0 }, 64.0, 0.0],
            health: 20.0,
            faction: 1,
        })
    };
    // Regiment `id` decides on ticks with tick % 10 == id.
    for tick in [0, 1, 10, 11, 20, 21] {
        runtime.prepare(tick, sample);
        runtime.update_morale(tick);
    }
    for (i, m) in runtime.members.iter().enumerate() {
        let rallied = i < 100 && m.id.0 % 2 == 0;
        assert_eq!(!m.routing, rallied, "member {i} at x {}", m.pos[0]);
    }
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
    let mut morale = Morale {
        kills: 1,
        ..Morale::default()
    };
    morale.update(0, 100, 0, 100.0, false);
    assert_eq!(morale.value, 78.0);
    morale.pressure.panicking = 100;
    morale.update(10, 100, 0, 100.0, false);
    assert_eq!(morale.value, 68.0);
    morale.pressure.adjacent_rout = true;
    morale.update(20, 100, 0, 100.0, false);
    assert_eq!(morale.value, 58.0);
    morale.update(30, 100, 3, 100.0, false);
    assert_eq!(morale.value, 44.0);
    morale.update(240, 100, 3, 100.0, false);
    assert_eq!(
        morale.value, 49.0,
        "rate penalty must expire after 200 ticks"
    );
}

#[test]
fn missile_cause_survives_deferred_damage_and_is_not_counted_twice() {
    let mut melee = Morale::default();
    let mut missile = Morale {
        missile_losses: 1,
        ..Morale::default()
    };
    missile.update(0, 100, 1, 100.0, false);
    melee.update(0, 100, 1, 100.0, false);
    assert_eq!(missile.value - melee.value, 1.0);
    missile.update(10, 100, 2, 100.0, false);
    melee.update(10, 100, 2, 100.0, false);
    assert_eq!(
        missile.value - melee.value,
        1.0,
        "second melee loss must not become a missile loss"
    );
}

#[test]
fn symmetric_kill_rewards_still_break_a_disciplined_regiment() {
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
    for percent in 1..=40 {
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
    let break_percent = break_percent.expect("panic must break symmetric combat");
    assert!((20..=24).contains(&break_percent), "break={break_percent}");
}

#[test]
fn worn_regiment_in_contact_gives_way_while_a_resting_one_holds() {
    let worn = Morale {
        value: PULSE_MORALE_FLOOR + 5.0,
        cohesion: PULSE_MORALE_FLOOR + 5.0,
        ..Morale::default()
    };
    let mut engaged = worn.clone();
    let mut resting = worn;
    for decision in 0..40 {
        engaged.pressure.engaged = true;
        engaged.update(decision * 10, 100, 0, 100.0, false);
        resting.update(decision * 10, 100, 0, 100.0, false);
    }
    assert!(engaged.broken, "worn front froze at {}", engaged.value);
    assert!(
        !resting.broken,
        "rested regiment broke at {}",
        resting.value
    );
}
