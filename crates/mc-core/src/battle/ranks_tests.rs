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
                        if other != bi && (o.pos[0] - m.pos[0]).hypot(o.pos[2] - m.pos[2]) < HEAP {
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

/// The faster look of fighters in contact must not spread to the marching ranks: before the lines
/// meet, the retargets per tick stay within the ordinary one-in-ten cadence of the eligible men
/// (front rank, loose order, archers), so far riders and back ranks add no search.
#[test]
fn marching_fighters_keep_the_ordinary_retarget_cadence_before_contact() {
    let mut state = started(1000);
    let (mut retargeted, mut eligible) = (0_u64, 0_u64);
    for _ in 0..300 {
        crate::tick::run_tick(&mut state, &[]);
        let battle = &state.hostile.battle;
        retargeted += battle.work.retargeted;
        eligible += (0..battle.columns.ids.len())
            .filter(|&bi| super::targeting::eligible(&battle.formation, &battle.columns, bi))
            .count() as u64;
    }
    assert!(eligible > 100_000, "{eligible}");
    assert!(
        retargeted * 10 <= eligible * 11,
        "retargeted {retargeted} of {eligible} eligible fighter-ticks"
    );
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
    let mut quiet_windows = 0;
    let mut windows = 0;
    let mut window_dead = dead(&state);
    let mut contact_ticks = 0;
    for t in 1..=ticks {
        crate::tick::run_tick(&mut state, &[]);
        let battle = &state.hostile.battle;
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
                        (slot[0] - m.pos[0]).hypot(slot[2] - m.pos[2])
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
    eprintln!("RANKS contact_windows={windows} contact_windows_without_deaths={quiet_windows}");
}

/// Heaps through the first clash and the pursuit that follows. Run explicitly.
#[test]
#[ignore = "1000 a side, 2200 ticks; run explicitly"]
fn foot_and_horse_do_not_heap_through_contact_and_pursuit() {
    let mut state = started(1000);
    let (mut foot, mut horse) = (0.0_f64, 0.0_f64);
    for t in 1..=2200 {
        crate::tick::run_tick(&mut state, &[]);
        if t % 100 == 0 {
            let c = census(&state.hostile.battle);
            foot = foot.max(100.0 * c.heaped_foot as f64 / c.foot.max(1) as f64);
            horse = horse.max(100.0 * c.heaped_horse as f64 / c.horse.max(1) as f64);
        }
    }
    eprintln!("RANKS max heap foot {foot:.2}% horse {horse:.2}%");
    assert!(foot < 3.0, "foot heap {foot}%");
    assert!(horse < 15.0, "horse heap {horse}%");
    assert!(dead(&state) > 300, "contact must still kill");
}

/// Reaction of melee fighters to a moving enemy, in ticks, on real native ticks:
/// - `idle`: consecutive ticks a front fighter of a regiment in or next to contact has no target
///   although a living enemy stands within his reach (reach 6): the delay before he turns to an
///   enemy that came into reach (target left, stepped aside or fell);
/// - `swing`: ticks from taking a new target to the start of his windup, for the fighters that swing
///   at that man before the target changes (cautious fighters swing rarely by design).
/// Prints mean and p95 of both. BATTLE_UNITS_PER_SIDE, BATTLE_RANKS_TICKS.
#[test]
#[ignore = "diagnostic; run explicitly"]
fn reaction_latency_table() {
    use super::combat::Action;
    let side = std::env::var("BATTLE_UNITS_PER_SIDE").map_or(1000, |v| v.parse().unwrap());
    let ticks = std::env::var("BATTLE_RANKS_TICKS").map_or(2500, |v| v.parse::<usize>().unwrap());
    let mut state = started(side);
    let mut idle_run: Vec<u32> = Vec::new();
    let mut idle: Vec<u32> = Vec::new();
    let mut aimed: Vec<Option<(crate::boundary::EntityId, u64, bool)>> = Vec::new();
    let mut swing: Vec<u32> = Vec::new();
    let mut fresh_targets = 0_usize;
    // Back-and-forth: three-tick displacement vectors that turn more than 90 degrees.
    let mut history: Vec<[[f64; 2]; 3]> = Vec::new();
    let mut last_target: Vec<Option<crate::boundary::EntityId>> = Vec::new();
    // [foot, horse] x [samples, reversals, reversals with a target change, with target lost]
    let mut turns = [[0_u64; 4]; 2];
    // horse reversals by state: [targeted close (<3 blocks), targeted far, sweeping, slot/other]
    let mut horse_state = [0_u64; 4];
    let mut refused_turns = 0_u64;
    // Direct reaction: from the tick a fighter's man leaves his reach (alive, now beyond 6 blocks)
    // to the first tick he FACES (yaw within 30 degrees of) the man or his new target, and to the
    // first tick he STEPS towards him (more than 0.02 blocks along the bearing).
    // watch = (start tick, old target); results (turn latency, step latency), None = not within 40 ticks.
    let mut watch: Vec<Option<(u64, crate::boundary::EntityId, Option<u64>, Option<u64>)>> =
        Vec::new();
    let mut prev_target: Vec<Option<crate::boundary::EntityId>> = Vec::new();
    let mut prev_pos: Vec<[f64; 2]> = Vec::new();
    let mut direct: Vec<(Option<u64>, Option<u64>)> = Vec::new();
    for t in 1..=ticks as u64 {
        crate::tick::run_tick(&mut state, &[]);
        let battle = &state.hostile.battle;
        let columns = &battle.columns;
        let f = &battle.formation;
        if idle_run.len() != columns.ids.len() {
            idle_run = vec![0; columns.ids.len()];
            aimed = vec![None; columns.ids.len()];
        }
        if t % 3 == 0 {
            if history.len() != f.members.len() {
                history = vec![[[f64::NAN; 2]; 3]; f.members.len()];
                last_target = vec![None; f.members.len()];
            }
            for (bi, m) in f.members.iter().enumerate() {
                let now = [m.pos[0], m.pos[2]];
                let h = &mut history[bi];
                let (a, b) = (h[1], h[2]);
                *h = [h[1], h[2], now];
                let target = columns.targets.get(bi).and_then(|x| x.map(|x| x.id));
                let before = std::mem::replace(&mut last_target[bi], target);
                if !m.alive || m.routing || m.wounded || a[0].is_nan() {
                    continue;
                }
                let (v1, v2) = ([b[0] - a[0], b[1] - a[1]], [now[0] - b[0], now[1] - b[1]]);
                let (l1, l2) = (v1[0].hypot(v1[1]), v2[0].hypot(v2[1]));
                if l1 < 0.25 || l2 < 0.25 {
                    continue;
                }
                let class = usize::from(f.regiments[m.regiment].spec.frontage > 1.0);
                turns[class][0] += 1;
                if (v1[0] * v2[0] + v1[1] * v2[1]) / (l1 * l2) < 0.0 {
                    turns[class][1] += 1;
                    turns[class][2] +=
                        u64::from(target.is_some() && before.is_some() && target != before);
                    turns[class][3] += u64::from(before.is_some() && target.is_none());
                    if class == 1 {
                        let close = columns
                            .targets
                            .get(bi)
                            .copied()
                            .flatten()
                            .map(|x| x.distance);
                        let k = match close {
                            Some(d) if d < 3.0 => 0,
                            Some(_) => 1,
                            None if battle
                                .combat
                                .sweeps
                                .get(m.regiment)
                                .is_some_and(Option::is_some) =>
                            {
                                2
                            }
                            None => 3,
                        };
                        horse_state[k] += 1;
                        refused_turns +=
                            u64::from(k == 1 && battle.combat.fighters[bi].charge.refused);
                    }
                }
            }
        }
        if watch.len() != columns.ids.len() {
            watch = vec![None; columns.ids.len()];
            prev_target = vec![None; columns.ids.len()];
            prev_pos = vec![[f64::NAN; 2]; columns.ids.len()];
        }
        for bi in 0..f.members.len().min(columns.ids.len()) {
            let m = &f.members[bi];
            let now = [columns.positions[bi][0], columns.positions[bi][2]];
            let moved = if prev_pos[bi][0].is_nan() {
                [0.0; 2]
            } else {
                [now[0] - prev_pos[bi][0], now[1] - prev_pos[bi][1]]
            };
            prev_pos[bi] = now;
            let current = columns.targets[bi].map(|x| x.id);
            let before = std::mem::replace(&mut prev_target[bi], current);
            let steady = !battle.combat.fighters[bi].charge.refused
                && columns.active[bi]
                && !columns.carried[bi]
                && !columns.ranged[bi]
                && m.alive
                && !m.routing
                && !m.wounded;
            if !steady {
                watch[bi] = None;
                continue;
            }
            let position_of = |id: crate::boundary::EntityId| {
                state
                    .entities
                    .slot_of(id)
                    .filter(|s| state.entities.body(*s).alive)
                    .map(|s| state.entities.pos(s))
            };
            if watch[bi].is_none()
                && let Some(old) = before
                && current != Some(old)
                && let Some(p) = position_of(old)
                && (p[0] - now[0]).hypot(p[2] - now[1]) > 6.0
                && (p[0] - now[0]).hypot(p[2] - now[1]) < 12.0
            {
                watch[bi] = Some((t, old, None, None));
            }
            if let Some((start, old, mut turn, mut step)) = watch[bi] {
                let want = current.and_then(position_of).or_else(|| position_of(old));
                if let Some(p) = want {
                    let (dx, dz) = (p[0] - now[0], p[2] - now[1]);
                    let length = dx.hypot(dz).max(1e-9);
                    let yaw = state.entities.slot_of(columns.ids[bi]).map_or(0.0, |s| {
                        f64::from(state.entities.rotations()[s.0 as usize][0])
                    });
                    let bearing = -mc_physics::mth::atan2(dx, dz) * 180.0 / std::f64::consts::PI;
                    let error = ((yaw - bearing + 540.0).rem_euclid(360.0) - 180.0).abs();
                    if turn.is_none() && error <= 30.0 {
                        turn = Some(t - start);
                    }
                    if step.is_none() && (moved[0] * dx + moved[1] * dz) / length > 0.02 {
                        step = Some(t - start);
                    }
                }
                watch[bi] = Some((start, old, turn, step));
                if (turn.is_some() && step.is_some()) || t - start >= 40 {
                    direct.push((turn, step));
                    watch[bi] = None;
                }
            }
        }
        let mut cells: FxHashMap<(u16, i32, i32), Vec<usize>> = FxHashMap::default();
        for bi in 0..columns.ids.len() {
            if columns.targetable[bi] {
                cells
                    .entry((
                        columns.factions[bi],
                        (columns.positions[bi][0] / 6.0).floor() as i32,
                        (columns.positions[bi][2] / 6.0).floor() as i32,
                    ))
                    .or_default()
                    .push(bi);
            }
        }
        for bi in 0..f.members.len().min(columns.ids.len()) {
            let m = &f.members[bi];
            let r = &f.regiments[m.regiment];
            let melee_front = columns.active[bi]
                && !columns.carried[bi]
                && !columns.ranged[bi]
                && !battle.combat.fighters[bi].charge.refused
                && m.alive
                && !m.routing
                && !m.wounded
                && m.rank == 0
                && r.enemy.is_some()
                && r.gap <= 12.0;
            if !melee_front {
                if idle_run[bi] > 0 {
                    idle.push(idle_run[bi]);
                    idle_run[bi] = 0;
                }
                aimed[bi] = None;
                continue;
            }
            let p = columns.positions[bi];
            let (cx, cz) = ((p[0] / 6.0).floor() as i32, (p[2] / 6.0).floor() as i32);
            let enemy_in_reach = (-1..=1).any(|dx| {
                (-1..=1).any(|dz| {
                    [1_u16, 2]
                        .into_iter()
                        .filter(|&fac| fac != columns.factions[bi])
                        .any(|fac| {
                            cells.get(&(fac, cx + dx, cz + dz)).is_some_and(|list| {
                                list.iter().any(|&o| {
                                    let q = columns.positions[o];
                                    (q[0] - p[0]).hypot(q[2] - p[2]) <= 6.0
                                        && (q[1] - p[1]).abs() < 2.0
                                })
                            })
                        })
                })
            });
            if enemy_in_reach && columns.targets[bi].is_none() {
                idle_run[bi] += 1;
            } else if idle_run[bi] > 0 {
                idle.push(idle_run[bi]);
                idle_run[bi] = 0;
            }
            let current = columns.targets[bi].map(|x| x.id);
            match (current, aimed[bi]) {
                (Some(id), Some((old, _, _))) if id == old => {}
                (Some(id), _) => {
                    aimed[bi] = Some((id, t, false));
                    fresh_targets += 1;
                }
                (None, _) => aimed[bi] = None,
            }
            if let Some((id, since, done)) = aimed[bi]
                && !done
                && let Action::Windup { target, start, .. } = battle.combat.fighters[bi].action
                && target == id
                && start >= since
            {
                swing.push((start - since) as u32);
                aimed[bi] = Some((id, since, true));
            }
        }
    }
    let stat = |v: &mut Vec<u32>| {
        v.sort_unstable();
        let mean = v.iter().map(|&x| f64::from(x)).sum::<f64>() / v.len().max(1) as f64;
        let p95 = v
            .get(((v.len().max(1) - 1) as f64 * 0.95) as usize)
            .copied()
            .unwrap_or(0);
        (
            v.len(),
            mean,
            p95,
            v.get((v.len().max(1) - 1) / 2).copied().unwrap_or(0),
        )
    };
    {
        let stat = |v: &mut Vec<u64>| {
            v.sort_unstable();
            let mean = v.iter().sum::<u64>() as f64 / v.len().max(1) as f64;
            (
                v.len(),
                mean,
                v.get(((v.len().max(1) - 1) as f64 * 0.95) as usize)
                    .copied()
                    .unwrap_or(0),
                v.get((v.len().max(1) - 1) / 2).copied().unwrap_or(0),
            )
        };
        let mut turns_l: Vec<u64> = direct.iter().filter_map(|d| d.0).collect();
        let mut steps_l: Vec<u64> = direct.iter().filter_map(|d| d.1).collect();
        let mut turns_c: Vec<u64> = direct.iter().map(|d| d.0.unwrap_or(40)).collect();
        let mut steps_c: Vec<u64> = direct.iter().map(|d| d.1.unwrap_or(40)).collect();
        let (n, mean, p95, p50) = stat(&mut turns_c);
        eprintln!("REACT censored(40) turn_to_him events={n} mean {mean:.2} p50 {p50} p95 {p95}");
        let (n, mean, p95, p50) = stat(&mut steps_c);
        eprintln!(
            "REACT censored(40) step_toward_him events={n} mean {mean:.2} p50 {p50} p95 {p95}"
        );
        let (n, mean, p95, p50) = stat(&mut turns_l);
        eprintln!(
            "REACT direct man-left-reach events={} turn_to_him: reacted {n} mean {mean:.2} p50 {p50} p95 {p95} not within 40 ticks {}",
            direct.len(),
            direct.len() - n
        );
        let (n, mean, p95, p50) = stat(&mut steps_l);
        eprintln!(
            "REACT direct man-left-reach events={} step_toward_him: reacted {n} mean {mean:.2} p50 {p50} p95 {p95} not within 40 ticks {}",
            direct.len(),
            direct.len() - n
        );
    }
    let (n, mean, p95, p50) = stat(&mut idle);
    eprintln!(
        "REACT idle_with_enemy_in_reach runs={n} mean_ticks={mean:.2} p50_ticks={p50} p95_ticks={p95}"
    );
    for (name, c) in ["foot", "horse"].into_iter().zip(turns) {
        eprintln!(
            "REACT turns {name}: moving samples (3-tick vectors >0.25) {} reversals(>90deg) {} ({:.2}%) with target change {} target lost {}",
            c[0],
            c[1],
            100.0 * c[1] as f64 / c[0].max(1) as f64,
            c[2],
            c[3]
        );
    }
    eprintln!(
        "REACT horse reversals by state: targeted<3 {} targeted>=3 {} sweeping {} slot/other {} (of targeted>=3, charge refused: {})",
        horse_state[0], horse_state[1], horse_state[2], horse_state[3], refused_turns
    );
    let (n, mean, p95, p50) = stat(&mut swing);
    eprintln!(
        "REACT new_target_to_windup swings={n} of {fresh_targets} new targets mean_ticks={mean:.2} p50_ticks={p50} p95_ticks={p95}"
    );
}
