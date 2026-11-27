//! In-process battle tick diagnostic over the operator `battle setup` scene.
//!
//! Drives the real dimension-coordinated `run_tick` and then the tick loop's
//! WAL entry build (`journal_entries_for_tick`, the `journal_build` phase), so
//! the per-phase numbers are the dashboard's `TickTimes` fields without pacing,
//! edge fanout, WAL I/O or checkpoint capture. One window with the staged
//! armies paused, then one with the battle running. Build the release test
//! binary in the ordinary lane and run it in the perf lane; receipts and exact
//! commands live in `.analysis/codex-logs/battle-tick-perf/`.

use std::sync::Arc;

use crate::battle::staging::{BattleCommand, execute};
use crate::boundary::{ChangeSet, PlayerInput, TickTimes};
use crate::projectiles::PostTimes;
use crate::registry::CoreRegistry;
use crate::tick::{TickState, run_tick};
use mc_domain::ConnectionId;

const PHASES: [&str; 13] = [
    "input",
    "chunks",
    "entity",
    "merge",
    "block",
    "commit",
    "snapshot",
    "views",
    "publish",
    "plugins",
    "coordination",
    "journal_build",
    "total",
];

fn phase_values(times: &TickTimes, journal_us: u64, total_us: u64) -> [u64; 13] {
    [
        times.input_us,
        times.chunks_us,
        times.entity_us,
        times.merge_us,
        times.block_us,
        times.commit_us,
        times.snapshot_us,
        times.views_us,
        times.publish_us,
        times.plugins_us,
        times.coordination_us,
        journal_us,
        total_us,
    ]
}

fn env_or(name: &str, default: usize) -> usize {
    std::env::var(name).map_or(default, |value| value.parse().unwrap())
}

fn summarize(label: &str, rows: &[[u64; 13]], touched: &[usize], entries: &[usize]) {
    let mut line = format!("BATTLE_TICK window={label} ticks={}", rows.len());
    for (index, phase) in PHASES.iter().enumerate() {
        let mut values: Vec<u64> = rows.iter().map(|row| row[index]).collect();
        values.sort_unstable();
        let mean = values.iter().sum::<u64>() as f64 / values.len() as f64 / 1000.0;
        let p50 = values[(values.len() - 1) / 2] as f64 / 1000.0;
        let p95 = values[((values.len() - 1) * 95) / 100] as f64 / 1000.0;
        line.push_str(&format!(" {phase}_ms={mean:.3}/{p50:.3}/{p95:.3}"));
    }
    let mean = |values: &[usize]| values.iter().sum::<usize>() as f64 / values.len() as f64;
    line.push_str(&format!(
        " journal_touched_mean={:.0} journal_entries_mean={:.0}",
        mean(touched),
        mean(entries)
    ));
    eprintln!("{line}");
}

/// Mean/p50/p95/max milliseconds of `values` (microseconds).
fn spread(mut values: Vec<u64>) -> String {
    if values.is_empty() {
        return "-".into();
    }
    values.sort_unstable();
    let ms = |value: u64| value as f64 / 1000.0;
    let mean = values.iter().sum::<u64>() as f64 / values.len() as f64 / 1000.0;
    format!(
        "{mean:.3}/{:.3}/{:.3}/{:.3}",
        ms(values[(values.len() - 1) / 2]),
        ms(values[((values.len() - 1) * 95) / 100]),
        ms(values[values.len() - 1]),
    )
}

