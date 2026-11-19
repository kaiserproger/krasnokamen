//! Delivering one admitted event and retiring an instance that failed.

use super::*;

use super::clock::timer_refusal;
use super::lifecycle::batch_limit;
use crate::HostError;
use crate::adapter::{AdapterError, PlayerSessions, to_script_batch};
use crate::bindings::exports::krasnokamen::plugin::events::EventContext;
use crate::timers;
use mc_script::{
    CommandBatchError, ScriptBatchSubmissionError, ScriptEventKind, ScriptHostEndpoint,
};

/// Whether one instance wants this event: by subscription, or - for a player
/// command - because it declared the root the player ran.
pub(super) fn interested<S: HostServices + 'static>(
    hosted: &Hosted<S>,
    name: &str,
    event: &ScriptEvent,
) -> bool {
    match event.kind() {
        ScriptEventKind::PlayerCommand { root, .. } => hosted.commands.contains(root),
        _ => hosted.subscriptions.contains(name),
    }
}

/// Call one instance and submit what it answered.
///
/// The answer holds two kinds of request: the commands the server owns, which go
/// through the one admission every plugin command takes, and the timers this
/// instance owns, which are applied to its own schedule. Both are staged together
/// and both are published only when the whole answer was admitted, so a batch the
/// host refuses commits no timer of its own either, and a plugin cannot leave a
/// timer behind by answering one it cannot have.
///
/// A failure retires the instance and removes its routes: a guest that traps or
/// answers something invalid must not keep a registration that promises answers
/// it can no longer give. A retired instance is never called again, so the timers
/// it held never fire.
pub(super) fn deliver<S: HostServices + 'static>(
    hosted: &mut Hosted<S>,
    endpoint: &ScriptHostEndpoint,
    sessions: &dyn PlayerSessions,
    context: EventContext,
    events: &[Event],
) {
    let started = Instant::now();
    let result = hosted.instance.on_events(context, events);
    hosted.record_callback(started);
    let batch = match result {
        Ok(batch) => batch,
        Err(error) => {
            // The guest answered with a `plugin-error`: that is a first-class
            // answer, not misbehaviour, so the batch is dropped and the instance
            // keeps serving. Only an instance the call itself retired (a trap, an
            // exhausted budget, an answer past a bound) loses its routes.
            if hosted.instance.retired_because().is_some() {
                retire(hosted, endpoint, &error);
            } else {
                tracing::debug!(plugin = %hosted.id, %error, "event refused by the plugin");
                hosted.diagnostics.commands_refused += 1;
            }
            return;
        }
    };
    hosted.diagnostics.events_delivered += events.len() as u64;
    let (batch, mutations) = match batch.split_timers(hosted.observed_tick) {
        Ok(split) => split,
        Err(error) => {
            // A timer request the contract cannot accept is the plugin's own
            // malformed answer, like a command the server's DTO refuses.
            retire(hosted, endpoint, &HostError::Answer(timer_refusal(error)));
            return;
        }
    };
    // The timers this answer asks for are staged on a copy of the instance's own
    // schedule, so an answer the host does not admit commits no timer either: a
    // refused batch leaves the plugin's timers exactly where they were.
    let staged = match timers::stage(&hosted.timers, mutations) {
        Ok(staged) => staged,
        Err(error) => {
            retire(hosted, endpoint, &HostError::Answer(timer_refusal(error)));
            return;
        }
    };
    if let Err(error) = submit(
        endpoint,
        &hosted.admission,
        batch,
        batch_limit(&hosted.limits),
        sessions,
        &mut hosted.diagnostics,
    ) {
        // A batch that could not be submitted is not a broken guest: queue
        // backpressure, a full admission ledger or a player who left between the
        // event and admission all drop the batch and leave the instance live.
        // Only a batch the *contract* refused (forged provenance, invalid DTO,
        // denied capability) is misbehaviour.
        match error {
            SubmissionFailure::Transient(message) => {
                tracing::debug!(plugin = %hosted.id, %message, "batch dropped, instance stays live");
                notify_batch_rejected(hosted, endpoint, context);
            }
            SubmissionFailure::Contract(message) => {
                retire(hosted, endpoint, &HostError::Answer(message));
            }
        }
        return;
    }
    if let Some(staged) = staged {
        hosted.timers = staged;
    }
}

