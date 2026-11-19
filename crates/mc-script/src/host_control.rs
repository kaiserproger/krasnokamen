//! Trusted host control input, the admitted event envelope and the host mailbox.

use super::*;

use std::ops::Deref;

/// Trusted control input of the host half of the boundary.
///
/// This is not a server observation and never a guest DTO: the boundary moves a
/// payload between the two trusted halves without interpreting it, and only the
/// runtime that queued it knows what it carries. A consumer that serves events
/// only drops such input unread, which closes whatever response its payload holds.
#[non_exhaustive]
pub enum ScriptHostInput {
    /// One immutable server event, in one FIFO with every other event.
    Event(ScriptHostEvent),
    /// Host-private control: a reload envelope whose payload only its sender reads.
    Reload(Box<dyn std::any::Any + Send>),
    /// One native pre-commit question, in the same FIFO and the same order.
    ///
    /// It is a request rather than an event because only the host can answer it:
    /// the boundary moves it to the runtime that runs the registered handlers, and
    /// the runtime answers it through the request's own one-shot reply. A consumer
    /// that only serves events drops it unread, which answers the owner with
    /// [`precommit::HookFailure::Unavailable`] instead of leaving it waiting.
    Precommit(precommit::Request),
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum EventTargetGeneration {
    Untargeted,
    Targeted(Option<u64>),
}

impl ScriptHostEvent {
    pub(crate) fn untargeted(event: ScriptEvent) -> Self {
        Self {
            event,
            target_generation: EventTargetGeneration::Untargeted,
        }
    }

    /// Recover the immutable guest-visible event after the host accepts it.
    #[must_use]
    pub fn into_event(self) -> ScriptEvent {
        self.event
    }
}

impl Deref for ScriptHostEvent {
    type Target = ScriptEvent;

    fn deref(&self) -> &Self::Target {
        &self.event
    }
}

impl ScriptHostInputSender {
    /// Queue one reload envelope, waiting for bounded mailbox capacity.
    ///
    /// A boundary that closed its host side stops the weak upgrade, and a closed
    /// send fails the same way: both report [`ScriptQueueError::Closed`], which is
    /// what a reload caller turns into its own closed-host result. Capacity is
    /// awaited rather than reported, so [`ScriptQueueError::Full`] is never
    /// returned from here.
    pub async fn send_reload(
        &self,
        payload: Box<dyn std::any::Any + Send>,
    ) -> Result<(), ScriptQueueError> {
        let Some(sender) = self.sender.upgrade() else {
            return Err(ScriptQueueError::Closed);
        };
        sender
            .send(ScriptHostInput::Reload(payload))
            .await
            .map_err(|_| ScriptQueueError::Closed)
    }
}
