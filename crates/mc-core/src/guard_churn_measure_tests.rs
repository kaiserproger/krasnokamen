//! TEMPORARY measurement (D20 `Guard churn`, not to be committed): WAL bytes and
//! journal time of a battle run that reaches a mass rout, attributed per fighter
//! to `action.until`-only rewrites.
//!
//! Drives the real `run_tick` + `core::journal_entries_for_tick` on the same
//! fixture `battle_tick_micro_tests` uses (`BattleCommand::Setup`), so the
//! records are production `Battle`/`BattlePatch` records.
//!
//! Env: `GUARD_SIDE` (default 100), `GUARD_TICKS` (default 6000).

use std::sync::Arc;

use serde_json::Value;

use crate::battle::staging::{BattleCommand, execute};
use crate::boundary::ChangeSet;
use crate::journal::JournalEntry;
use crate::registry::CoreRegistry;
use crate::tick::{TickState, run_tick};

fn env_or(name: &str, default: usize) -> usize {
    std::env::var(name).map_or(default, |value| value.parse().unwrap())
}

/// Report line: stderr for a failing run, and always into `GUARD_REPORT`.
fn emit(text: String) {
    eprintln!("{text}");
    if let Some(path) = std::env::var_os("GUARD_REPORT") {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .expect("report opens");
        writeln!(file, "{text}").expect("report writes");
    }
}

fn record_bytes(entry: &JournalEntry) -> usize {
    serde_json::to_string(entry).expect("journal entry encodes").len() + 1
}

fn battle_bytes(entry: &JournalEntry) -> usize {
    match entry {
        JournalEntry::Dimension { entries, .. } => entries.iter().map(battle_bytes).sum(),
        JournalEntry::Battle { .. } | JournalEntry::BattlePatch { .. } => record_bytes(entry),
        _ => 0,
    }
}