/// The serial `post` stage over the window and over its flight ticks (at
/// least one projectile not settled in the ground), each as
/// mean/p50/p95/max ms, with the whole tick on the same flight ticks.
fn summarize_post(label: &str, rows: &[[u64; 13]], posts: &[PostTimes]) {
    let flight: Vec<usize> = (0..posts.len())
        .filter(|&index| posts[index].shots > posts[index].grounded)
        .collect();
    let pick = |field: fn(&PostTimes) -> u64| -> Vec<u64> {
        flight.iter().map(|&index| field(&posts[index])).collect()
    };
    let mean_ms =
        |values: Vec<u64>| values.iter().sum::<u64>() as f64 / values.len().max(1) as f64 / 1000.0;
    let flying: Vec<u64> = flight
        .iter()
        .map(|&index| u64::from(posts[index].shots - posts[index].grounded))
        .collect();
    eprintln!(
        "BATTLE_TICK post window={label} ticks={} post_ms={} flight_ticks={} flight_post_ms={} flight_total_ms={} flight_entity_ms={} flight_stage_mean_ms=pickup:{:.3},hostile:{:.3},weapons:{:.3},hittables:{:.3},candidates:{:.3},shots:{:.3} flying_mean={:.0} flying_max={}",
        posts.len(),
        spread(posts.iter().map(|post| post.total_us).collect()),
        flight.len(),
        spread(pick(|post| post.total_us)),
        spread(flight.iter().map(|&index| rows[index][12]).collect()),
        spread(flight.iter().map(|&index| rows[index][2]).collect()),
        mean_ms(pick(|post| post.pickup_us)),
        mean_ms(pick(|post| post.hostile_us)),
        mean_ms(pick(|post| post.weapons_us)),
        mean_ms(pick(|post| post.hittables_us)),
        mean_ms(pick(|post| post.candidates_us)),
        mean_ms(pick(|post| post.shots_us)),
        flying.iter().sum::<u64>() as f64 / flying.len().max(1) as f64,
        flying.iter().max().copied().unwrap_or(0),
    );
}

