//! Synthetic natural relief: rolling one-block terraces, a hill, trees, a pond
//! and a wall on the flat fixture world (grass top at y=64).
use super::*;
use crate::battle::staging::{self, BattleCommand};
use crate::boundary::ChangeSet;
use crate::tick::{TickState, run_tick};

const ORIGIN: [f64; 3] = [8.5, 64.0, 8.5];

fn state_id(state: &TickState, name: &str) -> u32 {
    let id = mc_data::Identifier::parse(name).unwrap();
    state.registry.blocks.block(&id).unwrap().default.0
}

/// Rolling terraces: a 0..3 triangle wave of one-block steps every six
/// blocks, plus a four-block hill of three-block rings. The hill centre
/// (1 mod 6 on both axes) keeps ring edges off terrace edges, so neighbouring
/// columns differ by one block; two-block walls come only from `wall`.
fn height(x: i32, z: i32) -> i32 {
    let t = (x.div_euclid(6) + z.div_euclid(6)).rem_euclid(6);
    let hill = 4 - ((x - 31).abs().max((z + 59).abs()) / 3).min(4);
    64 + t.min(6 - t) + hill.max(0)
}

fn tree_at(x: i32, z: i32) -> bool {
    x.rem_euclid(9) == 4 && z.rem_euclid(11) == 5
}

/// Under the first side's second infantry rank (z = -29.5).
fn pond(x: i32, z: i32) -> bool {
    (0..=2).contains(&x) && (-31..=-29).contains(&z)
}

/// Wall of three blocks across the first side's path (it faces +z). Its row
/// and the rows beside it hold no trunk: no wall-and-tree dead-end pocket.
fn wall(x: i32, z: i32) -> bool {
    (-6..=2).contains(&x) && z == -20
}

/// Cave openings: a 3x3 one six deep in the second side's infantry path and a
/// 2x2 one eight deep under its front rank (z 44.5, files at x -31 and -30).
fn hole(x: i32, z: i32) -> Option<i32> {
    if (-20..=-18).contains(&x) && (20..=22).contains(&z) {
        Some(6)
    } else if (-31..=-30).contains(&x) && (44..=45).contains(&z) {
        Some(8)
    } else {
        None
    }
}

fn relief_world(state: &mut TickState, area: std::ops::RangeInclusive<i32>) {
    let (grass, dirt, water) = (
        state.registry.states.grass_block,
        state.registry.states.dirt,
        state.registry.states.water,
    );
    let (log, leaves, stone) = (
        state_id(state, "minecraft:oak_log"),
        state_id(state, "minecraft:oak_leaves"),
        state.registry.states.stone,
    );
    for x in area.clone() {
        for z in area.clone() {
            let top = height(x, z);
            for y in 63..top {
                state.world.set_block(x, y, z, dirt);
            }
            state.world.set_block(x, top - 1, z, grass);
            if pond(x, z) {
                state.world.set_block(x, top - 1, z, water);
            }
            if let Some(depth) = hole(x, z) {
                for y in top - depth..top {
                    state.world.set_block(x, y, z, state.registry.states.air);
                }
            }
            if wall(x, z) {
                for y in top..top + 3 {
                    state.world.set_block(x, y, z, stone);
                }
            }
        }
    }
    for x in area.clone() {
        for z in area.clone() {
            if !tree_at(x, z) || pond(x, z) || wall(x, z) {
                continue;
            }
            let top = height(x, z);
            for y in top..top + 5 {
                state.world.set_block(x, y, z, log);
            }
            for dx in -1..=1 {
                for dz in -1..=1 {
                    for y in top + 3..=top + 5 {
                        if (dx, dz) != (0, 0) || y == top + 5 {
                            state.world.set_block(x + dx, y, z + dz, leaves);
                        }
                    }
                }
            }
        }
    }
    state.world.fold_overlay();
}

/// The flat fixture, every chunk of the staging ticket resident, relief built.
fn relief_fixture() -> TickState {
    let registry = std::sync::Arc::new(crate::registry::CoreRegistry::load(None).unwrap());
    let world = crate::horde_bench::flat_world(&registry);
    let mut state = TickState::new(registry, world, 2, 512);
    let want = (-13..=13)
        .flat_map(|x| (-13..=13).map(move |z| (mc_world::ChunkPos { x, z }, 0)))
        .collect();
    state.world.ensure_blocking(&want);
    relief_world(&mut state, -150..=160);
    state
}

fn stage(state: &mut TickState, side: usize) -> String {
    let mut changes = ChangeSet::default();
    staging::execute(
        state,
        &mut changes,
        ORIGIN,
        &BattleCommand::Setup {
            seed: 42,
            count: side,
        },
    )
    .unwrap();
    let desired = state.desired_chunks();
    state.world.ensure_blocking(&desired);
    state.refresh_active_chunks(state.tick);
    for _ in 0..4 {
        run_tick(state, &[]);
    }
    staging::execute(state, &mut changes, ORIGIN, &BattleCommand::Start).unwrap()
}

