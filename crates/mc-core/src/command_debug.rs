//! Vanilla `/debug` profiler and function tracer.
//!
//! DEBUG-PROFILER: `/debug start` begins tick-time profiling, `/debug stop`
//! ends it and reports elapsed ticks, wall time and average TPS. Starting
//! twice or stopping without a session errors like vanilla. `start` returns
//! 0; `stop` returns the floored average TPS.
//!
//! `/debug function <id|#tag>` uses the ordinary function interpreter, including
//! its command budget and deferred completions. The I/O worker writes the final
//! trace; success is published only after that write. Recursive traces refuse.
//! Permission level 3 is required, matching vanilla.
use super::command_execution::CommandResult;
use crate::boundary::{ChangeSet, ConnectionId, EntityId};
use crate::tick::TickState;

#[derive(Default)]
pub(crate) struct Profiler {
    session: Option<(u64, std::time::Instant)>,
}

/// Prepared on the tick owner; only the existing I/O worker writes the artifact.
pub(crate) struct Report {
    pub feedback: &'static str,
    pub arguments: Vec<mc_nbt::Tag>,
    pub result: i32,
    pub success: bool,
    name: String,
    content: String,
}

impl Report {
    pub(crate) fn write(&self, datapack_root: &std::path::Path) -> Result<(), String> {
        use std::io::Write;
        let directory = datapack_root
            .parent()
            .ok_or("Datapack root has no world directory")?
            .join("debug");
        std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        let path = directory.join(&self.name);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| error.to_string())?;
        if let Err(error) = file
            .write_all(self.content.as_bytes())
            .and_then(|()| file.sync_all())
        {
            drop(file);
            let _ = std::fs::remove_file(path);
            return Err(error.to_string());
        }
        Ok(())
    }
}

/// One active trace; completed traces live on disk, not in an unbounded history.
#[derive(Debug, Default)]
pub(crate) struct TraceArtifact {
    name: String,
    connection: Option<ConnectionId>,
    completion: Option<crate::command_context::CompletionId>,
    commands: usize,
    functions: usize,
    content: String,
    truncated: bool,
}

const MAX_TRACE_BYTES: usize = 16 * 1024 * 1024;

impl std::fmt::Write for TraceArtifact {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        if self.content.len().saturating_add(text.len()) > MAX_TRACE_BYTES {
            return Err(std::fmt::Error);
        }
        self.content.push_str(text);
        Ok(())
    }
}

impl TraceArtifact {
    fn record(&mut self, line: std::fmt::Arguments<'_>) {
        use std::fmt::Write;
        if self.truncated {
            return;
        }
        if self
            .write_fmt(line)
            .and_then(|()| self.write_char('\n'))
            .is_err()
        {
            self.content
                .push_str("\n[ERROR] Function trace output budget exceeded (16 MiB)\n");
            self.truncated = true;
        }
    }
}

pub(crate) fn trace_budget_exhausted(state: &mut TickState) {
    if state.debug_tracing
        && let Some(trace) = state.debug_traces.last_mut()
    {
        trace.record(format_args!("[ERROR] Function command budget exceeded"));
        trace.truncated = true;
    }
}

pub(crate) fn swap_runtime(left: &mut TickState, right: &mut TickState) {
    std::mem::swap(&mut left.debug_profiler, &mut right.debug_profiler);
    std::mem::swap(&mut left.debug_tracing, &mut right.debug_tracing);
    std::mem::swap(&mut left.debug_traces, &mut right.debug_traces);
}

pub(crate) fn trace_enter(state: &mut TickState, name: &str, lines: usize) {
    if state.debug_tracing
        && let Some(trace) = state.debug_traces.last_mut()
    {
        if trace.content.is_empty() {
            trace.name = name.to_owned();
        }
        trace.record(format_args!("[F] {name} size={lines}"));
    }
}

pub(crate) fn trace_command(state: &mut TickState, text: &str) {
    if state.debug_tracing
        && let Some(trace) = state.debug_traces.last_mut()
    {
        trace.commands += 1;
        trace.record(format_args!("[C] {text}"));
    }
}

pub(crate) fn trace_result(state: &mut TickState, success: bool, value: i32) {
    if state.debug_tracing
        && let Some(trace) = state.debug_traces.last_mut()
    {
        trace.record(format_args!("[R] success={success} result={value}"));
    }
}

pub(crate) fn trace_message(
    state: &mut TickState,
    connection: ConnectionId,
    change: &crate::boundary::PlayerChange,
) {
    if !state.debug_tracing {
        return;
    }
    // A deferred trace may span ticks. Unrelated chat (including private
    // messages to this source) during that wait is not command trace output.
    if state
        .command_context
        .current
        .is_none_or(|context| context.connection != connection)
    {
        return;
    }
    if let Some(trace) = state.debug_traces.last_mut()
        && trace.connection == Some(connection)
    {
        match change {
            crate::boundary::PlayerChange::Chat { message } => {
                trace.record(format_args!("[M] {message}"));
            }
            crate::boundary::PlayerChange::StyledChat { .. } => {
                trace.record(format_args!("[M] {change:?}"));
            }
            _ => {}
        }
    }
}

pub(crate) fn trace_waiting_for(
    state: &TickState,
    id: crate::command_context::CompletionId,
) -> bool {
    state.debug_tracing
        && state
            .debug_traces
            .last()
            .is_some_and(|trace| trace.completion == Some(id))
}

