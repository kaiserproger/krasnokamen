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

/// Cave openings with a stone floor (the flat fixture is three blocks thick):
/// a 3x3 one six deep in the second side's infantry path and a 2x2 one eight
/// deep under its front rank (z 44.5, files at x -31 and -30).
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
                state.world.set_block(x, top - depth - 1, z, stone);
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
    // A body 0.42 short of the wall with a slightly oblique heading (the pose
    // that jammed the movement test): the lateral probes drift along the
    // wall's normal, yet the nearer (west) end still wins.
    let oblique = deflect(world, registry, [-3.1307, 65.0, -20.7232], 0.3, [0.0, 1.58], [0.21, 19.62], 1.0);
    assert!(oblique[0] < 0.0, "{oblique:?}");
    // Horses from the stalled movement test. A horse against the trunk at
    // (31, -72) with the oak's canopy (y 68) over its back cannot jump the
    // terrace step to the west (y 67): it goes round the east side.
    let horse = 0.698_242_187_5;
    let pocket = deflect(world, registry, [30.698, 66.0, -72.698], horse, [0.0, 4.6], [0.8, 23.4], 1.0);
    assert!(pocket[0] > 0.0, "{pocket:?}");
    // A horse grazing the trunk at (40, -72) drops the drift into it, or
    // physics refuses the jump onto the terrace step ahead.
    let graze = deflect(world, registry, [39.302, 64.0, -70.698], horse, [0.026, 4.6], [0.2, 21.4], 1.0);
    assert_eq!(graze, [0.0, 4.6]);
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
        // A cavalry charge's knockback throws a body up to ~1.4 blocks in a
        // tick; a staging relocation moves it farther.
        for (bi, (now, before)) in positions.iter().zip(&previous).enumerate() {
            if let (Some(now), Some(before)) = (now, before) {
                let horizontal = mth_dist(*now, *before);
                assert!(
                    horizontal <= 2.0 && (now[1] - before[1]).abs() <= 1.5,
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

/// A soldier who falls into a sinkhole anyway cannot climb out, but the
/// regiment's virtual leader never waits for it: the line keeps advancing.
#[test]
fn a_fighter_in_a_sinkhole_does_not_freeze_its_regiment() {
    let mut state = relief_fixture();
    stage(&mut state, 100);
    let formation = &state.hostile.battle.formation;
    let (fallen, regiment) = formation
        .members
        .iter()
        .find_map(|m| {
            let r = &formation.regiments[m.regiment];
            (r.spec.order == crate::battle::formation::Order::Advance
                && r.spec.frontage <= 1.0
                && m.rank == 0)
                .then_some((m.id, m.regiment))
        })
        .expect("an advancing infantry regiment");
    let r = &formation.regiments[regiment];
    let origin = r.spec.origin;
    let others: Vec<_> = formation.members[r.start..r.start + r.len]
        .iter()
        .map(|m| m.id)
        .filter(|&id| id != fallen)
        .collect();
    let start: Vec<_> = others
        .iter()
        .map(|&id| state.entities.pos(state.entities.slot_of(id).unwrap()))
        .collect();
    // The opening eight deep under the second side's front rank.
    let Some(Ok(floor)) = support(&state.world, &state.registry, [-30.0, 0.0, 45.0], 0.3) else {
        panic!("no floor in the opening");
    };
    assert_eq!(floor, f64::from(height(-30, 45) - 8));
    let slot = state.entities.slot_of(fallen).unwrap();
    state.entities.set_pos(slot, [-30.0, floor, 45.0]);
    for _ in 0..200 {
        run_tick(&mut state, &[]);
    }
    let pit = state.entities.pos(state.entities.slot_of(fallen).expect("the fallen soldier lives"));
    assert!(pit[1] <= floor + 1.0, "the fallen soldier climbed out: {pit:?}");
    let leader = state.hostile.battle.formation.regiments[regiment].spec.origin;
    let advanced = mth_dist(leader, origin);
    assert!(advanced >= 4.0, "the regiment waited for its fallen soldier: leader moved {advanced}");
    let mut moved: Vec<f64> = others
        .iter()
        .zip(&start)
        .map(|(&id, &before)| mth_dist(state.entities.pos(state.entities.slot_of(id).unwrap()), before))
        .collect();
    moved.sort_by(f64::total_cmp);
    let median = moved[moved.len() / 2];
    assert!(median >= 4.0, "the regiment froze: median soldier moved {median}");
}

/// A broken regiment whose flight crosses a cave opening runs round it:
/// fugitives keep the drop avoidance of advancing troops.
#[test]
fn routers_run_round_a_cave_opening_in_their_flight() {
    let mut state = relief_fixture();
    stage(&mut state, 100);
    let formation = &state.hostile.battle.formation;
    let regiment = formation
        .regiments
        .iter()
        .position(|r| {
            r.spec.faction == 2
                && r.spec.order == crate::battle::formation::Order::Advance
                && r.spec.frontage <= 1.0
        })
        .expect("an advancing infantry regiment of the second side");
    let r = &formation.regiments[regiment];
    let ids: Vec<_> = formation.members[r.start..r.start + r.len].iter().map(|m| m.id).collect();
    // The second side flees +z; put the regiment just south of the 3x3
    // opening (x -20..-18, z 20..22) so its flight runs straight over it.
    for (k, &id) in ids.iter().enumerate() {
        let x = -23.5 + (k % 8) as f64;
        let z = 12.5 + (k / 8) as f64;
        let Some(Ok(y)) = support(&state.world, &state.registry, [x, 0.0, z], 0.3) else {
            panic!("no ground at {x} {z}");
        };
        let slot = state.entities.slot_of(id).unwrap();
        state.entities.set_pos(slot, [x, y, z]);
    }
    let formation = &mut state.hostile.battle.formation;
    formation.regiments[regiment].morale.broken = true;
    let r = &formation.regiments[regiment];
    for m in &mut formation.members[r.start..r.start + r.len] {
        m.routing = true;
    }
    let start: Vec<f64> = ids.iter().map(|&id| state.entities.pos(state.entities.slot_of(id).unwrap())[2]).collect();
    for tick in 0..200 {
        run_tick(&mut state, &[]);
        for &id in &ids {
            let Some(slot) = state.entities.slot_of(id) else { continue };
            let p = state.entities.pos(slot);
            let (x, z) = (p[0].floor() as i32, p[2].floor() as i32);
            assert!(
                hole(x, z).is_none_or(|_| p[1] > f64::from(height(x, z)) - 2.0),
                "a fugitive fell into the cave opening at {p:?}, tick {tick}"
            );
        }
    }
    let mut fled: Vec<f64> = ids
        .iter()
        .zip(&start)
        .filter_map(|(&id, &z)| state.entities.slot_of(id).map(|slot| state.entities.pos(slot)[2] - z))
        .collect();
    fled.sort_by(f64::total_cmp);
    let median = fled[fled.len() / 2];
    assert!(median >= 8.0, "the regiment did not flee: median {median}");
}

/// A cave mouth or overhang is never walked into from open ground (fugitives
/// in the G1 take walked down a ravine ramp into a cave and vanished
/// underground); a body already under the roof walks out.
#[test]
fn bodies_never_walk_from_open_ground_under_a_terrain_roof() {
    let mut state = relief_fixture();
    let stone = state.registry.states.stone;
    for x in 106..=108 {
        for z in -100..=-98 {
            state.world.set_block(x, height(x, z) + 3, z, stone);
        }
    }
    state.world.fold_overlay();
    let (world, registry) = (&state.world, &state.registry);
    let ground = |x: i32, z: i32| [f64::from(x) + 0.5, f64::from(height(x, z)), f64::from(z) + 0.5];
    let east = [1.0, 0.0];
    assert!(!crossable(world, registry, ground(105, -99), 0.3, east), "walked under the roof");
    assert!(crossable(world, registry, ground(105, -97), 0.3, east), "open ground beside it");
    assert!(crossable(world, registry, ground(108, -99), 0.3, east), "a body under it walks out");
}

/// A body under rock (pushed or knocked into a cave mouth) walks out toward
/// the nearest open sky instead of following its heading deeper in.
#[test]
fn a_body_under_a_terrain_roof_walks_out_toward_open_sky() {
    let mut state = relief_fixture();
    let stone = state.registry.states.stone;
    for x in 106..=112 {
        for z in -102..=-96 {
            state.world.set_block(x, height(x, z) + 3, z, stone);
        }
    }
    state.world.fold_overlay();
    let (world, registry) = (&state.world, &state.registry);
    // Two blocks in from the roof's west edge, five from its east edge.
    let inside = [107.5, f64::from(height(107, -99)), -98.5];
    let east = [0.079, 0.0];
    let out = deflect(world, registry, inside, 0.3, east, [20.0, 0.0], 1.0);
    assert!(out[0] < 0.0, "went deeper under the roof: {out:?}");
    // A body holding its slot under the roof (no drive) walks out too.
    let held = deflect(world, registry, inside, 0.3, [0.0, 0.0], [0.0, 0.0], 1.0);
    assert!(held[0] < -0.05, "stayed under the roof: {held:?}");
    // Under open sky the same drive, or no drive, is returned exactly.
    let open = [107.5, f64::from(height(107, -94)), -93.5];
    assert_eq!(deflect(world, registry, open, 0.3, east, [20.0, 0.0], 1.0), east);
    assert_eq!(deflect(world, registry, open, 0.3, [0.0, 0.0], [20.0, 0.0], 1.0), [0.0, 0.0]);
}

/// The overhang where a fugitive stayed at the end of the d0cabbab and
/// 6d1cf207 site-935 takes: columns x -186..=-180 (left to right), z
/// 19..=25 (top to bottom), y 83..=95 with the body's feet at 85, `#` solid
/// (`receipts/BvReliefBattle/rout-falls/probe-d0cabbab.json`). The body
/// stood at (-182.6, 85, 22.8), under dirt at 88..=91. North of it, (-184,
/// 23) and (-183, 23) drop past y 83; (-182, 23) is a one-block step up.
const POCKET: [[&str; 7]; 7] = [
    ["############.", "############.", ".###########.", "..##########.", "..##########.", ".###########.", "############."],
    ["############.", "#.##########.", "..##########.", "...#########.", "....########.", "...#########.", "..##########."],
    ["###########..", "##..#######..", "##..#######..", "#....######..", "......#####..", ".....######..", "....#######.."],
    ["####.#####...", "####.####....", "###..####....", "##...####....", "##...####....", "#....####....", ".....####...."],
    ["####.........", "##...........", ".............", ".............", "###..........", "##...........", "##..........."],
    [".............", ".............", ".............", ".............", ".............", "#............", "##..........."],
    [".............", ".............", ".............", ".............", "#............", "##...........", "###.........."],
];

/// A body under an overhang whose nearest open columns are drops walks out
/// by the step up beside them instead of standing at the drop's edge.
#[test]
fn a_body_under_an_overhang_walks_out_by_the_step_not_the_drop() {
    const FEET: i32 = 70;
    let mut state = relief_fixture();
    let (stone, air) = (state.registry.states.stone, state.registry.states.air);
    // (-186, 19) maps to (140, -103); y 83 to FEET - 2. Each column's lowest
    // cell continues down to a stone floor at FEET - 7.
    for (dz, row) in POCKET.iter().enumerate() {
        for (dx, column) in row.iter().enumerate() {
            let (x, z) = (140 + dx as i32, -103 + dz as i32);
            for y in FEET - 7..=FEET + 12 {
                let cell = column.as_bytes().get((y - FEET + 2).max(0) as usize);
                let solid = y == FEET - 7 || cell == Some(&b'#');
                state.world.set_block(x, y, z, if solid { stone } else { air });
            }
        }
    }
    state.world.fold_overlay();
    let (world, registry) = (&state.world, &state.registry);
    let mut pos = [143.4, f64::from(FEET), -99.2];
    let out = exit(world, registry, pos).expect("under the overhang");
    let target = ((pos[0] + out[0]).floor() as i32, (pos[2] + out[1]).floor() as i32);
    assert_eq!(target, (144, -99), "the way out is the step, not the drop: {out:?}");
    // Holding its slot (no drive), the body walks out and up the step.
    let solid = |p: [f64; 3]| {
        world
            .physics_block_at(p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32)
            .is_none_or(|state| registry.materials.classify(state) == BlockMaterial::Solid)
    };
    for _ in 0..40 {
        if exit(world, registry, pos).is_none() {
            break;
        }
        let drive = deflect(world, registry, pos, 0.3, [0.0, 0.0], [0.0, 0.0], 1.0);
        pos[0] += drive[0] * mc_physics::TICK_SECONDS;
        pos[2] += drive[1] * mc_physics::TICK_SECONDS;
        if solid(pos) {
            pos[1] += 1.0;
        }
        assert!(solid([pos[0], pos[1] - 1.0, pos[2]]), "walked over the drop at {pos:?}");
    }
    assert!(exit(world, registry, pos).is_none(), "still under the overhang at {pos:?}");
    assert_eq!(pos[1], f64::from(FEET + 1), "out by the step: {pos:?}");
}

/// A rider whose horse is gone walks under the same drop, cliff and cave
/// rules as foot, although its regiment keeps the cavalry frontage.
#[test]
fn dismounted_riders_run_round_a_cave_opening_in_their_flight() {
    let mut state = relief_fixture();
    stage(&mut state, 100);
    let formation = &state.hostile.battle.formation;
    let regiment = formation
        .regiments
        .iter()
        .position(|r| r.spec.faction == 2 && r.spec.frontage > 1.0)
        .expect("a cavalry regiment of the second side");
    let r = &formation.regiments[regiment];
    let ids: Vec<_> = formation.members[r.start..r.start + r.len].iter().map(|m| m.id).collect();
    assert!(ids.len() >= 3, "too few riders: {}", ids.len());
    // The second side flees +z; three of the riders' files run straight over
    // the 3x3 opening (x -20..-18, z 20..22).
    for (k, &id) in ids.iter().enumerate() {
        crate::mounts::dismount(&mut state, id);
        let slot = state.entities.slot_of(id).unwrap();
        assert!(state.entities.row(slot).entity_life.mount.vehicle.is_none(), "still riding");
        let (x, z) = (-21.5 + k as f64, 12.5);
        let Some(Ok(y)) = support(&state.world, &state.registry, [x, 0.0, z], 0.3) else {
            panic!("no ground at {x} {z}");
        };
        state.entities.set_pos(slot, [x, y, z]);
    }
    let formation = &mut state.hostile.battle.formation;
    formation.regiments[regiment].morale.broken = true;
    let r = &formation.regiments[regiment];
    for m in &mut formation.members[r.start..r.start + r.len] {
        m.routing = true;
    }
    let start: Vec<f64> = ids.iter().map(|&id| state.entities.pos(state.entities.slot_of(id).unwrap())[2]).collect();
    for tick in 0..200 {
        run_tick(&mut state, &[]);
        for &id in &ids {
            let Some(slot) = state.entities.slot_of(id) else { continue };
            let p = state.entities.pos(slot);
            let (x, z) = (p[0].floor() as i32, p[2].floor() as i32);
            assert!(
                hole(x, z).is_none_or(|_| p[1] > f64::from(height(x, z)) - 2.0),
                "a dismounted rider fell into the cave opening at {p:?}, tick {tick}"
            );
        }
    }
    let mut fled: Vec<f64> = ids
        .iter()
        .zip(&start)
        .filter_map(|(&id, &z)| state.entities.slot_of(id).map(|slot| state.entities.pos(slot)[2] - z))
        .collect();
    fled.sort_by(f64::total_cmp);
    let median = fled[fled.len() / 2];
    assert!(median >= 8.0, "the riders did not flee: median {median}");
}

/// Ground heights (first air above the ground) of the G1 site's gully end,
/// where 20-40 fleeing blue riders stood pressed together from t 90 to the
/// end of rehearsals r2-r5: columns x 8..=21 (left to right), rows z
/// 233..=250 (top to bottom), from `receipts/BvReliefBattle/gully/probe.json`
/// (no overhangs in this patch). The gully floor (x 19-20, 89-91) runs up
/// to a shelf at (19, 93, 245), shut in by two-block terraces to the west,
/// walls to the east and south and a three-block drop beside the shelf.
const GULLY: [[i32; 14]; 18] = [
    [99, 98, 98, 97, 96, 95, 94, 93, 92, 92, 91, 90, 89, 97],
    [99, 98, 98, 97, 96, 95, 94, 93, 92, 92, 91, 90, 89, 97],
    [99, 98, 98, 97, 96, 95, 94, 93, 92, 92, 91, 90, 89, 97],
    [99, 98, 98, 97, 96, 95, 94, 93, 93, 92, 91, 90, 89, 88],
    [99, 98, 98, 97, 96, 95, 94, 94, 93, 92, 91, 90, 89, 90],
    [99, 98, 98, 97, 96, 96, 95, 94, 93, 92, 91, 90, 89, 92],
    [99, 98, 98, 97, 97, 96, 95, 94, 93, 92, 91, 90, 89, 93],
    [99, 98, 98, 97, 97, 96, 95, 94, 93, 93, 92, 91, 89, 93],
    [99, 99, 98, 97, 97, 97, 96, 95, 94, 93, 92, 91, 90, 97],
    [99, 99, 98, 97, 97, 97, 97, 96, 95, 94, 92, 91, 90, 97],
    [100, 99, 98, 98, 97, 97, 97, 96, 95, 94, 93, 92, 90, 97],
    [101, 99, 98, 98, 97, 97, 97, 97, 96, 95, 94, 92, 90, 97],
    [101, 100, 99, 98, 97, 97, 97, 97, 97, 97, 95, 93, 91, 97],
    [102, 100, 99, 98, 97, 97, 97, 97, 97, 97, 97, 96, 93, 97],
    [103, 101, 99, 98, 97, 97, 97, 97, 97, 97, 97, 97, 97, 97],
    [103, 101, 99, 98, 98, 97, 97, 97, 97, 97, 97, 97, 97, 98],
    [103, 102, 100, 99, 98, 98, 98, 98, 97, 97, 97, 97, 97, 98],
    [103, 102, 101, 99, 99, 98, 98, 98, 98, 97, 97, 97, 97, 98],
];

/// Fleeing riders that ran up a gully into its dead end climb back out
/// instead of standing pressed into it for the rest of the battle.
#[test]
fn fleeing_riders_climb_out_of_a_gully_end() {
    // Real (x, y, z) to fixture: the shelf (19, 93, 245) lands at (7, 70,
    // 102), straight behind the second side, which flees +z.
    const SHIFT: [i32; 3] = [-12, -23, -143];
    let mut state = relief_fixture();
    let (stone, air) = (state.registry.states.stone, state.registry.states.air);
    for (dz, row) in GULLY.iter().enumerate() {
        for (dx, &top) in row.iter().enumerate() {
            let (x, z) = (8 + dx as i32 + SHIFT[0], 233 + dz as i32 + SHIFT[2]);
            let top = top + SHIFT[1];
            for y in 60..top + 10 {
                state.world.set_block(x, y, z, if y < top { stone } else { air });
            }
        }
    }
    state.world.fold_overlay();
    stage(&mut state, 100);
    let formation = &state.hostile.battle.formation;
    let regiment = formation
        .regiments
        .iter()
        .position(|r| r.spec.faction == 2 && r.spec.frontage > 1.0)
        .expect("a cavalry regiment of the second side");
    let r = &formation.regiments[regiment];
    let ids: Vec<_> = formation.members[r.start..r.start + r.len].iter().map(|m| m.id).collect();
    let pocket = [8.0, 102.3];
    let mut horses = Vec::new();
    for (k, &id) in ids.iter().enumerate() {
        let horse = crate::mounts::vehicle(&state, id).expect("a mounted rider");
        let horse_id = state.entities.body(horse).id;
        // Where the r5 horses stood: pressed into the shelf's south-east corner.
        state.entities.set_pos(horse, [pocket[0] + 0.1 * (k % 4) as f64, 70.0, pocket[1]]);
        crate::mounts::sync_rider(&mut state, horse);
        horses.push(horse_id);
    }
    let formation = &mut state.hostile.battle.formation;
    formation.regiments[regiment].morale.broken = true;
    let r = &formation.regiments[regiment];
    for m in &mut formation.members[r.start..r.start + r.len] {
        m.routing = true;
    }
    for _ in 0..200 {
        run_tick(&mut state, &[]);
    }
    let inside: Vec<_> = horses
        .iter()
        .filter_map(|&id| state.entities.slot_of(id).map(|slot| state.entities.pos(slot)))
        .filter(|p| p[1] < 72.0 && mc_physics::mth::length2(p[0] - pocket[0], p[2] - pocket[1]) < 3.0)
        .collect();
    assert!(inside.is_empty(), "riders still boxed in the gully end: {inside:?}");
}

/// The G1 site's cave tunnel under an undercut cliff where 14 red foot
/// stood at the end of rehearsal r7, at (-72.7, 87, 260.3): blocks of columns
/// x -78..=-64 (left to right), rows z 256..=267, cells y 84..=97 (`#` solid),
/// from `receipts/BvReliefBattle/cave-piles/probe.json`. The tunnel (feet 86-87,
/// under 4-6 blocks of rock) opens east onto a valley floor at x -70..-68.
const SITE_A: [[&str; 15]; 12] = [
    ["###########...", "###########...", "###########...", "############..", "############..", "############..", "############..", "############..", "############..", "############..", "############..", "############..", "############..", "############..", "############.."],
    ["###########...", "###########...", "###########...", "###########...", "###########...", "############..", "############..", "############..", "############..", "############..", "###########...", "###########...", "############..", "############..", "############.."],
    ["###########...", "###########...", "###########...", "###########...", "###########...", "###########...", "############..", "###########...", "###########...", "###########...", "###########...", "###########...", "###########...", "###########...", "###########..."],
    ["##########....", "###########...", "###########...", "###########...", "###########...", "###########...", "###########...", "#####.#####...", "##########....", "##########....", "##########....", "########......", "#########.....", "##########....", "##########...."],
    ["#..#######....", "##.#######....", "##..######....", "##..#######...", "##..#######...", "###..######...", "###..######...", "###...####....", "###...####....", "###..####.....", "###...........", "###...........", "###...........", "###...........", "####.........."],
    ["#...#####.....", "#...#####.....", "#...#####.....", "#...#####.....", "#....#####....", "##...#####....", "##....####....", "##...####.....", "##...####.....", "##............", "##............", "##............", "###...........", "###...........", "####.........."],
    ["#...####......", "#...####......", "#...####......", "#...####......", "#...####......", "#...####......", "#....####.....", "#...####......", "##............", "##............", "##............", "###...........", "###...........", "####..........", "#####........."],
    ["#.####........", "#..####.......", "#..####.......", "#..####.......", "#..####.......", "#..####.......", "##.####.......", "##.####.......", "##............", "##............", "###...........", "###...........", "###...........", "####..........", "######........"],
    ["#####.........", "#.####........", "#.####........", "#.####........", "######........", "######........", "#####.........", "#####.........", "##............", "###...........", "###...........", "###...........", "####..........", "####..........", "####.........."],
    ["#####.........", "#####.........", "#####.........", "#####.........", "#####.........", "#####.........", "#####.........", "#####.........", "##............", "###...........", "###...........", "####..........", "####..........", "####..........", "####.........."],
    ["#####.........", "#####.........", "#####.........", "#####.........", "#####.........", "####..........", "####..........", "####..........", "###...........", "###...........", "###...........", "####..........", "####..........", "####..........", "####.........."],
    ["####..........", "####..........", "####..........", "####..........", "####..........", "####..........", "####..........", "####..........", "####..........", "###...........", "####..........", "####..........", "###...........", "###...........", "###..........."],
];

/// Builds [`SITE_A`] into the relief fixture: real (x, y, z) + SITE_A_SHIFT.
const SITE_A_SHIFT: [i32; 3] = [178, -20, -156];

fn site_a(state: &mut TickState) {
    let (stone, air) = (state.registry.states.stone, state.registry.states.air);
    for (dz, row) in SITE_A.iter().enumerate() {
        for (dx, column) in row.iter().enumerate() {
            let (x, z) = (-78 + dx as i32 + SITE_A_SHIFT[0], 256 + dz as i32 + SITE_A_SHIFT[2]);
            for y in 58..=90 {
                let cell = (y - 84 - SITE_A_SHIFT[1]) as usize;
                let solid = y < 84 + SITE_A_SHIFT[1] || column.as_bytes().get(cell) == Some(&b'#');
                state.world.set_block(x, y, z, if solid { stone } else { air });
            }
        }
    }
    state.world.fold_overlay();
}

/// A body in the r7 tunnel pile walks out toward the valley mouth whatever
/// its drive, at walking pace at least.
#[test]
fn a_body_in_the_r7_tunnel_pile_walks_out() {
    let mut state = relief_fixture();
    site_a(&mut state);
    let (world, registry) = (&state.world, &state.registry);
    let pile = [-72.7 + 178.0, 67.0, 260.3 - 156.0];
    let mouth = [-69.5 + 178.0 - pile[0], 262.5 - 156.0 - pile[2]];
    for drive in [[-1.58, -1.58], [0.0, -2.0], [-2.0, 0.0], [1.58, 1.58], [0.0, 0.0], [-5.6, 0.0]] {
        let out = deflect(world, registry, pile, 0.3, drive, drive, 1.0);
        let along = (out[0] * mouth[0] + out[1] * mouth[1]) / mc_physics::mth::length2(mouth[0], mouth[1]);
        assert!(along > 1.0, "drive {drive:?} -> {out:?}: not walking out (along {along})");
    }
}

/// Foot placed in the r7 tunnel pile leave the tunnel under ordinary orders.
#[test]
fn foot_in_the_r7_tunnel_pile_leave_it() {
    let mut state = relief_fixture();
    site_a(&mut state);
    stage(&mut state, 100);
    let formation = &state.hostile.battle.formation;
    let regiment = formation
        .regiments
        .iter()
        .position(|r| r.spec.faction == 1 && r.spec.frontage <= 1.0)
        .expect("a foot regiment of the first side");
    let r = &formation.regiments[regiment];
    let ids: Vec<_> = formation.members[r.start..r.start + r.len].iter().map(|m| m.id).take(14).collect();
    let pile = [-72.7 + 178.0, 67.0, 260.3 - 156.0];
    // Packed as in r7: identical to 0.1 block, never exactly coincident.
    for (k, &id) in ids.iter().enumerate() {
        let slot = state.entities.slot_of(id).unwrap();
        let k = k as f64;
        state.entities.set_pos(slot, [pile[0] + 0.004 * (k % 4.0), pile[1], pile[2] + 0.004 * (k / 4.0).floor()]);
    }
    for _ in 0..300 {
        run_tick(&mut state, &[]);
    }
    let (world, registry) = (&state.world, &state.registry);
    let inside: Vec<_> = ids
        .iter()
        .filter_map(|&id| state.entities.slot_of(id).map(|slot| state.entities.pos(slot)))
        .filter(|p| {
            let feet = (p[1] + 1e-3).floor() as i32;
            cover(world, registry, p[0].floor() as i32, p[2].floor() as i32, feet + 2) > 0
        })
        .collect();
    assert!(inside.is_empty(), "foot still in the tunnel: {inside:?}");
}

/// A body on foot boxed in facing a cave mouth (r7: a blue pile in a
/// sinkhole, its tunnel north) stands; it no longer keeps a drive that
/// physics walks under the rock the rules refused. A rider keeps it.
#[test]
fn a_boxed_in_body_on_foot_stands_at_a_cave_mouth() {
    let mut state = relief_fixture();
    let (stone, air) = (state.registry.states.stone, state.registry.states.air);
    for x in 120..=124 {
        for z in -121..=-115 {
            for y in 60..=78 {
                let shaft = (x, z) == (122, -118) && y >= 66;
                let tunnel = (x, z) == (122, -119) && (66..=67).contains(&y);
                let solid = y <= 72 && !shaft && !tunnel;
                state.world.set_block(x, y, z, if solid { stone } else { air });
            }
        }
    }
    state.world.fold_overlay();
    let (world, registry) = (&state.world, &state.registry);
    let pos = [122.5, 66.0, -117.5];
    let north = [0.0, -1.58];
    assert_eq!(deflect_foot(world, registry, pos, 0.3, north, north, 1.0), [0.0, 0.0], "walked into the mouth");
    assert_eq!(deflect(world, registry, pos, 0.3, north, north, 1.0), north, "a rider keeps pressing");
}

/// First air above the ground round the G2 take's cliff notch, where 87
/// fleeing blue foot stood within 0.22 blocks of (-113.3, 94, 257.3) from
/// t 232 to the end of the take (receipts/BvHorseStack): columns x
/// -126..=-98 (left to right), rows z 244..=268 (top to bottom), from the
/// `battle_site_scan detail` columns of site 935 (no overhangs; the trees
/// below the cliff are left out). The upper field (94-95) ends in a cliff
/// four to six blocks high; at x -114 a one-block notch (94) juts out one
/// row further than the cliff to its west.
const G2_LEDGE: [[i32; 29]; 25] = [
    [
        94, 93, 93, 93, 93, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94,
        94, 94, 95, 95, 95, 95,
    ],
    [
        94, 94, 93, 93, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94,
        94, 94, 95, 95, 95, 95,
    ],
    [
        94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94,
        94, 94, 95, 95, 95, 95,
    ],
    [
        94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94,
        94, 94, 95, 95, 95, 95,
    ],
    [
        94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94,
        94, 94, 95, 95, 95, 95,
    ],
    [
        94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94,
        94, 94, 95, 95, 95, 95,
    ],
    [
        94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94,
        94, 95, 95, 95, 95, 95,
    ],
    [
        94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94,
        94, 95, 95, 95, 95, 95,
    ],
    [
        94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94,
        95, 95, 95, 95, 95, 95,
    ],
    [
        94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 95, 95, 95, 95, 94,
        95, 95, 95, 95, 95, 95,
    ],
    [
        94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 95, 95, 95, 95, 95, 95, 95, 95,
        95, 95, 95, 95, 95, 95,
    ],
    [
        87, 87, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 95, 95, 95, 95, 95, 95, 95, 95, 95, 95,
        95, 95, 95, 95, 95, 95,
    ],
    [
        91, 89, 88, 89, 89, 89, 94, 94, 94, 94, 94, 95, 95, 95, 95, 95, 95, 95, 95, 95, 95, 95, 95,
        95, 95, 95, 95, 95, 95,
    ],
    [
        94, 93, 91, 89, 90, 90, 90, 90, 90, 90, 90, 89, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94,
        94, 94, 94, 94, 94, 94,
    ],
    [
        95, 95, 95, 95, 94, 92, 91, 91, 91, 91, 91, 91, 90, 90, 90, 90, 90, 90, 89, 89, 89, 93, 93,
        93, 93, 93, 93, 93, 92,
    ],
    [
        95, 95, 95, 95, 94, 94, 94, 94, 94, 94, 93, 92, 91, 91, 91, 91, 91, 91, 90, 90, 90, 89, 89,
        88, 92, 92, 92, 91, 91,
    ],
    [
        95, 95, 95, 95, 95, 94, 94, 94, 94, 94, 94, 94, 93, 93, 92, 92, 92, 92, 92, 92, 91, 90, 90,
        89, 89, 88, 88, 88, 87,
    ],
    [
        94, 94, 94, 94, 94, 94, 93, 93, 93, 93, 93, 93, 93, 92, 91, 91, 91, 91, 91, 91, 91, 90, 90,
        90, 90, 89, 88, 88, 88,
    ],
    [
        94, 94, 94, 93, 93, 93, 93, 93, 93, 93, 93, 92, 92, 91, 91, 91, 91, 91, 91, 90, 90, 90, 90,
        90, 90, 90, 89, 88, 88,
    ],
    [
        94, 93, 93, 93, 92, 92, 92, 92, 92, 92, 92, 91, 91, 91, 91, 90, 90, 90, 90, 90, 90, 89, 89,
        89, 89, 89, 90, 89, 88,
    ],
    [
        93, 93, 92, 92, 91, 91, 91, 91, 91, 91, 91, 91, 90, 90, 90, 90, 90, 90, 89, 89, 89, 89, 89,
        89, 89, 89, 89, 89, 89,
    ],
    [
        93, 93, 92, 92, 92, 91, 91, 91, 91, 91, 91, 91, 91, 90, 90, 90, 90, 90, 90, 89, 89, 89, 89,
        89, 89, 89, 89, 89, 89,
    ],
    [
        93, 93, 92, 92, 92, 92, 91, 91, 91, 91, 91, 91, 91, 90, 90, 90, 90, 90, 90, 90, 89, 89, 89,
        89, 89, 89, 89, 89, 89,
    ],
    [
        93, 93, 92, 92, 92, 92, 92, 91, 91, 91, 91, 91, 91, 90, 90, 90, 90, 90, 90, 90, 90, 89, 89,
        89, 89, 89, 89, 89, 89,
    ],
    [
        94, 93, 92, 92, 92, 92, 92, 92, 91, 91, 91, 91, 91, 91, 90, 90, 90, 90, 90, 90, 90, 89, 89,
        89, 89, 89, 89, 89, 89,
    ],
];

/// Real (x, y, z) to fixture: the notch (-113.3, 94, 257.3) lands at
/// (6.7, 66, 100.3), behind the second side, which flees +z.
const G2_LEDGE_SHIFT: [i32; 3] = [120, -28, -157];

fn g2_ledge(state: &mut TickState) {
    let (stone, air) = (state.registry.states.stone, state.registry.states.air);
    for (dz, row) in G2_LEDGE.iter().enumerate() {
        for (dx, &top) in row.iter().enumerate() {
            let (x, z) = (
                -126 + dx as i32 + G2_LEDGE_SHIFT[0],
                244 + dz as i32 + G2_LEDGE_SHIFT[2],
            );
            let top = top + G2_LEDGE_SHIFT[1];
            for y in 55..top + 10 {
                state
                    .world
                    .set_block(x, y, z, if y < top { stone } else { air });
            }
        }
    }
    state.world.fold_overlay();
}

/// Most bodies within 0.5 blocks (horizontal, same level) of one of them.
fn densest(points: &[[f64; 3]]) -> usize {
    points
        .iter()
        .map(|a| {
            points
                .iter()
                .filter(|b| {
                    (a[1] - b[1]).abs() < 1.5
                        && mc_physics::mth::length2(a[0] - b[0], a[2] - b[2]) < 0.5
                })
                .count()
        })
        .max()
        .unwrap_or(0)
}

/// Fugitives on foot that run into the G2 cliff and along it into the notch
/// keep apart there: in the G2 take 87 stood on one point for the last
/// 25 s, one body for the camera that was meant to film them.
#[test]
fn fugitives_pressed_into_a_cliff_notch_do_not_heap_on_one_point() {
    let mut state = relief_fixture();
    g2_ledge(&mut state);
    stage(&mut state, 100);
    let formation = &state.hostile.battle.formation;
    let regiments: Vec<usize> = formation
        .regiments
        .iter()
        .enumerate()
        .filter(|(_, r)| r.spec.faction == 2 && r.spec.frontage <= 1.0)
        .map(|(i, _)| i)
        .collect();
    let ids: Vec<_> = regiments
        .iter()
        .flat_map(|&i| {
            let r = &formation.regiments[i];
            formation.members[r.start..r.start + r.len]
                .iter()
                .map(|m| m.id)
        })
        .take(48)
        .collect();
    assert!(ids.len() >= 40, "too few foot: {}", ids.len());
    // South-west of the notch, as the G2 fugitives came: real x -123..-112,
    // z 246..250.
    for (k, &id) in ids.iter().enumerate() {
        let x = -123.5 + f64::from(G2_LEDGE_SHIFT[0]) + (k % 12) as f64;
        let z = 246.5 + f64::from(G2_LEDGE_SHIFT[2]) + (k / 12) as f64;
        let Some(Ok(y)) = support(&state.world, &state.registry, [x, 0.0, z], 0.3) else {
            panic!("no ground at {x} {z}");
        };
        let slot = state.entities.slot_of(id).unwrap();
        state.entities.set_pos(slot, [x, y, z]);
    }
    let formation = &mut state.hostile.battle.formation;
    for &i in &regiments {
        formation.regiments[i].morale.broken = true;
        let r = &formation.regiments[i];
        for m in &mut formation.members[r.start..r.start + r.len] {
            m.routing = true;
        }
    }
    let positions = |state: &TickState| -> Vec<[f64; 3]> {
        ids.iter()
            .filter_map(|&id| {
                state
                    .entities
                    .slot_of(id)
                    .map(|slot| state.entities.pos(slot))
            })
            .collect()
    };
    let start: Vec<f64> = positions(&state).iter().map(|p| p[2]).collect();
    let mut worst = (0, 0);
    for tick in 1..=800 {
        run_tick(&mut state, &[]);
        if tick % 50 == 0 {
            worst = worst.max((densest(&positions(&state)), tick));
        }
    }
    let end = positions(&state);
    let mut fled: Vec<f64> = end.iter().zip(&start).map(|(p, z)| p[2] - z).collect();
    fled.sort_by(f64::total_cmp);
    assert!(
        fled[fled.len() / 2] >= 5.0,
        "the fugitives did not run to the cliff: {end:?}"
    );
    assert!(
        worst.0 <= 4,
        "fugitives heaped: {} bodies within 0.5 blocks of one at tick {}: {end:?}",
        worst.0,
        worst.1
    );
}