#[test]
fn placement_stands_on_steps_and_moves_off_trunks_and_water() {
    let state = relief_fixture();
    let (world, registry) = (&state.world, &state.registry);
    // A one-block step: the body stands on its own column, never inside the step.
    let (x, z) = (5, 3);
    assert_eq!(height(x, z), height(x + 1, z) - 1, "fixture has a step here");
    let at = [f64::from(x) + 0.5, 90.0, f64::from(z) + 0.5];
    assert_eq!(
        place(world, registry, at, 0.3, 1.95),
        Placement::At([at[0], f64::from(height(x, z)), at[2]])
    );
    // A body straddling the step stands on its higher side.
    let straddle = [f64::from(x) + 1.0, 50.0, f64::from(z) + 0.5];
    assert_eq!(
        place(world, registry, straddle, 0.3, 1.95),
        Placement::At([straddle[0], f64::from(height(x + 1, z)), straddle[2]])
    );
    // A trunk is never ground: the nearest clear column under the canopy.
    let trunk = [4.5, 64.0, 5.5];
    assert!(tree_at(4, 5));
    let Placement::At(moved) = place(world, registry, trunk, 0.3, 1.95) else {
        panic!("no ground beside the trunk");
    };
    let shift = mth_dist(moved, trunk);
    assert!(shift > 0.0 && shift <= std::f64::consts::SQRT_2 + 1e-12, "{moved:?}");
    assert_eq!(moved[1], f64::from(height(moved[0].floor() as i32, moved[2].floor() as i32)));
    // Water is never ground.
    let wet = [1.5, 64.0, -29.5];
    assert!(pond(1, -30));
    let Placement::At(dry) = place(world, registry, wet, 0.3, 1.95) else {
        panic!("no dry ground beside the pond");
    };
    assert!(!pond(dry[0].floor() as i32, dry[2].floor() as i32));
    // A chunk that is not resident is unknown, not ground.
    assert_eq!(
        place(world, registry, [5000.5, 64.0, 0.5], 0.3, 1.95),
        Placement::Wait
    );
    // Cave openings are never ground: a slot over one stands beside it, off
    // its rim, and no column beside the footprint drops more than two blocks.
    for slot in [[-30.0, 64.0, 44.5], [-19.5, 64.0, 21.5]] {
        let Placement::At(beside) = place(world, registry, slot, 0.3, 1.95) else {
            panic!("no ground beside the hole at {slot:?}");
        };
        for x in [beside[0] - 1.3, beside[0], beside[0] + 1.3] {
            for z in [beside[2] - 1.3, beside[2], beside[2] + 1.3] {
                assert!(hole(x.floor() as i32, z.floor() as i32).is_none(), "{beside:?}");
            }
        }
    }
}

fn mth_dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    mc_physics::mth::length2(a[0] - b[0], a[2] - b[2])
}

#[test]
fn deflection_keeps_open_ground_and_steps_but_turns_at_trunks_and_walls() {
    let state = relief_fixture();
    let (world, registry) = (&state.world, &state.registry);
    let ground = |x: i32, z: i32| [f64::from(x) + 0.5, f64::from(height(x, z)), f64::from(z) + 0.5];
    let east = [0.08, 0.0];
    let south = [0.0, 0.08];
    // Open ground and a one-block step ahead keep the exact drive (physics jumps it).
    let step = ground(5, 3);
    assert_eq!(deflect(world, registry, step, 0.3, east, east, 1.0), east);
    // A trunk ahead turns the drive; both ends are one block away, so `side` decides.
    let before_trunk = ground(3, 5);
    let turned = deflect(world, registry, before_trunk, 0.3, east, east, 1.0);
    assert_ne!(turned, east);
    assert!(turned[1] > 0.0 && (mc_physics::mth::length2(turned[0], turned[1]) - 0.08).abs() < 1e-12);
    let other = deflect(world, registry, before_trunk, 0.3, east, east, -1.0);
    assert!(other[1] < 0.0);
    // A three-block wall (x -6..=2) turns bodies toward its nearer end, whatever
    // `side` says and however the crowd pushes the drive: the line splits.
    let west_half = deflect(world, registry, ground(-5, -21), 0.3, south, south, -1.0);
    assert!(west_half[0] < 0.0, "{west_half:?}");
    let pushed = [0.03, 0.074];
    let east_half = deflect(world, registry, ground(1, -21), 0.3, pushed, south, 1.0);
    // Lateral dressing toward a slot behind the wall does not pull a body back.
    let dressing = [0.08, 0.0];
    let held = deflect(world, registry, ground(-5, -21), 0.3, dressing, south, 1.0);
    assert!(held[0] < 0.0, "{held:?}");
    assert!(east_half[0] > 0.0, "{east_half:?}");
    // A six-deep cave opening ahead is steered around, not walked into.
    assert_ne!(deflect(world, registry, ground(-20, 19), 0.3, south, south, 1.0), south);
}

