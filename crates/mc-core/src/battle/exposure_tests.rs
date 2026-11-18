use super::*;

fn state(
    alive: bool,
    seated: bool,
    wounded: bool,
    routing: bool,
    active: bool,
    targeted: bool,
    planned: bool,
) -> State {
    State {
        alive,
        seated,
        wounded,
        routing,
        active,
        targeted,
        planned,
    }
}

/// Every guard of `sweep_drives`, in its order, and the one case that keeps the
/// episode open: passes every guard, unseated, inside a planned regiment.
#[test]
fn classify_closes_the_episode_on_the_first_guard_that_ended_it() {
    assert_eq!(
        classify(state(false, false, false, false, true, false, true)),
        Some(Outcome::Dead)
    );
    assert_eq!(
        classify(state(true, true, false, false, true, false, true)),
        Some(Outcome::Reseated)
    );
    assert_eq!(
        classify(state(true, false, true, false, true, false, true)),
        Some(Outcome::Wounded)
    );
    assert_eq!(
        classify(state(true, false, false, true, true, false, true)),
        Some(Outcome::Routing)
    );
    assert_eq!(
        classify(state(true, false, false, false, false, false, true)),
        Some(Outcome::Inactive)
    );
    assert_eq!(
        classify(state(true, false, false, false, true, true, true)),
        Some(Outcome::Targeted)
    );
    assert_eq!(
        classify(state(true, false, false, false, true, false, false)),
        Some(Outcome::PlanEnd)
    );
    assert_eq!(
        classify(state(true, false, false, false, true, false, true)),
        None
    );
}

#[test]
fn outcome_and_plan_names_are_the_ones_the_analyzer_parses() {
    let names = [
        Outcome::Reseated,
        Outcome::Dead,
        Outcome::Wounded,
        Outcome::Routing,
        Outcome::Inactive,
        Outcome::Targeted,
        Outcome::PlanEnd,
        Outcome::Stopped,
        Outcome::Gone,
        Outcome::Unsettled,
    ]
    .map(Outcome::name);
    assert_eq!(
        names,
        [
            "reseated",
            "dead",
            "wounded",
            "routing",
            "inactive",
            "targeted",
            "plan_end",
            "stopped",
            "gone",
            "unsettled"
        ]
    );
    assert_eq!([Plan::Chase.name(), Plan::Flank.name()], ["chase", "flank"]);
}

/// An injected exposure is stamped, measured and reported. The motion window is
/// the phase contract in one line: the guards are evaluated at `tick`, and the
/// movement they cause is published under the following ticks.
#[test]
fn an_episode_reports_duration_travel_peak_and_the_motion_window() {
    let mut exposure = Exposure::default();
    let id = EntityId(1076);
    exposure.begin(5142);
    exposure.exposed(id, 7, [0.0, 64.0, 0.0], Plan::Chase);
    exposure.begin(5143);
    exposure.exposed(id, 7, [1.0, 64.0, 0.0], Plan::Chase);
    exposure.begin(5144);
    exposure.exposed(id, 7, [1.0, 64.0, 3.0], Plan::Flank);
    exposure.halt(None);
    let lines = exposure.lines();
    assert_eq!(
        lines.len(),
        2,
        "one episode line and one summary: {lines:?}"
    );
    assert_eq!(
        lines[0],
        "Battle exposure: id=1076 tick=5142 ticks=3 motion=5143-5145 plan=flank \
         from=[0.000,64.000,0.000] to=[1.000,64.000,3.000] travel=3.162 peak=3.000 excess=1 \
         outcome=unsettled"
    );
    assert_eq!(
        lines[1],
        "Battle exposure summary: start=5142 episodes=1 bodies=1 member_ticks=3 excess_episodes=1 \
         suppressed=0 capped=false"
    );
    assert!(
        lines[0].len() < 220,
        "a line stays bounded: {}",
        lines[0].len()
    );
}

/// The excess flag is the server's own > 2 blocks/tick signal, exclusive at the
/// bound (the D1 step itself is `> 2` blocks).
#[test]
fn excess_counts_only_steps_above_the_bound() {
    let mut exposure = Exposure::default();
    exposure.begin(10);
    exposure.exposed(EntityId(1), 0, [0.0, 0.0, 0.0], Plan::Chase);
    exposure.begin(11);
    exposure.exposed(EntityId(1), 0, [EXCESS_BLOCKS, 0.0, 0.0], Plan::Chase);
    exposure.halt(None);
    assert!(
        exposure.lines()[0].contains(" peak=2.000 excess=0 "),
        "{:?}",
        exposure.lines()[0]
    );
    assert!(
        exposure.lines()[1].contains("excess_episodes=0"),
        "{:?}",
        exposure.lines()
    );
}

