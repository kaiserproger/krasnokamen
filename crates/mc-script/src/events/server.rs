//! Server lifecycle and tick event snapshots.

use crate::*;

impl ScriptEvent {
    /// Build a server-started event snapshot.
    pub fn server_started() -> Self {
        Self {
            target_plugin_id: None,
            kind: ScriptEventKind::ServerStarted,
            origin_generation: None,
        }
    }

    /// Build a server-stopping event snapshot.
    pub fn server_stopping(reason: impl AsRef<str>) -> Self {
        let reason = reason.as_ref();
        validate_bounded_nonempty(
            "server stopping reason",
            reason,
            MAX_SCRIPT_DISCONNECT_REASON_BYTES,
        )
        .expect("server-authored stopping reason must be bounded");
        Self {
            target_plugin_id: None,
            kind: ScriptEventKind::ServerStopping {
                reason: reason.to_owned(),
            },
            origin_generation: None,
        }
    }

    /// Build a server-tick event snapshot.
    pub fn server_tick(tick: u64) -> Self {
        Self {
            target_plugin_id: None,
            kind: ScriptEventKind::ServerTick { tick },
            origin_generation: None,
        }
    }
}
