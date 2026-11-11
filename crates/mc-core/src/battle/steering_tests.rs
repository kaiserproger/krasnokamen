use super::*;
use crate::battle::formation::{Order, RegimentSpec, SoldierSample};

fn single(runtime: &mut FormationRuntime, id: i32, faction: u16, x: f64, order: Order) {
    runtime
        .add_regiment(
            RegimentSpec {
                id: id as u32,
                faction,
                origin: [x, 64.0, 0.0],
                facing: [if faction == 1 { 1.0 } else { -1.0 }, 0.0],
                files: 1,
                order,
            },
            &[EntityId(id)],
        )
        .unwrap();
}

fn integrate(runtime: &mut FormationRuntime, tick: u64) {
    let samples: FxHashMap<_, _> = runtime
        .members
        .iter()
        .map(|m| {
            (
                m.id,
                SoldierSample {
                    pos: m.pos,
                    health: 20.0,
                    faction: runtime.regiments[m.regiment].spec.faction,
                },
            )
        })
        .collect();
    runtime.prepare(tick, |id| samples.get(&id).copied());
    runtime.neighbours.rebuild(&runtime.members);
    let drive: Vec<_> = (0..runtime.members.len())
        .map(|i| movement(runtime, i))
        .collect();
    for (m, v) in runtime.members.iter_mut().zip(drive) {
        m.pos[0] += v[0] * mc_physics::TICK_SECONDS;
        m.pos[2] += v[2] * mc_physics::TICK_SECONDS;
    }
}

#[test]
fn frozen_simultaneous_rows_cannot_close_below_minimum_spacing() {
    let mut runtime = FormationRuntime::default();
    single(&mut runtime, 10, 1, 0.0, Order::Pursue);
    single(&mut runtime, 20, 2, 1.0, Order::Pursue);
    runtime.regiments[0].enemy_routing = true;
    runtime.regiments[1].enemy_routing = true;
    for tick in 0..50 {
        // Force a mutual pursuit conflict; no target/decision scans can mask
        // the steering guard by deciding not to move in the first place.
        runtime.tick = tick;
        runtime.regiments[0].enemy = Some(runtime.members[1].pos);
        runtime.regiments[1].enemy = Some(runtime.members[0].pos);
        runtime.neighbours.rebuild(&runtime.members);
        let a = movement(&mut runtime, 0);
        let b = movement(&mut runtime, 1);
        runtime.members[0].pos[0] += a[0] * mc_physics::TICK_SECONDS;
        runtime.members[1].pos[0] += b[0] * mc_physics::TICK_SECONDS;
        let distance = (runtime.members[1].pos[0] - runtime.members[0].pos[0]).abs();
        assert!(
            distance >= MIN_SPACING - 1e-9,
            "tick={tick}, distance={distance}"
        );
    }
    assert!(runtime.work.separation_candidates > 0);
}

#[test]
fn exact_overlap_has_deterministic_opposite_separation_not_a_permanent_stack() {
    let mut runtime = FormationRuntime::default();
    single(&mut runtime, 10, 1, 0.0, Order::Hold);
    single(&mut runtime, 20, 2, 0.0, Order::Hold);
    runtime.neighbours.rebuild(&runtime.members);
    let a = movement(&mut runtime, 0);
    let b = movement(&mut runtime, 1);
    assert!(a[0] < 0.0 && b[0] > 0.0);
    assert!((a[0] + b[0]).abs() < 1e-9);
    assert_eq!(runtime.work.push_calls, 2);
}

#[test]
fn routers_flee_and_pursuers_follow_the_moving_rout_not_its_old_anchor() {
    let mut runtime = FormationRuntime::default();
    single(&mut runtime, 10, 1, 0.0, Order::Pursue);
    single(&mut runtime, 20, 2, 10.0, Order::Hold);
    runtime.regiments[1].morale.broken = true;
    runtime.members[1].routing = true;
    let mut previous = runtime.members[1].pos[0];
    for tick in 0..100 {
        integrate(&mut runtime, tick);
        assert!(
            runtime.members[1].pos[0] > previous,
            "router must keep running"
        );
        previous = runtime.members[1].pos[0];
    }
    assert!(runtime.members[1].pos[0] > 30.0);
    assert!(runtime.members[0].pos[0] > 20.0);
    assert!(
        runtime.regiments[0].enemy.unwrap()[0] > 25.0,
        "pursuit field must move with routers"
    );
    assert!(
        runtime.can_attack(EntityId(10)),
        "pursuit must permit actual attacks outside pulse gating"
    );
    assert!(!runtime.can_attack(EntityId(20)), "routers cannot attack");
}

#[test]
fn dense_cells_bound_inspection_and_impulses_to_five_neighbours() {
    let mut runtime = FormationRuntime::default();
    runtime
        .add_regiment(
            RegimentSpec {
                id: 0,
                faction: 1,
                origin: [0.0, 64.0, 0.0],
                facing: [1.0, 0.0],
                files: 20,
                order: Order::Hold,
            },
            &(1..=1000).map(EntityId).collect::<Vec<_>>(),
        )
        .unwrap();
    for m in &mut runtime.members {
        m.pos = [0.1, 64.0, 0.1];
    }
    runtime.neighbours.rebuild(&runtime.members);
    let _ = movement(&mut runtime, 0);
    assert!(runtime.work.separation_candidates <= 25 * CELL_CAP as u64);
    assert_eq!(runtime.work.push_calls, NEIGHBOURS as u64);
    assert_eq!(runtime.neighbours.overflow, 992);
}

#[test]
fn nearest_enemy_front_projects_to_lateral_file_not_enemy_centroid() {
    let mut runtime = FormationRuntime::default();
    runtime
        .add_regiment(
            RegimentSpec {
                id: 0,
                faction: 1,
                origin: [20.0, 64.0, 0.0],
                facing: [0.0, 1.0],
                files: 10,
                order: Order::Hold,
            },
            &(1..=100).map(EntityId).collect::<Vec<_>>(),
        )
        .unwrap();
    runtime
        .add_regiment(
            RegimentSpec {
                id: 1,
                faction: 2,
                origin: [0.0, 64.0, 10.0],
                facing: [0.0, -1.0],
                files: 50,
                order: Order::Hold,
            },
            &(101..=200).map(EntityId).collect::<Vec<_>>(),
        )
        .unwrap();
    let (point, _, work) = nearest_front(&runtime.regiments, 0);
    assert_eq!(point, Some([20.0, 64.0, 10.0]));
    assert_eq!(work, 1);
}

#[test]
fn routers_halt_inside_loaded_ticket_instead_of_entering_frozen_chunks() {
    let mut runtime = FormationRuntime::default();
    single(&mut runtime, 10, 1, 0.0, Order::Pursue);
    single(&mut runtime, 20, 2, 10.0, Order::Hold);
    runtime.set_bounds([-5.0, -5.0], [15.0, 5.0]).unwrap();
    runtime.regiments[1].morale.broken = true;
    runtime.members[1].routing = true;
    for tick in 0..100 {
        integrate(&mut runtime, tick);
        assert!(
            runtime
                .members
                .iter()
                .all(|m| m.pos[0] <= 15.0 && m.pos[0] >= -5.0)
        );
    }
    assert!((runtime.members[1].pos[0] - 15.0).abs() < 1e-8);
}