/// One body exposed twice is one at-risk body (the campaign's `n` counts
/// bodies), but every episode is reported.
#[test]
fn a_body_re_exposed_is_counted_once_and_reported_twice() {
    let mut exposure = Exposure::default();
    let id = EntityId(7);
    exposure.begin(100);
    exposure.exposed(id, 3, [0.0, 0.0, 0.0], Plan::Chase);
    exposure.halt(None);
    exposure.begin(200);
    exposure.exposed(id, 3, [5.0, 0.0, 0.0], Plan::Chase);
    exposure.halt(None);
    let lines = exposure.lines();
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.starts_with("Battle exposure: "))
            .count(),
        2
    );
    for summary in lines.iter().filter(|line| line.contains("summary")) {
        assert!(summary.contains("episodes=1 bodies=1"), "{summary}");
    }
}

/// A member index that has been reused by another identity closes the old
/// episode as `gone` instead of leaking it into the new body's episode.
#[test]
fn a_reused_member_index_never_merges_two_identities() {
    let mut exposure = Exposure::default();
    exposure.begin(10);
    exposure.exposed(EntityId(1), 4, [0.0, 0.0, 0.0], Plan::Chase);
    exposure.begin(11);
    exposure.exposed(EntityId(2), 4, [8.0, 0.0, 0.0], Plan::Chase);
    exposure.halt(None);
    let lines: Vec<&String> = exposure
        .lines()
        .iter()
        .filter(|line| line.starts_with("Battle exposure: "))
        .collect();
    assert_eq!(lines.len(), 2, "{:?}", exposure.lines());
    assert!(
        lines[0].starts_with("Battle exposure: id=1 "),
        "{}",
        lines[0]
    );
    assert!(
        lines[1].starts_with("Battle exposure: id=2 "),
        "{}",
        lines[1]
    );
    assert!(
        lines[0].contains("from=[0.000,0.000,0.000]"),
        "{}",
        lines[0]
    );
    assert!(
        lines[1].contains("from=[8.000,0.000,0.000]"),
        "{}",
        lines[1]
    );
}

/// The body cap bounds a run of many distinct bodies, the summary names the
/// truncation, and such a run is never usable as a count.
#[test]
fn the_body_cap_bounds_the_distinct_bodies_and_reports_the_rest() {
    let mut exposure = Exposure::default();
    for tick in 0..(LINES_MAX as u64 + 44) {
        exposure.begin(tick);
        exposure.exposed(
            EntityId(tick as i32 + 1),
            tick as usize,
            [0.0, 0.0, 0.0],
            Plan::Chase,
        );
    }
    exposure.halt(None);
    let lines = exposure.lines();
    let episodes = lines
        .iter()
        .filter(|line| line.starts_with("Battle exposure: "))
        .count();
    assert_eq!(episodes, BODIES_MAX, "one line per reported body");
    let summary = lines.last().expect("summary");
    assert_eq!(
        summary,
        "Battle exposure summary: start=0 episodes=64 bodies=64 member_ticks=300 \
         excess_episodes=0 suppressed=236 capped=true"
    );
}

/// The line cap bounds a flapping body's episodes independently of the body
/// cap: what it drops is counted, never silently lost.
#[test]
fn the_line_cap_bounds_the_emitted_episodes() {
    let mut exposure = Exposure::default();
    exposure.begin(0);
    exposure.bodies.push(EntityId(1));
    let episode = Episode {
        id: EntityId(1),
        member: 0,
        first_tick: 0,
        last_tick: 0,
        from: [0.0; 3],
        to: [0.0; 3],
        peak: 0.0,
        plan: Plan::Chase,
    };
    for _ in 0..(LINES_MAX + 44) {
        exposure.emit_episode(episode, Outcome::Targeted);
    }
    assert_eq!(exposure.emitted, LINES_MAX);
    assert_eq!(exposure.suppressed, 44);
    assert_eq!(exposure.lines.len(), LINES_MAX as usize);
    exposure.halt(None);
    assert!(
        exposure
            .lines()
            .last()
            .unwrap()
            .ends_with("suppressed=44 capped=true"),
        "{:?}",
        exposure.lines().last()
    );
}

/// A run that started reports even with no exposure at all: the summary is what
/// tells a clean take from a log without the event.
#[test]
fn a_run_without_exposure_writes_one_summary() {
    let mut exposure = Exposure::default();
    exposure.begin(4242);
    exposure.halt(None);
    assert_eq!(
        exposure.lines(),
        &[
            "Battle exposure summary: start=4242 episodes=0 bodies=0 member_ticks=0 \
           excess_episodes=0 suppressed=0 capped=false"
                .to_string()
        ],
        "one summary, no episode line"
    );
    exposure.halt(None);
    assert_eq!(
        exposure.lines().len(),
        1,
        "a run that never started reports nothing"
    );
}

