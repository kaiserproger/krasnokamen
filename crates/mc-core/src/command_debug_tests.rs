use super::*;
use std::time::{Duration, Instant};

fn fixture() -> (TickState, tempfile::TempDir) {
    let root = tempfile::tempdir().unwrap();
    let packs = root.path().join("datapacks");
    std::fs::create_dir(&packs).unwrap();
    let (mut state, _, _) = crate::hostile_tests::setup(8.0);
    state.datapacks =
        crate::datapacks::Runtime::new(Default::default(), packs, state.registry.clone());
    state.plugin_players.observe_permission(ConnectionId(1), 4);
    (state, root)
}

fn run(state: &mut TickState, text: &str, permission: u8) {
    let executor = state.connections.entity_of(ConnectionId(1)).unwrap();
    let context =
        crate::command_context::Context::source(state, ConnectionId(1), executor, permission);
    crate::command_context::run(state, text, context, &mut ChangeSet::default());
}

fn finish(state: &mut TickState) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while state.command_context.waiting() {
        assert!(
            Instant::now() < deadline,
            "debug report worker did not complete"
        );
        crate::datapacks::tick(state, &mut ChangeSet::default());
        std::thread::yield_now();
    }
}

#[test]
fn profiler_stop_publishes_actual_tick_rate_and_independent_report_before_success() {
    let (mut state, root) = fixture();
    run(&mut state, "scoreboard objectives add oracle dummy", 4);
    state.tick = 10_004;
    state.debug_profiler.session = Some((4, Instant::now() - Duration::from_secs(2)));
    run(
        &mut state,
        "execute store result score rate oracle run debug stop",
        4,
    );
    assert!(state.command_context.waiting());
    assert!(!state.scoreboard.scores.contains_key("rate"));
    finish(&mut state);
    assert!(state.scoreboard.scores["rate"]["oracle"].value > 20);
    let paths = std::fs::read_dir(root.path().join("debug"))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(paths.len(), 1);
    let report = std::fs::read_to_string(paths[0].path()).unwrap();
    assert!(
        report.contains("Start tick: 4\nEnd tick: 10004\n"),
        "{report}"
    );
    assert!(report.contains("Tick span: 10000 ticks\n"), "{report}");
    assert!(report.contains("Average TPS:"), "{report}");
    let feedback = format!("{:?}", state.outbox);
    assert!(feedback.contains("commands.debug.stopped"), "{feedback}");
    assert!(feedback.contains("10000"), "{feedback}");
}

#[test]
fn profiler_permissions_duplicate_start_and_idle_stop_do_not_mutate_session() {
    let (mut state, _root) = fixture();
    run(&mut state, "debug start", 2);
    assert!(state.debug_profiler.session.is_none());
    run(&mut state, "debug start", 3);
    let original = state.debug_profiler.session;
    assert!(original.is_some());
    run(&mut state, "debug start", 3);
    assert_eq!(state.debug_profiler.session, original);
    assert!(format!("{:?}", state.outbox).contains("commands.debug.alreadyRunning"));
    run(&mut state, "debug stop", 3);
    finish(&mut state);
    assert!(state.debug_profiler.session.is_none());
    run(&mut state, "debug stop", 3);
    assert!(!state.command_context.waiting());
    assert!(format!("{:?}", state.outbox).contains("commands.debug.notRunning"));
}

