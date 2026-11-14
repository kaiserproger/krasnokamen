//! Diagnostic hashes of real native ticks, not a replacement simulation or TPS gate.
use super::*;
use std::fmt::Write;
use std::hash::Hasher;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct FrameTrace {
    digest: u64,
    intents: usize,
    events: usize,
    arrows: usize,
    target_work: Work,
    steering_work: (u64, u64),
}

struct TraceWriter(rustc_hash::FxHasher);

impl std::fmt::Write for TraceWriter {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        self.0.write(text.as_bytes());
        Ok(())
    }
}

fn vector(hasher: &mut rustc_hash::FxHasher, values: &[f64; 3]) {
    for value in values {
        hasher.write_u64(value.to_bits());
    }
}

pub(super) fn frame_digest(battle: &BattleState, output: &Output) -> FrameTrace {
    let mut writer = TraceWriter(rustc_hash::FxHasher::default());
    writer.0.write_u64(battle.elapsed_ticks);
    writer.0.write_usize(battle.columns.ids.len());
    for bi in 0..battle.columns.ids.len() {
        writer.0.write_i32(battle.columns.ids[bi].0);
        vector(&mut writer.0, &battle.columns.drive[bi]);
        if let Some(target) = battle.columns.targets[bi] {
            writer.0.write_u8(1);
            writer.0.write_i32(target.id.0);
            vector(&mut writer.0, &target.pos);
            writer.0.write_u64(target.distance.to_bits());
        } else {
            writer.0.write_u8(0);
        }
    }
    // Battle footprints and output lanes are ordered Vecs. Stream Debug into
    // the hasher without allocating strings; floats use round-trippable Debug.
    write!(&mut writer, "{:?}{:?}", output.intents, output.events).unwrap();
    writer.0.write_usize(output.arrows.len());
    for arrow in &output.arrows {
        writer.0.write_i32(arrow.owner.0);
        vector(&mut writer.0, &arrow.from);
        vector(&mut writer.0, &arrow.velocity);
        writer.0.write_u32(arrow.damage.to_bits());
    }
    writer.0.write_u64(output.counters.tick);
    writer.0.write_u64(output.counters.units);
    for count in &output.counters.alive_per_faction {
        writer.0.write_u16(count.faction);
        writer.0.write_u64(count.alive);
    }
    FrameTrace {
        digest: writer.0.finish(),
        intents: output.intents.len(),
        events: output.events.len(),
        arrows: output.arrows.len(),
        target_work: battle.work,
        steering_work: (
            battle.formation.work.separation_candidates,
            battle.formation.work.push_calls,
        ),
    }
}

fn run_trace(threads: usize) -> Vec<FrameTrace> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap()
        .install(|| {
            let mut state = tests::staged_flat_fixture(1000);
            staging::execute(
                &mut state,
                &mut crate::boundary::ChangeSet::default(),
                [8.5, 64.0, 8.5],
                &staging::BattleCommand::Start,
            )
            .unwrap();
            let mut contact = false;
            for _ in 0..1800 {
                crate::tick::run_tick(&mut state, &[]);
                if state
                    .hostile
                    .battle
                    .formation
                    .regiments
                    .iter()
                    .any(|regiment| regiment.enemy.is_some() && regiment.gap <= 6.0)
                {
                    contact = true;
                    break;
                }
            }
            assert!(contact, "native armies never reached contact");
            // Contact stress fixture: start real charge pulses through the ordinary
            // order API. This makes raw intents/events non-vacuous within 300 ticks;
            // it does not change the production staged scenario's Advance orders.
            let formation = &mut state.hostile.battle.formation;
            for index in 0..formation.regiments.len() {
                let regiment = &formation.regiments[index];
                if regiment.spec.order == formation::Order::Advance {
                    let id = regiment.spec.id;
                    assert!(formation.set_order(id, formation::Order::Charge));
                }
            }
            state.hostile.battle.trace = Some(FrameTrace::default());
            let mut traces = Vec::with_capacity(300);
            for _ in 0..300 {
                let previous = state.hostile.battle.elapsed_ticks;
                crate::tick::run_tick(&mut state, &[]);
                assert_eq!(state.hostile.battle.elapsed_ticks, previous + 1);
                traces.push(state.hostile.battle.trace.unwrap());
            }
            assert!(
                traces.iter().any(|trace| trace.intents > 0),
                "no native battle intents"
            );
            assert!(
                traces.iter().any(|trace| trace.events > 0),
                "no native battle events"
            );
            traces
        })
}

#[test]
#[ignore = "native 2x1000 contact trace; run explicitly for before/after receipts"]
fn native_battle_serial_trace() {
    for (tick, trace) in run_trace(1).iter().enumerate() {
        println!(
            "BATTLE_TRACE {tick} {:016x} {} {} {}",
            trace.digest, trace.intents, trace.events, trace.arrows
        );
    }
}

#[test]
#[ignore = "native 2x1000, 300 ticks on each of one and sixteen threads"]
fn native_battle_hashes_are_thread_count_invariant() {
    assert_eq!(run_trace(1), run_trace(16));
}