/// Leaf JSON pointers where `new` differs from `old` (`""` for the root).
fn changed_paths(old: &Value, new: &Value, prefix: String, out: &mut Vec<String>) {
    if old == new {
        return;
    }
    match (old, new) {
        (Value::Object(a), Value::Object(b)) => {
            for (key, value) in b {
                let path = if prefix.is_empty() {
                    format!("/{key}")
                } else {
                    format!("{prefix}/{key}")
                };
                match a.get(key) {
                    Some(previous) => changed_paths(previous, value, path, out),
                    None => out.push(path),
                }
            }
            for key in a.keys() {
                if !b.contains_key(key) {
                    let path = if prefix.is_empty() {
                        format!("/{key}")
                    } else {
                        format!("{prefix}/{key}")
                    };
                    out.push(path);
                }
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            for (index, value) in b.iter().enumerate() {
                let path = format!("{prefix}/{index}");
                match a.get(index) {
                    Some(previous) => changed_paths(previous, value, path, out),
                    None => out.push(path),
                }
            }
            if a.len() > b.len() {
                out.push(format!("{prefix}/len"));
            }
        }
        _ => out.push(prefix),
    }
}

/// Only `action.until` moved: a routing guard re-stamp, no decision taken.
fn guard_only(old: Option<&Value>, new: &Value) -> bool {
    let Some(old) = old else {
        return false;
    };
    let mut paths = Vec::new();
    changed_paths(old, new, String::new(), &mut paths);
    // `Action` is an externally tagged enum: `{"Guard":{"until":N}}`.
    !paths.is_empty() && paths.iter().all(|path| path == "/action/Guard/until")
}

fn fighters_of(entry: &JournalEntry, out: &mut Vec<JournalEntry>) {
    match entry {
        JournalEntry::Dimension { entries, .. } => {
            for entry in entries {
                fighters_of(entry, out);
            }
        }
        _ => out.push(entry.clone()),
    }
}

struct Window {
    label: String,
    ticks: usize,
    battle_bytes: usize,
    guard_bytes: usize,
    fighters: usize,
    guard: usize,
    routing: f64,
    broken: f64,
    journal_us: u128,
    total_us: u128,
    wal_bytes: usize,
}

impl Window {
    fn new(label: &str) -> Self {
        Self {
            label: label.into(),
            ticks: 0,
            battle_bytes: 0,
            guard_bytes: 0,
            fighters: 0,
            guard: 0,
            routing: 0.0,
            broken: 0.0,
            journal_us: 0,
            total_us: 0,
            wal_bytes: 0,
        }
    }

    fn report(&self) {
        let ticks = self.ticks.max(1) as f64;
        emit(format!(
            "GUARD window={} ticks={} battle_B_per_tick={:.0} guard_only_B_per_tick={:.0} guard_share_of_battle={:.1}% fighters_per_tick={:.1} guard_only_per_tick={:.1} routing_per_tick={:.1} broken_per_tick={:.2} journal_us_per_tick={:.0} total_us_per_tick={:.0} wal_B_per_tick={:.0}",
            self.label,
            self.ticks,
            self.battle_bytes as f64 / ticks,
            self.guard_bytes as f64 / ticks,
            100.0 * self.guard_bytes as f64 / self.battle_bytes.max(1) as f64,
            self.fighters as f64 / ticks,
            self.guard as f64 / ticks,
            self.routing / ticks,
            self.broken / ticks,
            self.journal_us as f64 / ticks,
            self.total_us as f64 / ticks,
            self.wal_bytes as f64 / ticks,
        ));
    }
}

#[test]
fn guard_churn_measure() {
    let side = env_or("GUARD_SIDE", 100);
    let ticks = env_or("GUARD_TICKS", 6_000);
    // 0 keeps the natural run; a tick forces a deterministic mass rout by
    // breaking every regiment of side 2 as `cavalry_tests` does.
    let break_at = env_or("GUARD_BREAK_AT", 0);
    let registry = Arc::new(CoreRegistry::load(None).unwrap());
    let world = crate::horde_bench::flat_world(&registry);
    let mut state = TickState::new(registry, world, 12, 512);
    let mut changes = ChangeSet::default();
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
    for _ in 0..20 {
        let (output, _, _) = run_tick(&mut state, &[]);
        crate::core::journal_entries_for_tick(&mut state, &output).unwrap();
    }
    execute(&mut state, &mut changes, origin, &BattleCommand::Start).unwrap();
    emit(format!(
        "GUARD scene side={side} rows={} ticks={ticks} break_at={break_at}",
        state.entities.slots().count()
    ));

    let mut mirror: Vec<Value> = Vec::new();
    let mut windows = [
        Window::new("all"),
        Window::new("pre-rout"),
        Window::new("rout"),
        Window::new("post-rout"),
    ];
    for _ in 0..ticks {
        if break_at != 0 && state.hostile.battle.elapsed_ticks == break_at as u64 {
            for regiment in &mut state.hostile.battle.formation.regiments {
                if regiment.spec.faction == 2 {
                    regiment.morale.broken = true;
                }
            }
            emit(format!("GUARD forced rout at tick={break_at}"));
        }
        let start = std::time::Instant::now();
        let (output, _, _times) = run_tick(&mut state, &[]);
        let run_time = start.elapsed();
        let built = crate::core::journal_entries_for_tick(&mut state, &output).unwrap();
        let total = start.elapsed();
        let journal_us = (total - run_time).as_micros();

        // Scene gauge: routing men and broken regiments after the tick.
        let battle = &state.hostile.battle;
        // `context_at` is the columns' view; routing is `m.routing || !alive || wounded`.
        let routing = battle
            .formation
            .members
            .iter()
            .filter(|member| member.routing || !member.alive || member.wounded)
            .count();
        let broken = battle
            .formation
            .regiments
            .iter()
            .filter(|regiment| regiment.morale.broken)
            .count();
        let running = battle.running;

        let mut flat = Vec::new();
        for entry in &built {
            fighters_of(entry, &mut flat);
        }
        let wal_bytes: usize = built.iter().map(record_bytes).sum();
        let battle_total: usize = built.iter().map(battle_bytes).sum();
        let mut fighters = 0_usize;
        let mut guard = 0_usize;
        let mut guard_bytes = 0_usize;
        for entry in &flat {
            match entry {
                JournalEntry::Battle { value } => {
                    let image: Value = serde_json::from_str(value).unwrap();
                    let list = image["battle"]["combat"]["fighters"]
                        .as_array()
                        .expect("full image fighters")
                        .clone();
                    mirror = list;
                }
                JournalEntry::BattlePatch { value } => {
                    for patch in value["lists"].as_array().into_iter().flatten() {
                        if patch["list"] != "fighters" {
                            continue;
                        }
                        let len = patch["len"].as_u64().unwrap() as usize;
                        mirror.resize(len, Value::Null);
                        for pair in patch["set"].as_array().unwrap() {
                            let index = pair[0].as_u64().unwrap() as usize;
                            let element = &pair[1];
                            let previous = mirror.get(index);
                            if guard_only(previous, element) {
                                guard += 1;
                                guard_bytes += serde_json::to_string(&(index, element)).unwrap().len();
                            }
                            mirror[index] = element.clone();
                            fighters += 1;
                        }
                    }
                }
                _ => {}
            }
        }

        // A rout begins when a regiment breaks or men start fleeing.
        let phase = if routing > 0 || broken > 0 {
            if running { 2 } else { 3 }
        } else {
            1
        };
        for index in [0, phase] {
            let window = &mut windows[index];
            window.ticks += 1;
            window.battle_bytes += battle_total;
            window.guard_bytes += guard_bytes;
            window.fighters += fighters;
            window.guard += guard;
            window.routing += routing as f64;
            window.broken += broken as f64;
            window.journal_us += journal_us;
            window.total_us += total.as_micros();
            window.wal_bytes += wal_bytes;
        }
        let tick = state.hostile.battle.elapsed_ticks;
        if let Some(path) = std::env::var_os("GUARD_DIGEST") {
            if tick % 50 == 0 {
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .expect("digest opens");
                writeln!(file, "{}", digest(&state)).expect("digest writes");
            }
        }
        if standard_probe(&state) {
            emit(format!(
                "GUARD tick={tick} routing={routing} broken={broken} running={running} fighters={fighters} guard_only={guard} guard_B={guard_bytes} battle_B={battle_total} wal_B={wal_bytes}"
            ));
        }
    }
    for window in &windows {
        if window.ticks > 0 {
            window.report();
        }
    }
}

fn standard_probe(state: &TickState) -> bool {
    state.hostile.battle.elapsed_ticks % 200 == 0
}

/// Fixed-seed outcome digest: everything a battle decision changes except the
/// guard deadline itself, so base and fixed trees compare directly.
fn digest(state: &TickState) -> String {
    use std::hash::{Hash, Hasher};
    let battle = &state.hostile.battle;
    let (mut alive, mut wounded, mut health, mut pos) = (0_usize, 0_usize, 0.0_f64, 0.0_f64);
    let mut dead: Vec<i32> = Vec::new();
    for member in &battle.formation.members {
        if member.alive {
            alive += 1;
        } else {
            dead.push(member.id.0);
        }
        if member.wounded {
            wounded += 1;
        }
        if let Some(slot) = state.entities.slot_of(member.id) {
            let at = state.entities.pos(slot);
            health += f64::from(state.entities.row(slot).health);
            pos += at[0] * 3.0 + at[1] * 5.0 + at[2] * 7.0;
        }
    }
    dead.sort_unstable();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    dead.hash(&mut hasher);
    let kills: u16 = battle
        .formation
        .regiments
        .iter()
        .map(|regiment| regiment.morale.kills)
        .sum();
    format!(
        "tick={} alive={alive} wounded={wounded} dead_hash={:016x} health={:016x} pos={:016x} kills={kills} running={}",
        battle.elapsed_ticks,
        hasher.finish(),
        health.to_bits(),
        pos.to_bits(),
        battle.running,
    )
}