/// One flapping body — closing and reopening every tick — does exhaust the line
/// budget, and the run says so instead of silently dropping lines.
#[test]
fn a_flapping_body_exhausts_the_line_cap_and_the_run_reports_it() {
    let (entities, formation, columns) = (
        EntityStore::default(),
        FormationRuntime::default(),
        Columns::default(),
    );
    let sample = Sample {
        entities: &entities,
        formation: &formation,
        columns: &columns,
    };
    let mut exposure = Exposure::default();
    let mut tick = 0;
    for _ in 0..(LINES_MAX + 44) {
        exposure.begin(tick);
        exposure.exposed(EntityId(1), 0, [0.0, 0.0, 0.0], Plan::Chase);
        tick += 1;
        // the body is not exposed on the next tick: its episode closes (`gone`)
        exposure.begin(tick);
        exposure.close(&sample, &[], &[]);
        tick += 1;
    }
    exposure.halt(None);
    let lines = exposure.lines();
    let episodes = lines
        .iter()
        .filter(|line| line.starts_with("Battle exposure: "))
        .count();
    assert_eq!(episodes, LINES_MAX as usize, "the line cap bounds one body");
    let summary = lines.last().expect("summary");
    assert!(summary.contains("episodes=300 bodies=1"), "{summary}");
    assert!(summary.contains("suppressed=44"), "{summary}");
    assert!(summary.ends_with("capped=true"), "{summary}");
}

/// The body cap refuses a new body past the bound but never refuses a body the
/// run has already reported: a known id may close and reopen freely.
#[test]
fn the_body_cap_refuses_a_new_body_but_never_a_known_one() {
    let (entities, formation, columns) = (
        EntityStore::default(),
        FormationRuntime::default(),
        Columns::default(),
    );
    let sample = Sample {
        entities: &entities,
        formation: &formation,
        columns: &columns,
    };
    let mut exposure = Exposure::default();
    let mut tick = 0;
    for id in 0..BODIES_MAX as i32 {
        exposure.begin(tick);
        exposure.exposed(EntityId(id), id as usize, [0.0, 0.0, 0.0], Plan::Chase);
        tick += 1;
        exposure.begin(tick);
        exposure.close(&sample, &[], &[]);
        tick += 1;
    }
    // a 65th body is refused …
    exposure.begin(tick);
    exposure.exposed(EntityId(9999), 9999, [0.0, 0.0, 0.0], Plan::Chase);
    assert!(!exposure.is_exposed(EntityId(9999)));
    assert_eq!(exposure.suppressed, 1);
    // … while a known body re-enters without being suppressed
    exposure.begin(tick + 1);
    exposure.exposed(EntityId(5), 5, [0.0, 0.0, 0.0], Plan::Chase);
    assert!(exposure.is_exposed(EntityId(5)));
    assert_eq!(
        exposure.suppressed, 1,
        "a known body is never refused for the body cap"
    );
    exposure.halt(None);
    let summary = exposure.lines().last().expect("summary");
    assert!(
        summary.contains(&format!("bodies={BODIES_MAX}")),
        "{summary}"
    );
}

/// A paused battle closes its open episodes and starts a fresh run.
#[test]
fn halt_closes_open_episodes_and_starts_a_fresh_run() {
    let mut exposure = Exposure::default();
    exposure.begin(50);
    exposure.exposed(EntityId(5), 1, [0.0, 0.0, 0.0], Plan::Flank);
    exposure.halt(None);
    assert!(exposure.lines()[0].ends_with("outcome=unsettled"));
    assert_eq!(exposure.open, 0);
    assert_eq!(exposure.member_ticks, 0);
    exposure.halt(None);
    assert_eq!(
        exposure
            .lines()
            .iter()
            .filter(|line| line.contains("summary"))
            .count(),
        1,
        "an empty halt reports nothing twice"
    );
    exposure.begin(90);
    exposure.exposed(EntityId(6), 2, [0.0, 0.0, 0.0], Plan::Chase);
    assert!(exposure.is_exposed(EntityId(6)));
    exposure.halt(None);
    assert!(exposure.lines().last().unwrap().contains("episodes=1"));
}

/// One body exposed over many ticks is one episode, not one per tick.
#[test]
fn a_body_exposed_over_many_ticks_is_one_episode() {
    let mut exposure = Exposure::default();
    for tick in 0..(LINES_MAX as u64 * 4) {
        exposure.begin(tick);
        exposure.exposed(EntityId(1), 0, [0.0, 0.0, 0.0], Plan::Chase);
    }
    exposure.halt(None);
    let summary = exposure.lines().last().expect("summary");
    assert!(summary.contains("episodes=1 bodies=1"), "{summary}");
    assert!(
        summary.contains(&format!("member_ticks={}", LINES_MAX * 4)),
        "{summary}"
    );
}

/// With the observation switched off (the telemetry mutation control) the
/// tracker records nothing; the simulation is untouched either way.
#[test]
fn a_disabled_tracker_records_no_episode() {
    let mut exposure = Exposure::default();
    exposure.set_enabled(false);
    exposure.begin(5);
    exposure.exposed(EntityId(1), 0, [0.0, 0.0, 0.0], Plan::Chase);
    assert!(!exposure.is_exposed(EntityId(1)));
    assert_eq!(exposure.member_ticks, 0);
    exposure.halt(None);
    let lines = exposure.lines();
    assert!(
        lines
            .iter()
            .all(|line| !line.starts_with("Battle exposure: ")),
        "no episode is recorded: {lines:?}"
    );
    assert!(
        lines.last().unwrap().contains("episodes=0"),
        "a run that started still reports its empty summary: {lines:?}"
    );
}
