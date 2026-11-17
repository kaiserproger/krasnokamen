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
    ]
    .map(Outcome::name);
    assert_eq!(
        names,
        [
            "reseated", "dead", "wounded", "routing", "inactive", "targeted", "plan_end",
            "stopped", "gone"
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
         from=[0.000,64.000,0.000] to=[1.000,64.000,3.000] travel=3.162 peak=3.000 outcome=stopped"
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

/// The body cap: one flapper cannot spend the whole line budget, and the run
/// reports that it was cut.
#[test]
fn the_body_cap_stops_a_flapping_body_from_using_every_line() {
    let mut exposure = Exposure::default();
    for tick in 0..(LINES_MAX as u64 * 4) {
        exposure.begin(tick);
        // one body closes and reopens every tick, twice the line budget
        exposure.exposed(EntityId(1), 0, [0.0, 0.0, 0.0], Plan::Chase);
    }
    exposure.halt(None);
    let summary = exposure.lines().last().expect("summary");
    assert!(summary.contains("episodes=1 bodies=1"), "{summary}");
}

/// A mass close at battle scale: 1000 distinct bodies exposed in one tick and
/// closed in the next stays bounded, linear and truthful.
#[test]
fn a_thousand_bodies_close_in_one_tick_within_the_caps() {
    let mut exposure = Exposure::default();
    exposure.begin(1);
    for id in 0..1000 {
        exposure.exposed(
            EntityId(id),
            id as usize,
            [f64::from(id), 0.0, 0.0],
            Plan::Flank,
        );
    }
    // every body moves half a block, then the whole set leaves the guard set
    exposure.begin(2);
    exposure.halt(None);
    let lines = exposure.lines();
    let episodes = lines
        .iter()
        .filter(|line| line.starts_with("Battle exposure: "))
        .count();
    assert_eq!(episodes, BODIES_MAX, "the body cap bounds the reported set");
    let summary = lines.last().expect("summary");
    assert!(summary.contains("bodies=64"), "{summary}");
    assert!(summary.contains("suppressed=936"), "{summary}");
    assert!(summary.ends_with("capped=true"), "{summary}");
}

/// A paused battle closes its open episodes and starts a fresh run.
#[test]
fn halt_closes_open_episodes_and_starts_a_fresh_run() {
    let mut exposure = Exposure::default();
    exposure.begin(50);
    exposure.exposed(EntityId(5), 1, [0.0, 0.0, 0.0], Plan::Flank);
    exposure.halt(None);
    assert!(exposure.lines()[0].ends_with("outcome=stopped"));
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
    assert!(exposure.lines().is_empty());
}