#[test]
fn profiler_does_not_expire_after_thirty_minutes_or_invent_one_tick() {
    let (mut state, root) = fixture();
    state.debug_profiler.session = Some((state.tick, Instant::now() - Duration::from_secs(1801)));
    run(&mut state, "scoreboard objectives add oracle dummy", 4);
    run(
        &mut state,
        "execute store result score rate oracle run debug stop",
        4,
    );
    finish(&mut state);
    assert_eq!(state.scoreboard.scores["rate"]["oracle"].value, 0);
    let entry = std::fs::read_dir(root.path().join("debug"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert!(
        std::fs::read_to_string(entry.path())
            .unwrap()
            .contains("Tick span: 0 ticks")
    );
}

#[test]
fn profiler_report_io_failure_is_not_success() {
    let (mut state, root) = fixture();
    std::fs::write(root.path().join("debug"), "not a directory").unwrap();
    run(&mut state, "scoreboard objectives add oracle dummy", 4);
    run(&mut state, "debug start", 3);
    run(
        &mut state,
        "execute store success score completed oracle run debug stop",
        3,
    );
    finish(&mut state);
    assert_eq!(state.scoreboard.scores["completed"]["oracle"].value, 0);
    assert!(!format!("{:?}", state.outbox).contains("commands.debug.stopped"));
}

fn install_functions(state: &mut TickState, root: &std::path::Path, functions: &[(&str, &[&str])]) {
    let mut packs = state.datapacks.packs().clone();
    for (name, lines) in functions {
        packs.functions.insert(
            (*name).into(),
            lines.iter().map(|line| (*line).to_owned()).collect(),
        );
    }
    packs
        .tags
        .insert("test:traced".into(), vec!["test:outer".to_owned()].into());
    state
        .datapacks
        .install(packs, root.join("datapacks"), state.registry.clone());
}

fn report(root: &std::path::Path) -> String {
    let files = std::fs::read_dir(root.join("debug"))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(files.len(), 1);
    std::fs::read_to_string(files[0].path()).unwrap()
}

#[test]
fn function_trace_uses_real_interpreter_messages_results_and_change_set() {
    for target in ["test:outer", "#test:traced"] {
        let (mut state, root) = fixture();
        install_functions(
            &mut state,
            root.path(),
            &[
                ("test:outer", &["function test:inner", "say trace-message"]),
                (
                    "test:inner",
                    &["summon minecraft:cow 6 60 4", "time set 321"],
                ),
            ],
        );
        let executor = state.connections.entity_of(ConnectionId(1)).unwrap();
        let context = crate::command_context::Context::source(&state, ConnectionId(1), executor, 3);
        let mut changes = ChangeSet::default();
        crate::command_context::run(
            &mut state,
            &format!("debug function {target}"),
            context,
            &mut changes,
        );
        assert_eq!(state.day_time, 321);
        assert_eq!(
            changes.entities_spawned.len(),
            1,
            "trace must not discard the mutation ChangeSet"
        );
        finish(&mut state);
        let trace = report(root.path());
        assert!(trace.contains("[F] test:outer"), "{trace}");
        assert!(trace.contains("[F] test:inner"), "{trace}");
        assert!(
            trace.contains("[M]") && trace.contains("trace-message"),
            "{trace}"
        );
        assert!(trace.contains("[R] success=true result=1"), "{trace}");
        assert!(!trace.contains(": ok"), "{trace}");
        assert!(
            state.debug_traces.is_empty(),
            "completed reports are not retained in RAM"
        );
    }
}

#[test]
fn deferred_trace_finishes_children_before_report_and_outer_function_continuation() {
    let (mut state, root) = fixture();
    install_functions(
        &mut state,
        root.path(),
        &[
            (
                "test:caller",
                &[
                    "execute store result score trace oracle run debug function test:outer",
                    "scoreboard players operation after oracle = trace oracle",
                ],
            ),
            (
                "test:outer",
                &[
                    "execute store success score created oracle run datapack create traced \"Traced pack\"",
                    "execute store result score packs oracle run datapack list",
                    "scoreboard players set last oracle 9",
                ],
            ),
        ],
    );
    run(&mut state, "scoreboard objectives add oracle dummy", 4);
    run(&mut state, "function test:caller", 4);
    assert!(state.command_context.waiting());
    assert!(!state.scoreboard.scores.contains_key("after"));
    assert!(!state.scoreboard.scores.contains_key("last"));
    state.push_player(
        ConnectionId(1),
        crate::boundary::PlayerChange::Chat {
            message: "PRIVATE-IDLE-MESSAGE".into(),
        },
    );
    state.push_player(
        ConnectionId(2),
        crate::boundary::PlayerChange::Chat {
            message: "OTHER-SOURCE-MESSAGE".into(),
        },
    );
    finish(&mut state);
    assert_eq!(state.scoreboard.scores["created"]["oracle"].value, 1);
    assert_eq!(state.scoreboard.scores["packs"]["oracle"].value, 2);
    assert_eq!(state.scoreboard.scores["last"]["oracle"].value, 9);
    assert_eq!(state.scoreboard.scores["trace"]["oracle"].value, 3);
    assert_eq!(state.scoreboard.scores["after"]["oracle"].value, 3);
    let trace = report(root.path());
    assert!(
        trace.contains("commands.datapack.create.success"),
        "{trace}"
    );
    assert!(trace.contains("[R] success=true result=2"), "{trace}");
    assert!(!trace.contains("PRIVATE-IDLE-MESSAGE"), "{trace}");
    assert!(!trace.contains("OTHER-SOURCE-MESSAGE"), "{trace}");
    assert!(
        !trace.contains("operation after"),
        "outer caller leaked into trace: {trace}"
    );
}

#[test]
fn trace_permissions_and_recursive_error_preserve_outer_execution() {
    let (mut state, root) = fixture();
    install_functions(
        &mut state,
        root.path(),
        &[(
            "test:outer",
            &["execute run debug function test:outer", "time set 743"],
        )],
    );
    run(&mut state, "debug function test:outer", 2);
    assert!(!state.command_context.waiting());
    assert!(!root.path().join("debug").exists());
    run(&mut state, "debug function test:outer", 3);
    finish(&mut state);
    assert_eq!(state.day_time, 743);
    let trace = report(root.path());
    assert!(
        trace.contains("commands.debug.function.noRecursion"),
        "{trace}"
    );
    assert!(trace.contains("[R] success=false result=0"), "{trace}");
    assert!(trace.contains("[R] success=true result=1"), "{trace}");
    assert!(!state.debug_tracing);
}

#[test]
fn trace_output_exhaustion_is_bounded_and_stores_failure_after_report() {
    let (mut state, root) = fixture();
    install_functions(
        &mut state,
        root.path(),
        &[("test:outer", &["datapack list"])],
    );
    run(&mut state, "scoreboard objectives add oracle dummy", 4);
    run(
        &mut state,
        "execute store success score traced oracle run debug function test:outer",
        4,
    );
    let trace = state.debug_traces.last_mut().unwrap();
    let large = "x".repeat(MAX_TRACE_BYTES);
    trace.record(format_args!("{large}"));
    assert!(trace.truncated);
    assert!(trace.content.len() <= MAX_TRACE_BYTES + 128);
    finish(&mut state);
    assert_eq!(state.scoreboard.scores["traced"]["oracle"].value, 0);
    assert!(report(root.path()).contains("trace output budget exceeded"));
    assert!(format!("{:?}", state.outbox).contains("Function trace budget exceeded"));
}

#[test]
fn trace_command_budget_exhaustion_is_not_success() {
    let (mut state, root) = fixture();
    install_functions(
        &mut state,
        root.path(),
        &[(
            "test:outer",
            &["time set 101", "time set 102", "time set 103"],
        )],
    );
    run(&mut state, "scoreboard objectives add oracle dummy", 4);
    run(
        &mut state,
        "gamerule minecraft:max_command_sequence_length 4",
        4,
    );
    run(
        &mut state,
        "execute store success score traced oracle run debug function test:outer",
        4,
    );
    finish(&mut state);
    assert_eq!(state.scoreboard.scores["traced"]["oracle"].value, 0);
    assert_ne!(state.day_time, 103);
    assert!(report(root.path()).contains("Function command budget exceeded"));
}

#[test]
fn function_trace_plain_reload_waits_for_worker_and_writes_one_report() {
    let (mut state, root) = fixture();
    install_functions(&mut state, root.path(), &[("test:reload", &["reload"])]);
    run(&mut state, "debug function test:reload", 3);
    assert!(state.command_context.waiting());
    assert!(!root.path().join("debug").exists());
    finish(&mut state);
    let trace = report(root.path());
    assert!(trace.contains("reload"), "{trace}");
    assert!(trace.contains("[R] success=true"), "{trace}");
    assert!(!format!("{:?}", state.outbox).contains("already in progress"));
    assert!(
        state.datapacks.packs().functions.is_empty(),
        "reload must publish before the trace completes"
    );
}
