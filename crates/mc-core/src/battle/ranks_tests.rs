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
    /// The mounted share of `marching`, `off_slot` and `slot_error_sum`.
    pub horse_marching: usize,
    pub horse_off_slot: usize,
    pub horse_error_sum: f64,
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
            .entry([
                (m.pos[0] / 2.0).floor() as i32,
                (m.pos[2] / 2.0).floor() as i32,
            ])
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
                            && (o.pos[0] - m.pos[0]).powi(2) + (o.pos[2] - m.pos[2]).powi(2)
                                < HEAP * HEAP
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
            let error = ((slot[0] - m.pos[0]).powi(2) + (slot[2] - m.pos[2]).powi(2)).sqrt();
            c.slot_error_sum += error;
            c.off_slot += usize::from(error > OFF_SLOT);
            if r.spec.frontage > 1.0 {
                c.horse_marching += 1;
                c.horse_off_slot += usize::from(error > OFF_SLOT);
                c.horse_error_sum += error;
            }
            if m.rank == 0 && r.spec.frontage <= 1.0 {
                front.push(
                    (m.pos[0] - slot[0]) * r.spec.facing[0]
                        + (m.pos[2] - slot[2]) * r.spec.facing[1],
                );
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

fn started(side: usize) -> crate::tick::TickState {
    let mut state = staged_flat_fixture(side);
    staging::execute(
        &mut state,
        &mut crate::boundary::ChangeSet::default(),
        [8.5, 64.0, 8.5],
        &staging::BattleCommand::Start,
    )
    .unwrap();
    state
}

/// Arrows land on the foot long before contact (vanilla hit knockback threw every
/// struck soldier up to three blocks from his slot, so the whole front regiment
/// stood as a scatter, not ranks and files). Every soldier of a regiment still
/// marching must hold his slot.
#[test]
fn marching_soldiers_hold_rank_and_file_under_arrow_fire() {
    let mut state = started(1000);
    for _ in 0..300 {
        crate::tick::run_tick(&mut state, &[]);
    }
    let c = census(&state.hostile.battle);
    assert!(c.marching > 1900, "{c:?}");
    assert_eq!(c.off_slot, 0, "{c:?}");
    assert!(c.slot_error_sum / (c.marching as f64) < 0.15, "{c:?}");
    assert!(c.front_wobble < 0.1, "{c:?}");
    let (dead, _, _) = super::tests::native_battle_losses(&state);
    assert!(dead[0] + dead[1] > 0, "the arrows must still kill");
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
    let mut state = started(side);
    eprintln!(
        "RANKS tick marching foot_off% foot_err horse_off% horse_err front_wobble heap_foot% heap_horse% densest engaged dead"
    );
    let mut max_overflow = 0_u64;
    let mut quiet_windows = 0;
    let mut windows = 0;
    let mut window_dead = dead(&state);
    let mut contact_ticks = 0;
    for t in 1..=ticks {
        crate::tick::run_tick(&mut state, &[]);
        let battle = &state.hostile.battle;
        max_overflow = max_overflow.max(battle.formation.work.separation_overflow);
        contact_ticks += usize::from(
            battle
                .formation
                .regiments
                .iter()
                .any(|r| r.gap <= 6.0 && r.enemy.is_some()),
        );
        if t % 100 == 0 {
            let c = census(battle);
            eprintln!(
                "RANKS {t} {} {:.1} {:.2} {:.1} {:.2} {:.2} {:.2} {:.2} {} {} {}",
                c.marching,
                100.0 * (c.off_slot - c.horse_off_slot) as f64
                    / (c.marching - c.horse_marching).max(1) as f64,
                (c.slot_error_sum - c.horse_error_sum)
                    / (c.marching - c.horse_marching).max(1) as f64,
                100.0 * c.horse_off_slot as f64 / c.horse_marching.max(1) as f64,
                c.horse_error_sum / c.horse_marching.max(1) as f64,
                c.front_wobble,
                100.0 * c.heaped_foot as f64 / c.foot.max(1) as f64,
                100.0 * c.heaped_horse as f64 / c.horse.max(1) as f64,
                c.densest,
                c.engaged_regiments,
                dead(&state)
            );
        }
        if let Some(dir) = std::env::var_os("BATTLE_RANKS_DUMP")
            && [400, 560, 800, 1400].contains(&t)
        {
            let mut text = String::new();
            for m in battle.formation.members.iter().filter(|m| m.alive) {
                let r = &battle.formation.regiments[m.regiment];
                let kind = if r.spec.frontage > 1.0 {
                    "horse"
                } else {
                    "foot"
                };
                text += &format!(
                    "{} {kind} {:.2} {:.2}\n",
                    r.spec.faction, m.pos[0], m.pos[2]
                );
            }
            std::fs::write(std::path::Path::new(&dir).join(format!("t{t}.csv")), text).unwrap();
        }
        if t % 200 == 0 && t <= 800 {
            for r in &battle.formation.regiments {
                let errors: Vec<f64> = (r.start..r.start + r.len)
                    .filter(|&bi| battle.formation.members[bi].alive)
                    .map(|bi| {
                        let m = &battle.formation.members[bi];
                        let slot = battle.formation.heading_target(bi);
                        ((slot[0] - m.pos[0]).powi(2) + (slot[2] - m.pos[2]).powi(2)).sqrt()
                    })
                    .collect();
                eprintln!(
                    "REGIMENT t={t} id={} files={} frontage={} order={:?} gap={:.1} speed={:.3} mean_err={:.2} max_err={:.2}",
                    r.spec.id,
                    r.spec.files,
                    r.spec.frontage,
                    r.spec.order,
                    r.gap,
                    r.speed,
                    errors.iter().sum::<f64>() / errors.len().max(1) as f64,
                    errors.iter().copied().fold(0.0, f64::max)
                );
            }
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
    eprintln!(
        "RANKS contact_windows={windows} contact_windows_without_deaths={quiet_windows} separation_overflow_max={max_overflow}"
    );
}

#[test]
fn a_dense_heap_reports_dropped_separation_candidates() {
    // The separation grid keeps at most CELL_CAP bodies per sub-cell; the battle
    // work counters must report how many were dropped, or a heap measurement
    // cannot say whether neighbours were missing. One `run_tick` prepares the
    // columns, then this drives `steering::step` directly: it proves the counter
    // reaches `Work`, NOT that the production tick still calls the pass (a
    // removed call leaves this green; see HANDOFF-5 H4).
    let mut state = started(60);
    crate::tick::run_tick(&mut state, &[]);
    let mut battle = std::mem::take(&mut state.hostile.battle);
    for member in &mut battle.formation.members {
        if member.alive {
            member.pos = [8.0, 64.0, 8.0];
        }
    }
    let frame = crate::battle::Frame {
        entities: &state.entities,
        registry: &state.registry,
        world: &state.world,
        tick: state.tick,
    };
    crate::battle::steering::step(&frame, &mut battle, &mut crate::battle::Output::default());
    assert!(
        battle.formation.neighbours.overflow > 0,
        "one sub-cell heap must drop candidates past the cap"
    );
    assert_eq!(
        battle.formation.work.separation_overflow, battle.formation.neighbours.overflow,
        "the dropped-candidate count must reach the battle work counters"
    );
}

#[test]
fn the_production_tick_reports_the_separation_grids_dropped_candidates() {
    // HANDOFF-5 H4: `a_dense_heap_reports_dropped_separation_candidates` calls
    // `steering::step` itself, so a removed mirror call left it green. This one
    // moves the soldiers in the world and runs only `run_tick`: the production
    // battle tick must keep the grid's dropped-candidate count in `Work`.
    let mut state = started(60);
    crate::tick::run_tick(&mut state, &[]);
    let heap = [8.0, 64.0, 8.0];
    let slots: Vec<_> = state
        .hostile
        .battle
        .columns
        .slots
        .iter()
        .flatten()
        .copied()
        .collect();
    assert!(
        slots.len() > 64,
        "the staged battle must present its soldiers to the columns, got {}",
        slots.len()
    );
    for slot in slots {
        state.entities.set_pos(slot, heap);
    }
    crate::tick::run_tick(&mut state, &[]);
    let battle = &state.hostile.battle;
    assert!(
        battle.formation.neighbours.overflow > 0,
        "one sub-cell heap must drop candidates past the cap, dropped={}",
        battle.formation.neighbours.overflow
    );
    assert_eq!(
        battle.formation.work.separation_overflow, battle.formation.neighbours.overflow,
        "the production tick must mirror the dropped-candidate count into the battle work counters"
    );
}

/// Heaps through the first clash and the pursuit that follows. Run explicitly.
#[test]
#[ignore = "1000 a side through contact and pursuit, at most 6000 ticks; run explicitly"]
fn foot_and_horse_do_not_heap_through_contact_and_pursuit() {
    let mut state = started(1000);
    let (mut foot, mut horse) = (0.0_f64, 0.0_f64);
    let mut overflow = 0_u64;
    let mut pursuit_ticks = 0;
    let mut melee_kills = 0;
    let mut wounds = 0;
    // 9edae78e7 intentionally lengthened contact by reducing press lethality.
    // The old 2200-tick cutoff could stop before pursuit. Preserve its >300
    // casualty floor and both heap limits, but allow the deliberately longer
    // battle to reach that exposure and at least 200 actual pursuit ticks.
    for t in 1..=6000 {
        crate::tick::run_tick(&mut state, &[]);
        let battle = &state.hostile.battle;
        melee_kills += battle.combat.work.killed;
        wounds += battle.combat.work.wounded;
        if battle.formation.regiments.iter().any(|regiment| {
            regiment.spec.order == formation::Order::Pursue
                && regiment.enemy_routing
                && regiment.live_count > 0
        }) {
            pursuit_ticks += 1;
        }
        overflow = overflow.max(state.hostile.battle.formation.work.separation_overflow);
        if t % 100 == 0 {
            let c = census(&state.hostile.battle);
            foot = foot.max(100.0 * c.heaped_foot as f64 / c.foot.max(1) as f64);
            horse = horse.max(100.0 * c.heaped_horse as f64 / c.horse.max(1) as f64);
        }
        if t >= 2200 && pursuit_ticks >= 200 && dead(&state) > 300 {
            break;
        }
    }
    let (deaths, removed, native_wounds) = super::tests::native_battle_losses(&state);
    eprintln!(
        "RANKS max heap foot {foot:.2}% horse {horse:.2}% separation_overflow_max={overflow} deaths={deaths:?} removed={removed:?} native_wounds={native_wounds} melee_kills={melee_kills} wounds={wounds} pursuit_ticks={pursuit_ticks}",
    );
    assert!(foot < 3.0, "foot heap {foot}%");
    assert!(horse < 15.0, "horse heap {horse}%");
    assert!(pursuit_ticks >= 200, "pursuit_ticks={pursuit_ticks}");
    assert!(
        melee_kills > 0,
        "contact must still kill, melee_kills={melee_kills}"
    );
    assert!(dead(&state) > 300, "contact must still kill");
}