/// Tell one component that the preceding batch was not admitted, giving it one
/// no-output turn to discard local correlations that named commands the boundary
/// never received.
fn notify_batch_rejected<S: HostServices + 'static>(
    hosted: &mut Hosted<S>,
    endpoint: &ScriptHostEndpoint,
    context: EventContext,
) {
    let started = Instant::now();
    let result = hosted
        .instance
        .on_events(context, &[Event::CommandBatchRejected]);
    hosted.record_callback(started);
    let batch = match result {
        Ok(batch) => batch,
        Err(error) => {
            if hosted.instance.retired_because().is_some() {
                retire(hosted, endpoint, &error);
            } else {
                tracing::debug!(plugin = %hosted.id, %error, "plugin refused batch-rejection notification");
                hosted.diagnostics.commands_refused += 1;
            }
            return;
        }
    };
    if !batch.into_commands().is_empty() {
        retire(
            hosted,
            endpoint,
            &HostError::Answer("batch-rejection notification returned commands".to_owned()),
        );
        return;
    }
    hosted.diagnostics.events_delivered += 1;
}

pub(super) fn retire<S: HostServices + 'static>(
    hosted: &mut Hosted<S>,
    endpoint: &ScriptHostEndpoint,
    error: &HostError,
) {
    if hosted.auth_provider {
        tracing::error!(
            plugin = %hosted.id,
            %error,
            "auth plugin {} retired: {error}; restart required",
            hosted.id,
        );
    } else {
        tracing::warn!(plugin = %hosted.id, %error, "plugin instance retired after a failed callback");
    }
    hosted.retired = true;
    hosted.timers = TimerSchedule::default();
    endpoint.unregister_plugin_routes(&hosted.id);
}

pub(super) fn submit(
    endpoint: &ScriptHostEndpoint,
    admission: &HostCommandAdmission,
    batch: CommandBatch,
    batch_limit: NonZeroUsize,
    sessions: &dyn PlayerSessions,
    diagnostics: &mut InstanceDiagnostics,
) -> Result<(), SubmissionFailure> {
    let converted = match to_script_batch(batch, batch_limit, sessions, admission.capabilities()) {
        Ok(converted) => converted,
        Err(error) => {
            diagnostics.commands_refused += 1;
            return Err(match error {
                // A player who holds no session now: the command is refused, the
                // instance keeps running.
                AdapterError::UnknownPlayer => SubmissionFailure::Transient(format!("{error:?}")),
                // Everything else is the plugin's own answer being wrong: a
                // command past a bound the contract declares, a command needing a
                // capability the package never declared, a batch past the bound
                // the host itself enforces, or a command claiming host provenance.
                AdapterError::InvalidCommand { .. }
                | AdapterError::PermissionDenied { .. }
                | AdapterError::ProvenanceRejected
                | AdapterError::BatchRejected => SubmissionFailure::Contract(format!("{error:?}")),
            });
        }
    };
    let count = converted.commands().len() as u64;
    match endpoint.try_submit_plugin_batch(admission, converted) {
        Ok(()) => {
            diagnostics.commands_submitted += count;
            Ok(())
        }
        Err(error) => {
            diagnostics.commands_refused += count;
            // Queue backpressure and a full admission ledger are transient: the
            // batch is dropped and a later callback can retry. A forged command
            // or an unavailable admission cannot be retried into validity, and
            // only the contract's own refusals retire the instance.
            let transient = match &error {
                ScriptBatchSubmissionError::Full(_) | ScriptBatchSubmissionError::Closed(_) => true,
                ScriptBatchSubmissionError::Rejected { error, .. } => {
                    matches!(error, CommandBatchError::AdmissionUnavailable)
                }
                // The enum is `#[non_exhaustive]`: a refusal this host has not
                // been taught about is treated as backpressure rather than as the
                // package being broken, because retiring an instance and its
                // routes is the destructive answer and only the contract's own
                // refusals earn it. A new variant belongs in this match.
                _ => true,
            };
            let message = format!("{error:?}");
            Err(if transient {
                SubmissionFailure::Transient(message)
            } else {
                SubmissionFailure::Contract(message)
            })
        }
    }
}
