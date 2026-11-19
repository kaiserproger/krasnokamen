//! Custom payload event snapshots targeted to the channel owner.

use crate::*;

impl ScriptEvent {
    /// Build a custom-payload event snapshot targeted to the channel owner.
    /// The owned payload moves into the event without copying.
    pub fn custom_payload(
        target_plugin_id: impl AsRef<str>,
        player_id: ScriptPlayerId,
        phase: ScriptProtocolPhase,
        channel: impl AsRef<str>,
        payload: Vec<u8>,
    ) -> Result<Self, ScriptDtoError> {
        let target_plugin_id = validate_target_plugin_id(target_plugin_id.as_ref())?;
        let channel = validate_contract_resource_id(channel.as_ref())?;
        if payload.len() > MAX_SCRIPT_CUSTOM_PAYLOAD_BYTES {
            return Err(ScriptDtoError::ValueTooLong {
                field: "custom payload",
                max_bytes: MAX_SCRIPT_CUSTOM_PAYLOAD_BYTES,
                actual_bytes: payload.len(),
            });
        }
        Ok(Self {
            target_plugin_id: Some(target_plugin_id),
            kind: ScriptEventKind::CustomPayload {
                player_id,
                phase,
                channel,
                payload,
            },
            origin_generation: None,
        })
    }
}
