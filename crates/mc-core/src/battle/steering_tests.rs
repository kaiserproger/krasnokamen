use super::*;
use crate::battle::formation::{Order, RegimentSpec, SoldierSample};
use crate::boundary::EntityId;
use rustc_hash::FxHashMap;

fn single(runtime: &mut FormationRuntime, id: i32, faction: u16, x: f64, order: Order) {
    runtime
        .add_regiment(
            RegimentSpec {
                frontage: 1.0,
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
                    slot: None,
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
                frontage: 1.0,
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
                frontage: 1.0,
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
                frontage: 1.0,
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
    let (regiment, point, _, work) = nearest_front(&runtime.regiments, 0);
    assert_eq!(regiment, Some(1));
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

#[test]
fn outside_knockback_recovers_inward_without_teleporting_or_exceeding_speed_budget() {
    let mut runtime = FormationRuntime::default();
    runtime.set_bounds([-5.0, -5.0], [15.0, 5.0]).unwrap();
    for start in [
        [16.0, 64.0, 0.0],
        [-6.0, 64.0, 0.0],
        [0.0, 64.0, 6.0],
        [0.0, 64.0, -6.0],
        [16.0, 64.0, 6.0],
    ] {
        for desired in [
            [5.4, 1.25, 0.0],
            [-5.4, 1.25, 0.0],
            [0.0, 1.25, 5.4],
            [0.0, 1.25, 0.0],
        ] {
            let velocity = runtime.constrain_velocity(start, desired);
            let back = [
                start[0].clamp(-5.0, 15.0) - start[0],
                start[2].clamp(-5.0, 5.0) - start[2],
            ];
            assert!(velocity[0] * back[0] + velocity[2] * back[1] > 0.0);
            let budget = mth::length2(desired[0], desired[2]).max(0.079 / mc_physics::TICK_SECONDS);
            assert!(mth::length2(velocity[0], velocity[2]) <= budget + 1e-9);
            assert_eq!(velocity[1], desired[1]);
        }
    }
    // A near-edge overshoot lands at the boundary, not through the other side.
    let velocity = runtime.constrain_velocity([15.01, 64.0, 0.0], [5.4, 0.0, 0.0]);
    assert!((15.01 + velocity[0] * mc_physics::TICK_SECONDS - 15.0).abs() < 1e-9);
}

#[test]
fn knocked_out_router_returns_to_loaded_interior_on_ordinary_motion_steps() {
    let mut runtime = FormationRuntime::default();
    single(&mut runtime, 10, 1, 0.0, Order::Pursue);
    single(&mut runtime, 20, 2, 10.0, Order::Hold);
    runtime.set_bounds([-5.0, -5.0], [15.0, 5.0]).unwrap();
    runtime.regiments[1].morale.broken = true;
    runtime.members[1].routing = true;
    runtime.members[1].pos[0] = 16.0; // external physics, still in the ticket's inset margin
    let mut previous = 16.0;
    for tick in 0..20 {
        integrate(&mut runtime, tick);
        let current = runtime.members[1].pos[0];
        assert!(current <= previous && previous - current <= 0.25 + 1e-9);
        previous = current;
    }
    assert!((runtime.members[1].pos[0] - 15.0).abs() < 1e-8);
}

// Frozen pre-CSR one-block hash-grid oracle. Test-only: proves a storage/layout
// change did not change candidate caps, ties, work accounting or movement bits.
fn legacy_grid(members: &[Member]) -> (FxHashMap<[i32; 2], Vec<usize>>, u64) {
    let mut cells = FxHashMap::<[i32; 2], Vec<usize>>::default();
    let mut overflow = 0;
    for (bi, m) in members.iter().enumerate().filter(|(_, m)| m.alive) {
        let bucket = cells.entry(cell(m.pos)).or_default();
        if bucket.len() < CELL_CAP {
            bucket.push(bi);
        } else {
            overflow += 1;
        }
    }
    (cells, overflow)
}

fn legacy_nearest(
    cells: &FxHashMap<[i32; 2], Vec<usize>>,
    bi: usize,
    members: &[Member],
) -> ([(f64, usize); NEIGHBOURS], u64) {
    let actor = &members[bi];
    let [x, z] = cell(actor.pos);
    let mut best = [(f64::INFINITY, usize::MAX); NEIGHBOURS];
    let mut examined = 0;
    for dz in -2..=2 {
        for dx in -2..=2 {
            let Some(bucket) = cells.get(&[x + dx, z + dz]) else {
                continue;
            };
            for &other in bucket {
                if other == bi {
                    continue;
                }
                examined += 1;
                let m = &members[other];
                // The vertical window is the steering rule, not storage layout.
                if (actor.pos[1] - m.pos[1]).abs() > super::LEVELS {
                    continue;
                }
                let d2 = (actor.pos[0] - m.pos[0]).powi(2) + (actor.pos[2] - m.pos[2]).powi(2);
                if d2 > SENSE * SENSE {
                    continue;
                }
                let Some(at) = best.iter().position(|&(distance, old)| {
                    d2 < distance
                        || (d2 == distance && (old == usize::MAX || m.id < members[old].id))
                }) else {
                    continue;
                };
                for k in (at + 1..NEIGHBOURS).rev() {
                    best[k] = best[k - 1];
                }
                best[at] = (d2, other);
            }
        }
    }
    (best, examined)
}

#[test]
fn csr_preserves_hash_grid_overflow_candidate_work_and_bitwise_drive() {
    let mut runtime = FormationRuntime::default();
    let spec = RegimentSpec {
        id: 0,
        faction: 1,
        origin: [0.0, 64.0, 0.0],
        facing: [0.0, 1.0],
        files: 20,
        frontage: 1.0,
        order: Order::Hold,
    };
    runtime
        .add_regiment(spec, &(1..=1000).rev().map(EntityId).collect::<Vec<_>>())
        .unwrap();
    for (bi, m) in runtime.members.iter_mut().enumerate() {
        m.pos = if bi < 100 {
            [0.1, 64.0, 0.1]
        } else {
            [
                ((bi * 37) % 127) as f64 / 8.0 - 8.0,
                64.0 + (bi % 3) as f64,
                ((bi * 53) % 131) as f64 / 8.0 - 8.0,
            ]
        };
        m.alive = bi % 17 != 0;
    }
    runtime.members[1].pos = [-1_000_000.0, 64.0, 1_000_000.0];
    runtime.members[2].pos = [1_000_000.0, 64.0, -1_000_000.0];
    for bi in 3..11 {
        runtime.members[bi].pos = [
            if bi % 2 == 0 { -2.0 } else { -2.000_000_1 },
            64.0,
            if bi % 3 == 0 { 0.0 } else { -0.000_000_1 },
        ];
    }
    let (legacy, overflow) = legacy_grid(&runtime.members);
    runtime.neighbours.rebuild(&runtime.members);
    assert_eq!(runtime.neighbours.overflow, overflow);
    assert_eq!(
        runtime.neighbours.offsets.len(),
        runtime.neighbours.keys.len() + 1
    );
    assert!(
        runtime
            .neighbours
            .offsets
            .windows(2)
            .all(|pair| pair[0] <= pair[1])
    );
    assert!(runtime.neighbours.keys.len() <= runtime.members.len());
    assert_eq!(
        runtime.neighbours.items.len() as u64,
        runtime.members.iter().filter(|m| m.alive).count() as u64 - overflow
    );
    for bi in 0..runtime.members.len() {
        if !runtime.members[bi].alive {
            continue;
        }
        let expected = legacy_nearest(&legacy, bi, &runtime.members);
        let actual = runtime.neighbours.nearest(bi, &runtime.members);
        assert_eq!(actual.1, expected.1, "candidate work at bi={bi}");
        assert_eq!(
            actual.0.map(|(d, i)| (d.to_bits(), i)),
            expected.0.map(|(d, i)| (d.to_bits(), i)),
            "neighbours at bi={bi}"
        );
        let before = runtime.work;
        let drive = movement(&mut runtime, bi);
        let work = runtime.work;
        runtime.work = before;
        let old_drive = movement_with_neighbours(&mut runtime, bi, expected.0, expected.1);
        assert_eq!(
            drive.map(f64::to_bits),
            old_drive.map(f64::to_bits),
            "drive at bi={bi}"
        );
        assert_eq!(runtime.work, work);
    }
    let capacity = |g: &SeparationGrid| {
        [
            g.keys.capacity(),
            g.offsets.capacity(),
            g.items.capacity(),
            g.member_cell.capacity(),
            g.fine_cells.capacity(),
            g.adjacent.capacity(),
            g.entries.capacity(),
        ]
    };
    let warm = capacity(&runtime.neighbours);
    for (bi, m) in runtime.members.iter_mut().enumerate() {
        m.alive = true;
        m.pos = [bi as f64 * 100.0, 64.0, 0.0];
    }
    runtime.neighbours.rebuild(&runtime.members);
    assert_eq!(
        capacity(&runtime.neighbours),
        warm,
        "dispersal must not reallocate CSR buffers"
    );
    assert_eq!(runtime.neighbours.keys.len(), runtime.members.len());
    for m in &mut runtime.members {
        m.alive = false;
    }
    runtime.neighbours.rebuild(&runtime.members);
    assert!(runtime.neighbours.items.is_empty());
    assert_eq!(runtime.neighbours.offsets, vec![0]);
    assert_eq!(runtime.neighbours.overflow, 0);
}

/// Net displacement and direction reversals of one body over a 2 s window, as
/// the client sees it: positions every second tick, steps under 0.01 ignored.
fn window_jitter(window: &[[f64; 2]]) -> (f64, usize) {
    let samples: Vec<_> = window.iter().step_by(2).collect();
    let (first, last) = (samples[0], samples[samples.len() - 1]);
    let net = (last[0] - first[0]).hypot(last[1] - first[1]);
    let steps: Vec<_> = samples
        .windows(2)
        .map(|pair| [pair[1][0] - pair[0][0], pair[1][1] - pair[0][1]])
        .filter(|step| step[0].hypot(step[1]) > 0.01)
        .collect();
    let reversals = steps
        .windows(2)
        .filter(|pair| pair[0][0] * pair[1][0] + pair[0][1] * pair[1][1] < 0.0)
        .count();
    (net, reversals)
}

/// `side` x `side` one-man regiments whose slots lie `slot_spacing` apart, the
/// bodies standing a good deal wider apart: slots closer than the body spacing,
/// as in the pressed ranks of a stalled contact line, so every body is pulled
/// into the ones around it.
fn pressed_block(side: usize, slot_spacing: f64) -> FormationRuntime {
    let mut runtime = FormationRuntime::default();
    for row in 0..side {
        for file in 0..side {
            let id = (row * side + file) as i32 + 1;
            runtime
                .add_regiment(
                    RegimentSpec {
                        frontage: 1.0,
                        id: id as u32,
                        faction: 1,
                        origin: [file as f64 * slot_spacing, 64.0, row as f64 * slot_spacing],
                        facing: [1.0, 0.0],
                        files: 1,
                        order: Order::Hold,
                    },
                    &[EntityId(id)],
                )
                .unwrap();
            let m = &mut runtime.members[(id - 1) as usize];
            // A fixed, uneven start: no two bodies start on a symmetric spot.
            m.pos[0] = file as f64 * 0.9 + (id * 7 % 11) as f64 * 0.01;
            m.pos[2] = row as f64 * 0.9 + (id * 5 % 13) as f64 * 0.01;
        }
    }
    runtime
}

/// G2L5 take, minutes 2-4: a quarter of the standing soldiers in contact reversed
/// their direction four or more times in 2 s, 0.04 blocks at a time. The
/// separation push was a step at the body spacing and the slot pull had no dead
/// zone, so a body pressed against its neighbours was shoved out and pulled
/// back every other tick.
#[test]
fn pressed_ranks_stand_still_instead_of_trading_pushes_and_pulls() {
    let mut runtime = pressed_block(6, 0.5);
    let mut tracks: Vec<Vec<[f64; 2]>> = vec![Vec::new(); runtime.members.len()];
    for tick in 0..300_u64 {
        integrate(&mut runtime, tick);
        if tick >= 260 {
            for (track, m) in tracks.iter_mut().zip(&runtime.members) {
                track.push([m.pos[0], m.pos[2]]);
            }
        }
    }
    let jittery: Vec<_> = tracks
        .iter()
        .enumerate()
        .filter_map(|(member, track)| {
            let (net, reversals) = window_jitter(track);
            (net < 0.3 && reversals >= 4).then_some((member, reversals))
        })
        .collect();
    assert!(
        jittery.is_empty(),
        "standing bodies that reversed 4+ times in 2 s: {jittery:?}"
    );
    // Easing the push must not let the block collapse on itself (the step push
    // left pairs 0.03 apart in the same block).
    let mut closest = f64::INFINITY;
    for (a, first) in runtime.members.iter().enumerate() {
        for second in &runtime.members[a + 1..] {
            closest = closest.min(mth::length2(
                first.pos[0] - second.pos[0],
                first.pos[2] - second.pos[2],
            ));
        }
    }
    assert!(closest >= 0.4, "closest pair {closest}");
}

/// The separation push grows from nothing at the body spacing: a body a hair
/// inside it is nudged, not shoved a tenth of a block.
#[test]
fn separation_push_ramps_in_from_the_body_spacing() {
    let push_at = |gap: f64| {
        let mut runtime = FormationRuntime::default();
        single(&mut runtime, 10, 1, 0.0, Order::Hold);
        single(&mut runtime, 20, 2, gap, Order::Hold);
        // The second body's slot is where it stands, the first one's is the
        // spot it stands on: only the push moves it.
        runtime.neighbours.rebuild(&runtime.members);
        movement(&mut runtime, 0)[0].abs() * mc_physics::TICK_SECONDS
    };
    assert!(
        push_at(MIN_SPACING - 0.005) < 0.005,
        "{}",
        push_at(MIN_SPACING - 0.005)
    );
    assert!(push_at(MIN_SPACING - 0.08) > 0.0);
    assert!(push_at(MIN_SPACING - 0.08) < push_at(MIN_SPACING - 0.15));
    assert!(push_at(0.4) > 0.02);
}

/// Inside the dead zone a crowded body does not chase its slot; outside it,
/// and when nothing presses on it, it does.
#[test]
fn crowded_body_ignores_a_slot_within_the_dead_zone() {
    let drive_for = |neighbour: bool, offset: f64| {
        let mut runtime = FormationRuntime::default();
        single(&mut runtime, 10, 1, 0.0, Order::Hold);
        if neighbour {
            // Beside the body, outside the body spacing: crowding, no push.
            single(&mut runtime, 20, 1, 0.0, Order::Hold);
            runtime.members[1].pos[2] = 0.7;
        }
        runtime.members[0].pos[0] = offset;
        runtime.neighbours.rebuild(&runtime.members);
        movement(&mut runtime, 0)[0]
    };
    // Slot at x = 0; the neighbour 0.7 to the side crowds the body.
    assert_eq!(drive_for(true, -0.1), 0.0, "inside the dead zone");
    assert!(drive_for(true, -0.3) > 0.0, "outside the dead zone");
    assert!(drive_for(false, -0.1) > 0.0, "alone it dresses to its slot");
}
