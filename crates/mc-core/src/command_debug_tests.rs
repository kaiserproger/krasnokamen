//! `/debug start|stop|function` through the real command path.
use super::*;
use crate::boundary::{ChangeSet, PlayerChange};
use crate::command_execution::CommandResult::{Failed, Success};
use crate::datapacks::{PackSet, Runtime, Selection, settle};
use mc_nbt::Tag;
use std::path::Path;
use std::time::Duration;

fn pack(root: &Path, resources: &[(&str, &str)]) {
    let pack = root.join("t");
    std::fs::create_dir_all(&pack).unwrap();
    std::fs::write(
        pack.join("pack.mcmeta"),
        r#"{"pack":{"pack_format":101,"description":"t"}}"#,
    )
    .unwrap();
    for (path, body) in resources {
        let path = pack.join("data").join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
}

fn fixture(world: &Path) -> TickState {
    // The world folder sits in the server directory, which gets `debug/`.
    let root = world.join("world").join("datapacks");
    pack(
        &root,
        &[
            ("test/function/inner.mcfunction", "time add 3"),
            (
                "test/function/chatty.mcfunction",
                "tellraw @s {\"text\":\"to the player\"}\nsay to everyone\nmsg @s to me\ntime add 2",
            ),
            (
                "test/function/outer.mcfunction",
                "time add 10\nfunction test:inner\nbogus nonsense\ntime add 1",
            ),
            (
                "test/tags/function/both.json",
                r#"{"values":["test:inner","test:outer"]}"#,
            ),
        ],
    );
    let (mut state, _, _) = crate::hostile_tests::setup(8.0);
    state.datapacks = Runtime::new(PackSet::load(&root, Selection::default()).unwrap(), root);
    state
}

fn execute(
    state: &mut TickState,
    line: &str,
    permission: u8,
) -> crate::command_execution::CommandResult {
    state.command_context.current = None;
    let context = crate::command_context::Context::source(
        state,
        crate::command_console::CONNECTION,
        crate::command_console::EXECUTOR,
        permission,
    );
    crate::command_context::begin_sequence(state);
    state.command_context.current = Some(context);
    crate::command_context::native(state, line, context, &mut ChangeSet::default(), false)
        .unwrap_or(Failed)
}

fn text_of(tag: &Tag) -> String {
    plain(tag)
}

/// Console messages since the last call, as plain text.
fn said(state: &mut TickState) -> Vec<String> {
    state
        .outbox
        .remove(&crate::command_console::CONNECTION)
        .unwrap_or_default()
        .iter()
        .map(|change| match change {
            PlayerChange::StyledChat { content } => text_of(content),
            PlayerChange::Chat { message } => message.clone(),
            other => format!("{other:?}"),
        })
        .collect()
}

#[test]
fn profiler_reports_seconds_ticks_and_rate_and_refuses_double_start_and_stop() {
    let world = tempfile::tempdir().unwrap();
    let mut state = fixture(world.path());

    // Permission level 3: a level 2 operator does not even see the command.
    assert_eq!(execute(&mut state, "debug start", 2), Failed);
    assert!(said(&mut state)[0].starts_with("Unknown or incomplete command"));
    assert!(state.debug_profiler.started.is_none());

    assert_eq!(execute(&mut state, "debug stop", 3), Failed);
    assert_eq!(said(&mut state), ["commands.debug.notRunning"]);

    assert_eq!(execute(&mut state, "debug start", 3), Success(0));
    assert_eq!(said(&mut state), ["commands.debug.started"]);
    assert_eq!(execute(&mut state, "debug start", 3), Failed);
    assert_eq!(said(&mut state), ["commands.debug.alreadyRunning"]);

    // 41 ticks over 2 s of wall clock: 20.5 ticks per second, result 20.
    let two_seconds_ago = Instant::now() - Duration::from_secs(2);
    state.debug_profiler.started = Some((state.tick, two_seconds_ago));
    state.tick += 41;
    assert_eq!(execute(&mut state, "debug stop", 3), Success(20));
    assert_eq!(
        said(&mut state),
        ["commands.debug.stopped[2.00, 41, 20.50]"],
        "seconds, ticks, ticks per second, in the vanilla argument order"
    );
    assert!(state.debug_profiler.started.is_none());
    assert_eq!(execute(&mut state, "debug stop", 3), Failed);
    assert_eq!(said(&mut state), ["commands.debug.notRunning"]);
}

fn trace_file(world: &Path) -> (String, String) {
    let mut files = std::fs::read_dir(world.join("debug"))
        .expect("debug folder in the server directory")
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(files.len(), 1, "{files:?}");
    let path = files.pop().unwrap();
    (
        path.file_name().unwrap().to_str().unwrap().to_owned(),
        std::fs::read_to_string(path).unwrap(),
    )
}

#[test]
fn function_trace_records_commands_results_messages_errors_and_nested_calls() {
    let world = tempfile::tempdir().unwrap();
    let mut state = fixture(world.path());
    state.day_time = 0;
    state.wall_clock = || 1_767_225_600 + 86_461;

    assert_eq!(
        execute(&mut state, "debug function test:outer", 3),
        Success(4)
    );
    // The function really ran: 10 + 3 (nested) + 1.
    assert_eq!(state.day_time, 14);
    settle(&mut state);
    let (name, trace) = trace_file(world.path());
    assert_eq!(name, "debug-trace-2026-01-02_00.01.01.txt");

    // Only the summary reaches the player; the commands' own feedback is in the file.
    let shown = said(&mut state);
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert_eq!(
        shown[0],
        format!("commands.debug.function.success.single[4, test:outer, {name}]")
    );

    let lines: Vec<&str> = trace.lines().collect();
    assert_eq!(lines[0], " [F]test:outer size=4");
    assert_eq!(lines[1], "  [C]time add 10");
    assert!(
        lines[2].starts_with("   [M]"),
        "the command's feedback is traced one level in: {lines:?}"
    );
    assert!(
        lines[3].starts_with("  [R =") && lines[3].ends_with("]time add 10"),
        "{lines:?}"
    );
    // Nested call: the calling line shows first, the callee at the caller's
    // depth with its own lines one deeper, then the caller's result.
    assert_eq!(lines[4], "  [C]function test:inner");
    assert_eq!(lines[5], "  [F]test:inner size=1");
    assert_eq!(lines[6], "   [C]time add 3");
    assert!(lines[7].starts_with("    [M]"), "{lines:?}");
    assert!(
        lines[8].starts_with("   [R =") && lines[8].ends_with("]time add 3"),
        "{lines:?}"
    );
    assert_eq!(lines[9], "  [R =0]function test:inner");
    // An error is traced, with the failed command's result 0.
    assert_eq!(lines[10], "  [C]bogus nonsense");
    assert!(
        lines[11].contains("[E]Unknown or incomplete command"),
        "{lines:?}"
    );
    assert!(lines[11].ends_with("[R =0]bogus nonsense"), "{lines:?}");
    assert_eq!(lines[12], "  [C]time add 1");
}

#[test]
fn function_tag_trace_counts_every_function_and_reports_the_multiple_key() {
    let world = tempfile::tempdir().unwrap();
    let mut state = fixture(world.path());
    assert_eq!(
        execute(&mut state, "debug function #test:both", 3),
        Success(5)
    );
    settle(&mut state);
    let (name, trace) = trace_file(world.path());
    assert_eq!(
        said(&mut state),
        [format!(
            "commands.debug.function.success.multiple[5, 2, {name}]"
        )]
    );
    assert!(trace.contains(" [F]test:inner size=1\n"), "{trace}");
    assert!(trace.contains(" [F]test:outer size=4\n"), "{trace}");
}

#[test]
fn function_trace_refuses_unknown_functions_low_permission_and_recursion() {
    let world = tempfile::tempdir().unwrap();
    let mut state = fixture(world.path());

    assert_eq!(execute(&mut state, "debug function test:nope", 3), Failed);
    assert_eq!(said(&mut state), ["arguments.function.unknown[test:nope]"]);
    assert_eq!(execute(&mut state, "debug function #test:nope", 3), Failed);
    assert_eq!(
        said(&mut state),
        ["arguments.function.tag.unknown[test:nope]"]
    );
    assert_eq!(execute(&mut state, "debug function test:inner", 2), Failed);
    assert!(said(&mut state)[0].starts_with("Unknown or incomplete command"));
    assert!(
        !world.path().join("debug").exists(),
        "refusals write no trace"
    );

    // Tracing from inside a trace.
    state.debug_trace = Some(Tracer::new(crate::command_console::CONNECTION));
    assert_eq!(execute(&mut state, "debug function test:inner", 3), Failed);
    let trace = state.debug_trace.take().unwrap();
    assert!(
        trace.out.contains("commands.debug.function.noRecursion"),
        "{}",
        trace.out
    );
}

#[test]
fn function_trace_write_failure_is_reported_not_hidden() {
    let world = tempfile::tempdir().unwrap();
    let mut state = fixture(world.path());
    // A runtime with no world folder has nowhere to write the trace.
    state.datapacks = Runtime::new(
        PackSet::load(
            &world.path().join("world").join("datapacks"),
            Selection::default(),
        )
        .unwrap(),
        std::path::PathBuf::new(),
    );
    assert_eq!(
        execute(&mut state, "debug function test:inner", 3),
        Success(1)
    );
    said(&mut state);
    settle(&mut state);
    assert_eq!(said(&mut state), ["commands.debug.function.traceFailed"]);
}

#[test]
fn function_trace_records_only_what_the_source_is_sent() {
    let world = tempfile::tempdir().unwrap();
    let mut state = fixture(world.path());
    let player = crate::boundary::ConnectionId(1);
    let entity = state.connections.entity_of(player).expect("joined");
    state.outbox.clear();
    state.command_context.current = None;
    let context = crate::command_context::Context::source(&state, player, entity, 3);
    crate::command_context::begin_sequence(&mut state);
    state.command_context.current = Some(context);
    let result = crate::command_context::native(
        &mut state,
        "debug function test:chatty",
        context,
        &mut ChangeSet::default(),
        false,
    );
    assert_eq!(result, Ok(Success(4)));
    settle(&mut state);
    let (_, trace) = trace_file(world.path());
    // The player still gets what the commands deliver to them as a target...
    let delivered: Vec<String> = state
        .outbox
        .get(&player)
        .map(|changes| {
            changes
                .iter()
                .filter_map(|change| match change {
                    PlayerChange::StyledChat { content } => Some(text_of(content)),
                    PlayerChange::Chat { message } => Some(message.clone()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    for expected in ["to the player", "to everyone", "to me"] {
        assert!(
            delivered.iter().any(|line| line.contains(expected)),
            "{expected} reaches the player: {delivered:?}"
        );
    }
    // ... and the trace, which records only what the source is sent, has
    // neither: its messages are the sender's copy of the private message and
    // the time feedback (the command lines themselves are always traced).
    let messages: Vec<&str> = trace.lines().filter(|line| line.contains("[M]")).collect();
    assert_eq!(messages.len(), 2, "{trace}");
    assert!(
        messages[0].contains("commands.message.display.outgoing"),
        "{trace}"
    );
    assert!(messages[1].contains("commands.time.set"), "{trace}");
}
