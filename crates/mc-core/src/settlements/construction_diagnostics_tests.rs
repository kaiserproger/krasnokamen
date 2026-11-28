//! The acceptance half of the phase-log diagnostics: a real planner pass and a
//! courtship window must reach the log, not only the record types.
use super::tests::{fixture, fund, open_plan};
use super::*;
use crate::settlements::decision_log::{Courtship, ForcedEnabled};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct Output(Arc<Mutex<Vec<u8>>>);

impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// One planner decision and one courtship window, captured from the same
/// subscriber. Remove `Plan::emit`'s call in `plan_pass` and this test loses its
/// `settlement_planner_verdict` line; remove `Courtship::observe`'s
/// `tracing::info!` and it loses its `settlement_courtship_summary` line.
#[test]
fn planner_and_courtship_records_reach_the_phase_log() {
    let output = Output::default();
    let writer = output.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_writer(move || writer.clone())
        .finish();
    let (id, ticks) = tracing::subscriber::with_default(subscriber, || {
        let _forced = ForcedEnabled::take();
        // A funded House in a certified, open-plan village: the pass takes a
        // real branch and records it.
        let (mut state, id, _builder) = fixture();
        open_plan(&mut state);
        fund(&mut state, "house", 1);
        plan(&mut state, id);
        // The courtship record closes a window instead of sampling one.
        let mut courtship = Courtship::default();
        courtship.observe(id.0, 43, 10);
        courtship.counts.pairing_bed.no_candidate = 1;
        courtship.observe(id.0, 1_243, 621);
        (id, (43u64, 1_243u64))
    });
    let text = String::from_utf8(output.0.lock().unwrap().clone()).unwrap();
    let planner: Vec<_> = text
        .lines()
        .filter(|line| line.contains("settlement_planner_verdict"))
        .collect();
    assert_eq!(planner.len(), 1, "{text}");
    assert!(
        planner[0].contains(&format!("settlement={} ", id.0)),
        "{text}"
    );
    assert!(planner[0].contains("verdict="), "{text}");
    // This fixture's pass has a funded House whose walk is still running and
    // holds a legal lot, so it takes the `sweep_waiting` branch. The label and
    // the two lane facts below are what `finish!`/`SitePass::summary` record;
    // an unset verdict, a missed branch or a dropped house outcome fails here.
    assert!(planner[0].contains("verdict=\"sweep_waiting\""), "{text}");
    assert!(planner[0].contains("started=false"), "{text}");
    assert!(
        planner[0].contains("ordinary_house_funded=Some(true)")
            && planner[0].contains("ordinary_house_sweep=Some(\"sweep_incomplete\")"),
        "{text}"
    );
    let courtship: Vec<_> = text
        .lines()
        .filter(|line| line.contains("settlement_courtship_summary"))
        .collect();
    assert_eq!(courtship.len(), 1, "{text}");
    let (from_tick, tick) = (ticks.0, ticks.1);
    assert!(
        courtship[0].contains(&format!("settlement={} ", id.0))
            && courtship[0].contains(&format!("from_tick={from_tick}"))
            && courtship[0].contains(&format!("tick={tick}")),
        "{text}"
    );
    assert!(
        courtship[0].contains("pairing_bed_no_candidate=1"),
        "{text}"
    );
}