pub(crate) fn finish_trace(state: &mut TickState) -> Result<(), String> {
    let trace = state.debug_traces.pop().ok_or("No active function trace")?;
    state.debug_tracing = false;
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = format!("debug-trace-{timestamp}.txt");
    let report = Report {
        feedback: if trace.truncated {
            "Function trace budget exceeded; see the debug trace report"
        } else if trace.functions == 1 {
            "commands.debug.function.success.single"
        } else {
            "commands.debug.function.success.multiple"
        },
        arguments: if trace.truncated {
            Vec::new()
        } else {
            vec![
                mc_nbt::Tag::String(trace.commands.to_string()),
                mc_nbt::Tag::String(if trace.functions == 1 {
                    trace.name
                } else {
                    trace.functions.to_string()
                }),
                mc_nbt::Tag::String(name.clone()),
            ]
        },
        result: if trace.truncated {
            0
        } else {
            trace.commands as i32
        },
        success: !trace.truncated,
        name,
        content: trace.content,
    };
    state.datapacks.request_debug_report(
        trace.connection.expect("trace source"),
        trace.completion.expect("trace completion"),
        report,
    )
}

pub(crate) fn start(
    state: &mut TickState,
    connection: ConnectionId,
    _executor: EntityId,
    command: &crate::commands::Command,
) -> CommandResult {
    let _ = command;
    if state.debug_profiler.session.is_some() {
        crate::command_execution::feedback(
            state,
            connection,
            "commands.debug.alreadyRunning",
            Vec::new(),
        );
        return CommandResult::Failed;
    }
    state.debug_profiler.session = Some((state.tick, std::time::Instant::now()));
    crate::command_execution::success_feedback(
        state,
        connection,
        "commands.debug.started",
        Vec::new(),
    );
    CommandResult::Success(0)
}

pub(crate) fn stop(
    state: &mut TickState,
    connection: ConnectionId,
    _executor: EntityId,
    command: &crate::commands::Command,
) -> CommandResult {
    let _ = command;
    let Some((start_tick, started)) = state.debug_profiler.session.as_ref() else {
        crate::command_execution::feedback(
            state,
            connection,
            "commands.debug.notRunning",
            Vec::new(),
        );
        return CommandResult::Failed;
    };
    let ticks = state.tick.saturating_sub(*start_tick);
    let elapsed = started.elapsed();
    let seconds = elapsed.as_secs_f64();
    let average_tps = ticks as f64 / seconds;
    let completion = match state.command_context.completion_id() {
        Ok(id) => id,
        Err(error) => {
            crate::command_execution::feedback(state, connection, &error, Vec::new());
            return CommandResult::Failed;
        }
    };
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let report = Report {
        feedback: "commands.debug.stopped",
        arguments: vec![
            mc_nbt::Tag::String(format!("{seconds:.2}")),
            mc_nbt::Tag::String(ticks.to_string()),
            mc_nbt::Tag::String(format!("{average_tps:.2}")),
        ],
        result: average_tps as i32,
        success: true,
        name: format!("profile-results-{timestamp}.txt"),
        content: format!(
            "---- Krasnokamen tick profile ----\nStart tick: {start_tick}\nEnd tick: {}\nTime span: {} ms\nTick span: {ticks} ticks\nAverage TPS: {average_tps:.2}\n",
            state.tick,
            elapsed.as_millis(),
        ),
    };
    if let Err(error) = state
        .datapacks
        .request_debug_report(connection, completion, report)
    {
        crate::command_execution::feedback(state, connection, &error, Vec::new());
        return CommandResult::Failed;
    }
    state.debug_profiler.session = None;
    CommandResult::Deferred(completion)
}

pub(crate) fn trace_function(
    state: &mut TickState,
    connection: ConnectionId,
    executor: EntityId,
    name: &str,
    changes: &mut ChangeSet,
) -> CommandResult {
    if state.debug_tracing {
        crate::command_execution::feedback(
            state,
            connection,
            "commands.debug.function.noRecursion",
            Vec::new(),
        );
        return CommandResult::Failed;
    }
    let completion = match state.command_context.completion_id() {
        Ok(id) => id,
        Err(error) => {
            crate::command_execution::feedback(state, connection, &error, Vec::new());
            return CommandResult::Failed;
        }
    };
    let stop_depth = crate::datapacks::pending_depth(state);
    state.debug_tracing = true;
    if let Err(error) = crate::datapacks::enqueue(state, name, connection, executor) {
        state.debug_tracing = false;
        crate::command_execution::feedback(state, connection, &error, Vec::new());
        return CommandResult::Failed;
    }
    state.debug_traces.push(TraceArtifact {
        name: name.to_owned(),
        connection: Some(connection),
        completion: Some(completion),
        functions: crate::datapacks::pending_depth(state) - stop_depth,
        ..Default::default()
    });
    crate::datapacks::drain_until(state, changes, stop_depth);
    if !state.command_context.waiting()
        && let Err(error) = finish_trace(state)
    {
        crate::command_execution::feedback(state, connection, &error, Vec::new());
        return CommandResult::Failed;
    }
    CommandResult::Deferred(completion)
}

#[cfg(test)]
#[path = "command_debug_tests.rs"]
mod tests;
