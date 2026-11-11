//! Vanilla `/debug start|stop|function` (permission level 3).
//!
//! DEBUG-PROFILER: `start` begins profiling the server tick (result 0) and
//! `stop` reports the elapsed seconds, the ticks run and the ticks per second;
//! its result is the whole ticks per second. `start` while running and `stop`
//! while stopped fail with `commands.debug.alreadyRunning` / `notRunning`.
//!
//! DEBUG-FUNCTION-TRACE: `/debug function <id|#tag>` runs the function(s)
//! for real while a [`Tracer`] records every command, its result, the nested
//! function calls, the messages the commands send and their errors in the
//! pinned `DebugCommand$Tracer` format. The trace is written to
//! `debug/debug-trace-<UTC date>.txt` beside the world folder (the server
//! directory, as vanilla's `getFile("debug")`) by a worker thread
//! (never the tick), the result is the number of commands of the traced
//! functions, and tracing from inside a trace refuses with `noRecursion`.
use super::command_execution::CommandResult;
use crate::boundary::{ChangeSet, ConnectionId, EntityId};
use crate::command_feedback::text;
use crate::tick::TickState;
use std::time::Instant;

#[derive(Default)]
pub(crate) struct Profiler {
    /// Tick and wall clock at `/debug start`.
    pub(crate) started: Option<(u64, Instant)>,
}

/// The trace file under construction: vanilla `DebugCommand$Tracer` state.
pub(crate) struct Tracer {
    out: String,
    last_indent: usize,
    waiting_for_result: bool,
    /// The commanding connection: its command feedback is traced, not sent.
    pub(crate) connection: ConnectionId,
}

impl Tracer {
    fn new(connection: ConnectionId) -> Self {
        Self {
            out: String::new(),
            last_indent: 0,
            waiting_for_result: false,
            connection,
        }
    }

    fn print_indent(&mut self, depth: usize) {
        for _ in 0..=depth {
            self.out.push(' ');
        }
    }

    fn indent_and_save(&mut self, depth: usize) {
        self.print_indent(depth);
        self.last_indent = depth;
    }

    fn new_line(&mut self) {
        if self.waiting_for_result {
            self.out.push('\n');
            self.waiting_for_result = false;
        }
    }

    pub(crate) fn on_command(&mut self, depth: usize, command: &str) {
        self.new_line();
        self.indent_and_save(depth);
        self.out.push_str("[C]");
        self.out.push_str(command);
        self.waiting_for_result = true;
    }

    pub(crate) fn on_return(&mut self, depth: usize, command: &str, result: i32) {
        if self.waiting_for_result {
            self.out.push_str(&format!(" -> {result}\n"));
            self.waiting_for_result = false;
        } else {
            self.indent_and_save(depth);
            self.out.push_str(&format!("[R ={result}]{command}\n"));
        }
    }

    pub(crate) fn on_call(&mut self, depth: usize, function: &str, size: usize) {
        self.new_line();
        self.indent_and_save(depth);
        self.out.push_str(&format!("[F]{function} size={size}\n"));
    }

    pub(crate) fn on_error(&mut self, message: &str) {
        self.new_line();
        self.indent_and_save(self.last_indent + 1);
        self.out.push_str("[E]");
        self.out.push_str(message);
    }

    pub(crate) fn on_message(&mut self, message: &str) {
        self.new_line();
        self.print_indent(self.last_indent + 1);
        self.out.push_str("[M]");
        self.out.push_str(message);
        self.out.push('\n');
    }
}

/// `Component.getString()` without a language table: literal text, the keys
/// of translatable components with their arguments, and every `extra`.
pub(crate) fn plain(component: &mc_nbt::Tag) -> String {
    use mc_nbt::Tag;
    match component {
        Tag::String(value) => value.clone(),
        Tag::List(list) => list.elements.iter().map(plain).collect(),
        Tag::Compound(fields) => {
            let field = |name: &str| fields.iter().find(|(key, _)| key == name).map(|(_, v)| v);
            let mut out = String::new();
            if let Some(Tag::String(value)) = field("text") {
                out.push_str(value);
            } else if let Some(Tag::String(key)) = field("translate") {
                out.push_str(key);
                if let Some(Tag::List(arguments)) = field("with") {
                    let arguments: Vec<_> = arguments.elements.iter().map(plain).collect();
                    out.push('[');
                    out.push_str(&arguments.join(", "));
                    out.push(']');
                }
            }
            if let Some(extra) = field("extra") {
                out.push_str(&plain(extra));
            }
            out
        }
        _ => String::new(),
    }
}

/// `commands.debug.*` need permission level 3 (`Commands.LEVEL_ADMINS`).
fn admitted(state: &mut TickState, connection: ConnectionId) -> bool {
    if state
        .command_context
        .current
        .is_some_and(|context| context.permission >= 3)
    {
        return true;
    }
    crate::command_execution::error(
        state,
        connection,
        crate::commands::CommandError::Unknown.message(),
    );
    false
}

