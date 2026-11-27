//! Suspended command continuations remain owned by the native command runtime.
//! Workers carry only a completion identity and prepared data, never executable callbacks.
use super::{Context, Runtime};
use crate::boundary::ChangeSet;
use crate::tick::TickState;
use mc_domain::{ConnectionId, EntityId};
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CompletionId(u64);

pub(super) struct Waiting {
    id: CompletionId,
    context: Context,
    source: Option<EntityId>,
    stores: Vec<crate::command_store::Store>,
    result: Option<(bool, i32)>,
}

pub(super) struct PausedExecution {
    pub branches: Vec<(String, Context, bool, Vec<crate::command_store::Store>)>,
    pub stage: Option<usize>,
    pub successes: i32,
    pub produced: usize,
    pub depth: usize,
    pub function_running: bool,
    pub awaited: bool,
}

pub(super) enum Action {
    Branches(PausedExecution),
    Functions { depth: usize, stop_depth: usize },
    Trace(Waiting),
    Block(crate::command_blocks::Continuation),
}

impl Runtime {
    pub(crate) fn completion_id(&mut self) -> Result<CompletionId, String> {
        self.next_completion = self
            .next_completion
            .checked_add(1)
            .ok_or("Command completion identity exhausted")?;
        Ok(CompletionId(self.next_completion))
    }

    pub(crate) fn waiting(&self) -> bool {
        self.waiting.is_some()
    }

    pub(crate) fn suspend_functions(&mut self, stop_depth: usize) {
        self.continuations.push_back(Action::Functions {
            depth: self.depth,
            stop_depth,
        });
    }

    pub(crate) fn suspend_block(&mut self, continuation: crate::command_blocks::Continuation) {
        self.continuations.push_back(Action::Block(continuation));
    }
}

pub(crate) fn source_entity(state: &TickState, connection: ConnectionId) -> Option<EntityId> {
    state.connections.entity_of(connection).or_else(|| {
        state
            .other_dimensions
            .values()
            .find_map(|world| world.connections.entity_of(connection))
    })
}

pub(crate) fn suspend(
    state: &mut TickState,
    id: CompletionId,
    context: Context,
    stores: Vec<crate::command_store::Store>,
) -> Result<(), String> {
    let source = source_entity(state, context.connection);
    let waiting = Waiting {
        id,
        context,
        source,
        stores,
        result: None,
    };
    if state.command_context.waiting.is_some() {
        if crate::command_debug::trace_waiting_for(state, id) {
            // The child command and its function frames finish first. Only
            // then can the same I/O worker persist the enclosing trace.
            state
                .command_context
                .continuations
                .push_back(Action::Trace(waiting));
            return Ok(());
        }
        return Err("Command already waiting for completion".into());
    }
    state.command_context.waiting = Some(waiting);
    Ok(())
}

/// Publish only the first result for the admitted identity. A stale or duplicate
/// worker event cannot complete a later command, even from the same connection.
pub(crate) fn complete(state: &mut TickState, id: CompletionId, success: bool, value: i32) {
    if let Some(waiting) = &mut state.command_context.waiting
        && waiting.id == id
        && waiting.result.is_none()
    {
        waiting.result = Some((success, value));
    }
}

pub(crate) fn completion_context(state: &TickState, id: CompletionId) -> Option<Context> {
    state
        .command_context
        .waiting
        .as_ref()
        .filter(|waiting| {
            waiting.id == id && source_is_current(state, waiting.context, waiting.source)
        })
        .map(|waiting| waiting.context)
}

fn source_is_current(state: &TickState, context: Context, source: Option<EntityId>) -> bool {
    context.connection == crate::command_console::CONNECTION
        || source.is_some_and(|source| {
            std::iter::once(state)
                .chain(state.other_dimensions.values().map(Box::as_ref))
                .any(|world| {
                    world.connections.entity_of(context.connection) == Some(source)
                        && world
                            .plugin_players
                            .permission(context.connection)
                            .is_some_and(|permission| permission >= context.permission)
                })
        })
}

pub(crate) fn execute_job(state: &mut TickState, job: &super::Job, changes: &mut ChangeSet) {
    let observed = state.plugin_players.permission(job.context.connection);
    let Some(world) = crate::dimensions::travel::world(state, job.context.dimension) else {
        return;
    };
    if job_source_is_current(world, observed, job) {
        // Keep the shared runtime and every loaded world reachable. `native`
        // selects the source dimension through `execute_context`, which moves
        // the runtime with it; running on a detached child loses global targets,
        // continuations and the shared-command authority.
        run_job(state, job, changes);
    }
}

