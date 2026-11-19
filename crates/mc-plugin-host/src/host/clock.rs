//! The host-driven timer clock of every instance.

use super::*;

use super::delivery::retire;
use super::delivery::submit;
use super::lifecycle::batch_limit;
use crate::HostError;
use crate::adapter::PlayerSessions;
use crate::bindings::exports::krasnokamen::plugin::events::{EventContext, TimerFired};
use crate::timers;
use crate::timers::{MAX_TIMER_CALLBACKS_PER_TICK, TimerRefusal};
use mc_script::ScriptHostEndpoint;

/// Drive one instance's own timers from one pushed simulation tick.
///
/// A pushed tick fires the timers whose deadline it reached - earliest deadline
/// first, then smallest timer id - up to the contract's bound of callbacks per
/// tick; the timers still due after that wait for a later pushed tick that moves
/// the clock, and keep the deadline they were scheduled for. A tick that does not
/// move the clock is the tick it repeats: it has already had its delivery, so it
/// fires nothing. Every callback of one delivery shares one allowance and one
/// staging batch, and nothing any of them changed publishes unless the whole
/// delivery and the admission of its commands succeeded: a guest must not multiply
/// its budget by making many timers due at once, and a plugin must not
/// half-reschedule itself because a later callback failed.
pub(super) fn deliver_timers<S: HostServices + 'static>(
    hosted: &mut Hosted<S>,
    endpoint: &ScriptHostEndpoint,
    sessions: &dyn PlayerSessions,
    tick: u64,
) {
    if hosted.initial_timer_clock_pending {
        if let Err(error) = hosted.timers.rebase_deadlines(tick) {
            retire(hosted, endpoint, &HostError::Answer(timer_refusal(error)));
            return;
        }
        hosted.initial_timer_clock_pending = false;
        hosted.observed_tick = tick;
        return;
    }
    // A pushed tick that does not move the clock this instance observes is not a
    // new tick: the value it repeats already had its delivery, so it fires nothing
    // and a timer the per-tick bound deferred waits for a tick that does move.
    // That is also why a stale tick - one the server pushed after a later one -
    // can never fire a timer of its own.
    if tick <= hosted.observed_tick {
        return;
    }
    hosted.observed_tick = tick;
    // Nothing due at this tick is the common case - every plugin that holds no
    // timer, and every pushed tick before one is due - and it costs no copy of the
    // schedule and no guest call at all.
    if hosted.timers.next_due(tick).is_none() {
        return;
    }
    // The schedule one delivery works on is a staging copy. What it takes, cancels
    // or adds becomes the instance's own schedule only once the delivery has been
    // admitted, so a refusal anywhere leaves every timer where it was and a later
    // pushed tick delivers it again.
    let mut staged = hosted.timers.clone();
    let Some(due) = staged.take_next_due(tick) else {
        return;
    };
    if let Err(error) = hosted.instance.arm_delivery() {
        retire(hosted, endpoint, &error);
        return;
    }
    let mut batch = CommandBatch::new();
    let mut due = Some(due);
    let mut delivered = 0_usize;
    let mut command_count = 0;
    while let Some((scheduled_tick, timer_id)) = due {
        let events = [Event::TimerFired(TimerFired {
            timer_id,
            scheduled_tick,
            fired_tick: tick,
        })];
        let context = EventContext {
            tick,
            first_sequence: 0,
            count: 1,
        };
        let started = Instant::now();
        let result = hosted.instance.on_events_within_delivery(context, &events);
        hosted.record_callback(started);
        let answer = match result {
            Ok(answer) => answer,
            Err(error) => {
                // A `plugin-error` is a first-class answer, not misbehaviour: the
                // delivery is dropped and the instance keeps serving, exactly as
                // for any other event. Only an instance the call itself retired (a
                // trap, an exhausted budget, an answer past a bound) loses its
                // routes, and a retired instance has no timers left to fire.
                if hosted.instance.retired_because().is_some() {
                    retire(hosted, endpoint, &error);
                } else {
                    tracing::debug!(plugin = %hosted.id, %error, "timer callback refused by the plugin");
                    hosted.diagnostics.commands_refused += 1;
                }
                return;
            }
        };
        delivered += 1;
        hosted.diagnostics.events_delivered += 1;
        // Timer mutations consume the same allowance as game commands. Count
        // before splitting them out, or each callback gets a fresh timer budget.
        command_count += answer.len();
        if command_count > hosted.limits.commands_per_call {
            retire(
                hosted,
                endpoint,
                &HostError::Answer("the pushed tick exceeded its command bound".to_owned()),
            );
            return;
        }
        let (answer, mutations) = match answer.split_timers(tick) {
            Ok(split) => split,
            Err(error) => {
                retire(hosted, endpoint, &HostError::Answer(timer_refusal(error)));
                return;
            }
        };
        for mutation in mutations {
            // The staged copy is the schedule the rest of this delivery reads, so
            // a timer an earlier callback cancelled cannot fire later in the same
            // pushed tick, and a deadline an earlier callback moved is the one the
            // next callback sees.
            if let Err(error) = timers::apply(&mut staged, mutation) {
                retire(hosted, endpoint, &HostError::Answer(timer_refusal(error)));
                return;
            }
        }
        if let Err(error) = batch.extend(answer.into_commands(), &hosted.limits) {
            // The commands of one pushed tick share the bound one callback's answer
            // has: many due timers must not answer with more host work than a
            // single callback could.
            let refusal = format!("the pushed tick exceeded its command bound: {error}");
            retire(hosted, endpoint, &HostError::Answer(refusal));
            return;
        }
        if delivered >= MAX_TIMER_CALLBACKS_PER_TICK {
            // The contract's bound per pushed tick: the timers still due arrive on
            // a later tick, with the deadline they were scheduled for.
            break;
        }
        due = staged.take_next_due(tick);
    }
    match submit(
        endpoint,
        &hosted.admission,
        batch,
        batch_limit(&hosted.limits),
        sessions,
        &mut hosted.diagnostics,
    ) {
        Ok(()) => hosted.timers = staged,
        // Backpressure, a full admission ledger or a player who left between the
        // callback and admission: nothing of this pushed tick publishes and the
        // instance stays live, so the timers it did not settle arrive again later.
        Err(SubmissionFailure::Transient(message)) => {
            tracing::debug!(plugin = %hosted.id, %message, "timer delivery dropped, instance stays live");
        }
        Err(SubmissionFailure::Contract(message)) => {
            retire(hosted, endpoint, &HostError::Answer(message));
        }
    }
}

/// The words an operator reads for a timer request the host refused.
///
/// The refusal names what the plugin got wrong - the id, the delay, the deadline
/// the delay adds up to, or the pending maximum - because it is the plugin's own
/// answer that broke a bound the contract states, not a state the host could
/// retry on the plugin's behalf.
pub(super) fn timer_refusal(error: TimerRefusal) -> String {
    format!("timer request refused: {error}")
}