pub(crate) fn start(state: &mut TickState, connection: ConnectionId) -> CommandResult {
    if !admitted(state, connection) {
        return CommandResult::Failed;
    }
    if state.debug_profiler.started.is_some() {
        crate::command_execution::feedback(
            state,
            connection,
            "commands.debug.alreadyRunning",
            Vec::new(),
        );
        return CommandResult::Failed;
    }
    state.debug_profiler.started = Some((state.tick, Instant::now()));
    crate::command_execution::success_feedback(
        state,
        connection,
        "commands.debug.started",
        Vec::new(),
    );
    CommandResult::Success(0)
}

pub(crate) fn stop(state: &mut TickState, connection: ConnectionId) -> CommandResult {
    if !admitted(state, connection) {
        return CommandResult::Failed;
    }
    let Some((start_tick, start)) = state.debug_profiler.started.take() else {
        crate::command_execution::feedback(
            state,
            connection,
            "commands.debug.notRunning",
            Vec::new(),
        );
        return CommandResult::Failed;
    };
    let seconds = start.elapsed().as_secs_f64();
    let ticks = state.tick.saturating_sub(start_tick);
    let per_second = ticks as f64 / seconds;
    crate::command_execution::success_feedback(
        state,
        connection,
        "commands.debug.stopped",
        vec![
            text(format!("{seconds:.2}")),
            text(ticks),
            text(format!("{per_second:.2}")),
        ],
    );
    CommandResult::Success(per_second as i32)
}

/// `yyyy-MM-dd_HH.mm.ss` of the tick's wall clock, in UTC (vanilla's
/// `Util.getFilenameFormattedDateTime` uses the host's zone; the core carries
/// no zone database).
fn file_stamp(unix_seconds: i64) -> String {
    let stamp = crate::advancements::timestamp_from_unix(unix_seconds);
    format!("{}_{}", &stamp[..10], stamp[11..19].replace(':', "."))
}

/// `/debug function <id|#tag>`: resolve like `FunctionArgument`, run the
/// functions with the tracer attached, then hand the trace to the writer.
pub(crate) fn trace_function(
    state: &mut TickState,
    connection: ConnectionId,
    executor: EntityId,
    name: &str,
    changes: &mut ChangeSet,
) -> CommandResult {
    if !admitted(state, connection) {
        return CommandResult::Failed;
    }
    if state.debug_trace.is_some() {
        crate::command_execution::feedback(
            state,
            connection,
            "commands.debug.function.noRecursion",
            Vec::new(),
        );
        return CommandResult::Failed;
    }
    // A trace needs the function queue to itself: from inside a running
    // function its frames would run after the trace was finished.
    if state.datapacks.running {
        crate::command_execution::feedback(
            state,
            connection,
            "commands.debug.function.traceFailed",
            Vec::new(),
        );
        return CommandResult::Failed;
    }
    let canonical = |raw: &str| {
        if raw.contains(':') {
            raw.to_owned()
        } else {
            format!("minecraft:{raw}")
        }
    };
    let tag = name.strip_prefix('#');
    let packs = state.datapacks.packs();
    let resolved: Option<Vec<String>> = match tag {
        Some(raw) => packs.tags.get(&canonical(raw)).map(|names| names.to_vec()),
        None => {
            let id = canonical(name);
            packs.functions.contains_key(&id).then(|| vec![id])
        }
    };
    let Some(functions) = resolved else {
        crate::command_execution::feedback(
            state,
            connection,
            if tag.is_some() {
                "arguments.function.tag.unknown"
            } else {
                "arguments.function.unknown"
            },
            vec![text(tag.unwrap_or(name))],
        );
        return CommandResult::Failed;
    };
    let commands: usize = functions
        .iter()
        .map(|id| {
            state
                .datapacks
                .packs()
                .functions
                .get(id)
                .map_or(0, |lines| lines.len())
        })
        .sum();
    state.debug_trace = Some(Tracer::new(connection));
    crate::datapacks::run_functions(state, connection, executor, name, changes);
    let tracer = state.debug_trace.take().expect("trace installed above");
    let file = format!("debug-trace-{}.txt", file_stamp((state.wall_clock)()));
    state.datapacks.write_trace(connection, &file, tracer.out);
    let arguments = if functions.len() == 1 {
        vec![text(commands), text(&functions[0]), text(&file)]
    } else {
        vec![text(commands), text(functions.len()), text(&file)]
    };
    crate::command_execution::success_feedback(
        state,
        connection,
        if functions.len() == 1 {
            "commands.debug.function.success.single"
        } else {
            "commands.debug.function.success.multiple"
        },
        arguments,
    );
    CommandResult::Success(commands as i32)
}

#[cfg(test)]
#[path = "command_debug_tests.rs"]
mod tests;