fn run_job(state: &mut TickState, job: &super::Job, changes: &mut ChangeSet) {
    if let Some(pos) = job.block {
        crate::dimensions::travel::execute_block_chain(
            state,
            job.context.dimension,
            crate::command_blocks::Invocation::Start(pos),
            changes,
        );
    } else {
        super::run(state, &job.text, job.context, changes);
    }
}

/// Whether a queued job's source still owns its world. The connection and the
/// executor entity live in the world the command was queued from, while the
/// permission observed during that world's phase lands in the root's plugin
/// player table (`swap_globals` moves the table), so either table answers for it.
fn job_source_is_current(world: &TickState, observed: Option<u8>, job: &super::Job) -> bool {
    let context = job.context;
    if context.connection == crate::command_console::CONNECTION {
        return true;
    }
    let Some(source) = job.source else {
        return false;
    };
    if world.connections.entity_of(context.connection) != Some(source) {
        return false;
    }
    world
        .plugin_players
        .permission(context.connection)
        .or(observed)
        .is_some_and(|permission| permission >= context.permission)
}

/// Called only at the same authority boundary as ordinary native commands.
pub(crate) fn resume(state: &mut TickState, changes: &mut ChangeSet) {
    if state.dimension_globals.active && !state.command_context.shared {
        return;
    }
    resume_completed(state, changes);
    if !state.dimension_globals.active {
        let mut jobs = std::mem::take(&mut state.command_context.jobs);
        let mut executed = 0;
        while executed < jobs.len() && !state.command_context.waiting() {
            execute_job(state, &jobs[executed], changes);
            executed += 1;
        }
        jobs.drain(..executed);
        jobs.append(&mut state.command_context.jobs);
        state.command_context.jobs = jobs;
    }
}

fn resume_completed(state: &mut TickState, changes: &mut ChangeSet) {
    if !state
        .command_context
        .waiting
        .as_ref()
        .is_some_and(|waiting| waiting.result.is_some())
    {
        return;
    }
    let waiting = state
        .command_context
        .waiting
        .take()
        .expect("completed command");
    if !source_is_current(state, waiting.context, waiting.source) {
        state.command_context.continuations.clear();
        crate::datapacks::cancel_pending(state);
        return;
    }
    let (success, value) = waiting.result.expect("completed command");
    crate::command_debug::trace_result(state, success, value);
    let previous = state.command_context.current.replace(waiting.context);
    let depth = state.command_context.depth;
    state.command_context.resuming = true;
    for store in waiting.stores {
        store.apply(state, success, value);
    }
    let mut actions = std::mem::take(&mut state.command_context.continuations);
    while let Some(action) = actions.pop_front() {
        match action {
            Action::Branches(mut execution) => {
                if execution.awaited && success {
                    execution.successes = execution.successes.saturating_add(1);
                }
                state.command_context.depth = execution.depth;
                let running =
                    std::mem::replace(&mut state.datapacks.running, execution.function_running);
                let branches = execution
                    .branches
                    .iter_mut()
                    .map(|(text, context, continuation, stores)| {
                        (
                            text.as_str(),
                            *context,
                            *continuation,
                            std::mem::take(stores),
                        )
                    })
                    .collect::<VecDeque<_>>();
                state.command_context.completed_successes = super::execute_branches(
                    state,
                    branches,
                    execution.stage,
                    execution.successes,
                    execution.produced,
                    changes,
                );
                state.datapacks.running = running;
            }
            Action::Functions { depth, stop_depth } => {
                state.command_context.depth = depth;
                crate::datapacks::drain_until(state, changes, stop_depth);
            }
            Action::Trace(waiting) => {
                if !source_is_current(state, waiting.context, waiting.source) {
                    crate::datapacks::cancel_pending(state);
                    state.command_context.continuations.clear();
                    break;
                }
                let id = waiting.id;
                let context = waiting.context;
                state.command_context.waiting = Some(waiting);
                if let Err(error) = crate::command_debug::finish_trace(state) {
                    super::error(state, context, error);
                    complete(state, id, false, 0);
                }
            }
            Action::Block(continuation) => {
                let dimension = continuation.dimension;
                let successes = state.command_context.completed_successes;
                // The awaited command retains its budget, but the next block
                // begins a separate invocation just as a synchronous chain does.
                state.command_context.depth = 0;
                state.command_context.resuming = false;
                crate::dimensions::travel::execute_block_chain(
                    state,
                    dimension,
                    crate::command_blocks::Invocation::Resume(continuation, successes),
                    changes,
                );
                state.command_context.resuming = true;
            }
        }
        if state.command_context.waiting.is_some() {
            state.command_context.continuations.append(&mut actions);
            break;
        }
    }
    state.command_context.depth = depth;
    state.command_context.current = previous;
    state.command_context.resuming = false;
}