/// `BATTLE_UNITS_PER_SIDE` (default 10,000, the `field-10k` preset),
/// `BATTLE_PAUSED_TICKS` (default 300), `BATTLE_RUNNING_TICKS` (default
/// 600) and `BATTLE_PLAYERS` (default 1: the ladder's recorder, a survival
/// player at world spawn, joined before setup; protected so the scene stays
/// fixed). Each phase prints mean/p50/p95 milliseconds per window.
/// `KRASNOKAMEN_TRACE_ROW_MUT=1` adds `row_mut` caller attribution (it slows
/// `row_mut`, so never combine it with timing runs). Correctness switches
/// (never timing runs: they warm every row between ticks): `BATTLE_WAL_CRC=1`
/// prints each window's CRC-32 and size of its WAL payload (every entry's
/// JSON line, as `JournalWriter::append` frames it) to compare trees byte for
/// byte; `BATTLE_FENCE=1` asserts the touched-set differential fence after
/// every batch.
#[test]
#[ignore = "release battle tick diagnostic; run in the perf lane"]
fn battle_tick_phase_micro() {
    let side = env_or("BATTLE_UNITS_PER_SIDE", 10_000);
    let paused_ticks = env_or("BATTLE_PAUSED_TICKS", 300);
    let running_ticks = env_or("BATTLE_RUNNING_TICKS", 600);
    let players = env_or("BATTLE_PLAYERS", 1);
    let wal_crc = std::env::var_os("BATTLE_WAL_CRC").is_some();
    let fence = std::env::var_os("BATTLE_FENCE").is_some();
    let registry = Arc::new(CoreRegistry::load(None).unwrap());
    let world = crate::horde_bench::flat_world(&registry);
    // The ladder's server.toml: view and simulation distance 12.
    let mut state = TickState::new(registry, world, 12, 512);
    let mut changes = ChangeSet::default();
    let joins: Vec<_> = (1..=players as u64)
        .map(|connection| PlayerInput::Join {
            connection: ConnectionId(connection),
            uuid: uuid::Uuid::from_u128(u128::from(connection)),
            name: format!("recorder{connection}"),
            permission_level: 0,
        })
        .collect();
    run_tick(&mut state, &joins);
    for connection in 1..=players as u64 {
        let id = state
            .connections
            .entity_of(ConnectionId(connection))
            .expect("recorder joined");
        let slot = state.entities.slot_of(id).expect("recorder slot");
        state.entities.row_mut(slot).protected_until = u64::MAX;
    }
    let origin = [8.5, 64.0, 8.5];
    execute(
        &mut state,
        &mut changes,
        origin,
        &BattleCommand::Setup {
            seed: 42,
            count: side,
        },
    )
    .unwrap();
    let desired = state.desired_chunks();
    state.world.ensure_blocking(&desired);
    state.refresh_active_chunks(state.tick);
    let rows = state.entities.slots().count();
    for _ in 0..20 {
        let (output, _, _) = run_tick(&mut state, &[]);
        crate::core::journal_entries_for_tick(&mut state, &output).unwrap();
    }
    eprintln!("BATTLE_TICK setup side={side} rows={rows}");
    let mut moving_start = Vec::new();
    for running in [false, true] {
        let label = if running { "running" } else { "paused" };
        if running {
            execute(&mut state, &mut changes, origin, &BattleCommand::Start).unwrap();
            moving_start = state
                .entities
                .slots()
                .map(|slot| state.entities.pos(slot))
                .collect();
        }
        let ticks = if running { running_ticks } else { paused_ticks };
        state.entities.take_touch_sites();
        let mut samples = Vec::with_capacity(ticks);
        let mut touched = Vec::with_capacity(ticks);
        let mut entries = Vec::with_capacity(ticks);
        let mut wal = (crc32fast::Hasher::new(), 0_usize);
        let mut posts = Vec::with_capacity(ticks);
        let mut alive = None;
        for _ in 0..ticks {
            let start = std::time::Instant::now();
            let (output, _drains, times) = run_tick(&mut state, &[]);
            let run_time = start.elapsed();
            let built = crate::core::journal_entries_for_tick(&mut state, &output).unwrap();
            let total = start.elapsed();
            touched.push(
                crate::journal_touched::LAST_TOUCHED.load(std::sync::atomic::Ordering::Relaxed),
            );
            entries.push(built.len());
            if wal_crc {
                for entry in &built {
                    let line = serde_json::to_string(entry).expect("journal entry encodes");
                    wal.0.update(line.as_bytes());
                    wal.0.update(b"\n");
                    wal.1 += line.len() + 1;
                }
            }
            if fence {
                crate::journal_touched::assert_baselines_complete(&state, output.tick);
            }
            samples.push(phase_values(
                &times,
                (total - run_time).as_micros() as u64,
                total.as_micros() as u64,
            ));
            posts.push(state.projectiles.post);
            alive = times.battle.map(|counts| counts.alive.clone());
        }
        summarize(label, &samples, &touched, &entries);
        summarize_post(label, &samples, &posts);
        eprintln!("BATTLE_TICK end window={label} alive={alive:?}");
        if let Some(path) = std::env::var_os("BATTLE_TICK_CSV") {
            use std::io::Write;
            let mut csv = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .expect("BATTLE_TICK_CSV opens");
            for (index, (row, post)) in samples.iter().zip(&posts).enumerate() {
                writeln!(
                    csv,
                    "{label},{index},{},{},{},{},{},{},{},{},{},{},{}",
                    row[12],
                    row[2],
                    post.total_us,
                    post.pickup_us,
                    post.hostile_us,
                    post.weapons_us,
                    post.hittables_us,
                    post.candidates_us,
                    post.shots_us,
                    post.shots,
                    post.grounded,
                )
                .expect("BATTLE_TICK_CSV writes");
            }
        }
        if wal_crc {
            eprintln!(
                "BATTLE_TICK wal window={label} crc32={:08x} bytes={}",
                wal.0.finalize(),
                wal.1
            );
        }
        if fence {
            eprintln!("BATTLE_TICK fence window={label} held=true");
        }
        // Observation only: the accumulated fall distance of mounted riders.
        let riders = state.entities.slots().filter(|slot| {
            state
                .entities
                .row(*slot)
                .entity_life
                .mount
                .vehicle
                .is_some()
        });
        let (count, max_fall) = riders.fold((0, 0.0_f64), |(count, max), slot| {
            let fall = state.entities.row(slot).controls.fall_distance;
            (count + 1, max.max(fall))
        });
        eprintln!(
            "BATTLE_TICK riders window={label} mounted={count} max_fall_distance={max_fall:.2}"
        );
        let mut sites: Vec<_> = state.entities.take_touch_sites().into_iter().collect();
        sites.sort_by_key(|(_, counts)| std::cmp::Reverse(counts[0]));
        for ((file, line), [calls, villagers, first]) in sites.into_iter().take(12) {
            eprintln!(
                "BATTLE_TICK row_mut window={label} site={file}:{line} calls_per_tick={} villager_calls_per_tick={} first_touch_per_tick={}",
                calls / ticks,
                villagers / ticks,
                first / ticks,
            );
        }
    }
    // Scene fence: the staged population exists and the running window moved
    // units, so the running numbers are not an idle scene.
    let moved = state
        .entities
        .slots()
        .zip(&moving_start)
        .filter(|(slot, start)| state.entities.pos(*slot) != **start)
        .count();
    eprintln!("BATTLE_TICK fence rows={rows} moved={moved}");
    assert!(rows >= 2 * side, "staged rows: {rows}");
    assert!(moved > 0, "the running window must move battle units");
}
