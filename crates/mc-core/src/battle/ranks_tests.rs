//! Rank and heap metrics of a staged battle on real native ticks: how well soldiers hold their
//! slots before contact and how many stand in heaps. The table test is a diagnostic
//! (`BATTLE_UNITS_PER_SIDE`, `BATTLE_RANKS_TICKS`); the gates below are ordinary tests.
use super::tests::staged_flat_fixture;
use super::*;
use rustc_hash::FxHashMap;

/// Two bodies this close are one heap; a dressed line keeps a block between files.
const HEAP: f64 = 0.5;
/// Slot error beyond this is a body out of its rank or file.
const OFF_SLOT: f64 = 1.5;

#[derive(Debug, Default, Clone, Copy)]
pub(super) struct Census {
    /// Steady bodies of regiments out of melee reach.
    pub marching: usize,
    /// Of those: further than `OFF_SLOT` from the formation slot.
    pub off_slot: usize,
    pub slot_error_sum: f64,
    /// Bodies with another live body within `HEAP`, foot and horse apart.
    pub heaped_foot: usize,
    pub heaped_horse: usize,
    pub foot: usize,
    pub horse: usize,
    /// Most bodies in one two-block cell.
    pub densest: usize,
    /// Mean over marching foot regiments of the deviation of front rank depth from a line, blocks.
    pub front_wobble: f64,
    pub engaged_regiments: usize,
}

pub(super) fn census(battle: &BattleState) -> Census {
    let f = &battle.formation;
    let mut c = Census::default();
    let mut cells: FxHashMap<[i32; 2], Vec<usize>> = FxHashMap::default();
    let mut density: FxHashMap<[i32; 2], usize> = FxHashMap::default();
    for (bi, m) in f.members.iter().enumerate() {
        if !m.alive {
            continue;
        }
        cells
            .entry([m.pos[0].floor() as i32, m.pos[2].floor() as i32])
            .or_default()
            .push(bi);
        *density
            .entry([(m.pos[0] / 2.0).floor() as i32, (m.pos[2] / 2.0).floor() as i32])
            .or_default() += 1;
        let horse = f.regiments[m.regiment].spec.frontage > 1.0;
        if horse {
            c.horse += 1;
        } else {
            c.foot += 1;
        }
    }
    c.densest = density.values().copied().max().unwrap_or(0);
    for (bi, m) in f.members.iter().enumerate() {
        if !m.alive {
            continue;
        }
        let horse = f.regiments[m.regiment].spec.frontage > 1.0;
        let key = [m.pos[0].floor() as i32, m.pos[2].floor() as i32];
        let mut heaped = false;
        'near: for dx in -1..=1 {
            for dz in -1..=1 {
                if let Some(list) = cells.get(&[key[0] + dx, key[1] + dz]) {
                    for &other in list {
                        let o = &f.members[other];
                        if other != bi
                            && (o.pos[0] - m.pos[0]).hypot(o.pos[2] - m.pos[2]) < HEAP
                        {
                            heaped = true;
                            break 'near;
                        }
                    }
                }
            }
        }
        if heaped {
            if horse {
                c.heaped_horse += 1;
            } else {
                c.heaped_foot += 1;
            }
        }
    }
    let mut wobble = (0.0, 0);
    for r in &f.regiments {
        if r.gap <= 6.0 || r.morale.broken || r.live_count == 0 {
            c.engaged_regiments += usize::from(r.gap <= 6.0 && r.enemy.is_some());
            continue;
        }
        let mut front = Vec::new();
        for bi in r.start..r.start + r.len {
            let m = &f.members[bi];
            if !m.alive || m.wounded || m.routing {
                continue;
            }
            c.marching += 1;
            let slot = f.heading_target(bi);
            let error = (slot[0] - m.pos[0]).hypot(slot[2] - m.pos[2]);
            c.slot_error_sum += error;
            c.off_slot += usize::from(error > OFF_SLOT);
            if m.rank == 0 && r.spec.frontage <= 1.0 {
                front.push((m.pos[0] - slot[0]) * r.spec.facing[0] + (m.pos[2] - slot[2]) * r.spec.facing[1]);
            }
        }
        if front.len() > 1 {
            let mean = front.iter().sum::<f64>() / front.len() as f64;
            let var = front.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / front.len() as f64;
            wobble.0 += var.sqrt();
            wobble.1 += 1;
        }
    }
    c.front_wobble = wobble.0 / f64::from(wobble.1.max(1));
    c
}

fn dead(state: &crate::tick::TickState) -> usize {
    let (dead, _, _) = super::tests::native_battle_losses(state);
    dead[0] + dead[1]
}

/// Per-100-tick table of the staged flat battle. Env: BATTLE_UNITS_PER_SIDE (1000),
/// BATTLE_RANKS_TICKS (4000).
#[test]
#[ignore = "diagnostic table; run explicitly in release"]
fn rank_and_heap_table() {
    let side = std::env::var("BATTLE_UNITS_PER_SIDE").map_or(1000, |v| v.parse().unwrap());
    let ticks = std::env::var("BATTLE_RANKS_TICKS").map_or(4000, |v| v.parse::<usize>().unwrap());
    let mut state = staged_flat_fixture(side);
    staging::execute(
        &mut state,
        &mut crate::boundary::ChangeSet::default(),
        [8.5, 64.0, 8.5],
        &staging::BattleCommand::Start,
    )
    .unwrap();
    eprintln!(
        "RANKS tick marching off_slot% mean_err front_wobble heap_foot% heap_horse% densest engaged dead"
    );
    let mut quiet_windows = 0;
    let mut windows = 0;
    let mut window_dead = dead(&state);
    let mut contact_ticks = 0;
    for t in 1..=ticks {
        crate::tick::run_tick(&mut state, &[]);
        let battle = &state.hostile.battle;
        contact_ticks += usize::from(battle.formation.regiments.iter().any(|r| r.gap <= 6.0 && r.enemy.is_some()));
        if t % 100 == 0 {
            let c = census(battle);
            eprintln!(
                "RANKS {t} {} {:.1} {:.2} {:.2} {:.2} {:.2} {} {} {}",
                c.marching,
                100.0 * c.off_slot as f64 / c.marching.max(1) as f64,
                c.slot_error_sum / c.marching.max(1) as f64,
                c.front_wobble,
                100.0 * c.heaped_foot as f64 / c.foot.max(1) as f64,
                100.0 * c.heaped_horse as f64 / c.horse.max(1) as f64,
                c.densest,
                c.engaged_regiments,
                dead(&state)
            );
        }
        if t % 400 == 0 {
            let now = dead(&state);
            if contact_ticks > 300 {
                windows += 1;
                quiet_windows += usize::from(now == window_dead);
            }
            window_dead = now;
            contact_ticks = 0;
        }
    }
    eprintln!("RANKS contact_windows={windows} contact_windows_without_deaths={quiet_windows}");
}