#[test]
fn staged_bodies_stand_on_natural_relief_never_inside_floating_or_wet() {
    let mut state = relief_fixture();
    let reply = stage(&mut state, 100);
    // 180 foot soldiers plus 20 horses; riders sit on their horses.
    assert!(
        reply.ends_with("ground: bodies=200 inside=0 floating=0 wet=0"),
        "{reply}"
    );
    let heights: std::collections::BTreeSet<i64> = state
        .hostile
        .battle
        .formation
        .members
        .iter()
        .map(|m| state.entities.pos(state.entities.slot_of(m.id).unwrap())[1] as i64)
        .collect();
    assert!(heights.len() >= 4, "relief not exercised: {heights:?}");
}

/// Movement over relief: the armies cross terraces, the hill, trees and a
/// wall, close to melee and never teleport. The wall and trunks stall files
/// without the drive deflection (the negative control).
#[test]
fn armies_advance_over_relief_to_contact_without_stalls_or_teleports() {
    let mut state = relief_fixture();
    stage(&mut state, 100);
    let mut previous: Vec<Option<[f64; 3]>> = Vec::new();
    let mut contact = None;
    let mut censused = 0;
    for tick in 0..1_200_u64 {
        run_tick(&mut state, &[]);
        let battle = &state.hostile.battle;
        let positions: Vec<Option<[f64; 3]>> = battle
            .formation
            .members
            .iter()
            .map(|m| state.entities.slot_of(m.id).map(|slot| state.entities.pos(slot)))
            .collect();
        for (bi, (now, before)) in positions.iter().zip(&previous).enumerate() {
            if let (Some(now), Some(before)) = (now, before) {
                let horizontal = mth_dist(*now, *before);
                assert!(
                    horizontal <= 1.0 && (now[1] - before[1]).abs() <= 1.5,
                    "member {bi} jumped {before:?} -> {now:?} at tick {tick}"
                );
                let (x, z) = (now[0].floor() as i32, now[2].floor() as i32);
                assert!(
                    hole(x, z).is_none_or(|_| now[1] > f64::from(height(x, z)) - 2.0),
                    "member {bi} fell into a cave opening at {now:?}, tick {tick}"
                );
            }
        }
        previous = positions;
        if battle.elapsed_ticks % crate::battle::formation::STALL_TICKS == 1 {
            let (stalled, advancing) = (battle.formation.stalled, battle.formation.advancing);
            censused = censused.max(advancing);
            if stalled * 50 >= advancing.max(1) {
                for (bi, m) in battle.formation.members.iter().enumerate() {
                    let r = &battle.formation.regiments[m.regiment];
                    let Some(slot) = state.entities.slot_of(m.id) else { continue };
                    let p = state.entities.pos(slot);
                    let (target, _) = battle.formation.destination(bi);
                    let cols: Vec<_> = (-1..=1).flat_map(|dz| (-1..=1).map(move |dx| (dx, dz))).map(|(dx, dz)| {
                        let x = p[0].floor() as i32 + dx; let z = p[2].floor() as i32 + dz;
                        surface(&state.world, &state.registry, x, z).map(|s| s.map(|y| y - p[1].floor()))
                    }).collect();
                    let h = battle.formation.heading_target(bi);
                    let heading = [h[0] - m.pos[0], h[2] - m.pos[2]];
                    let probe = deflect(&state.world, &state.registry, p, 0.3, [0.0, 1.58], heading, 1.0);
                    eprintln!("STALL? reg={} order={:?} file={} rank={} pos={:.2?} target={:.2?} drive={:.3?} vel={:.3?} routing={} wounded={} mpos={:.2?} heading={:.2?} probe={:.3?} cols={:?} speed={:.3}",
                        r.spec.id, r.spec.order, m.file, m.rank, p, target, battle.columns.drive.get(bi), state.entities.vel(slot), m.routing, m.wounded, m.pos, heading, probe, cols, r.speed);
                }
            }
            assert!(
                stalled * 50 < advancing.max(1),
                "{stalled} of {advancing} advancing soldiers stalled at tick {tick}"
            );
        }
        if contact.is_none()
            && battle.formation.regiments.iter().any(|r| {
                r.spec.order != crate::battle::formation::Order::Hold
                    && r.enemy.is_some()
                    && r.gap <= 6.0
                    && r.live_count > 0
            })
        {
            contact = Some(tick);
        }
    }
    assert!(contact.is_some(), "no melee contact over relief");
    assert!(censused >= 100, "stall census saw only {censused} advancing soldiers");
}
