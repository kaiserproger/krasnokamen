//! Targeted protected-zone entry/exit and zone command snapshots.

use crate::*;

impl ScriptEvent {
    /// Build a targeted zone-entry snapshot owned by the plugin that registered the zone.
    pub(crate) fn player_zone_entered(
        target_plugin_id: impl AsRef<str>,
        player_id: ScriptPlayerId,
        context: ScriptPlayerContext,
        zone: &ScriptAxisAlignedZone,
    ) -> Result<Self, ScriptDtoError> {
        Ok(Self {
            target_plugin_id: Some(validate_target_plugin_id(target_plugin_id.as_ref())?),
            kind: ScriptEventKind::PlayerZoneEntered {
                player_id,
                context,
                zone_id: zone.id().to_owned(),
            },
            origin_generation: None,
        })
    }

    /// Build a targeted zone-exit snapshot owned by the plugin that registered the zone.
    pub(crate) fn player_zone_exited(
        target_plugin_id: impl AsRef<str>,
        player_id: ScriptPlayerId,
        context: ScriptPlayerContext,
        zone: &ScriptAxisAlignedZone,
    ) -> Result<Self, ScriptDtoError> {
        Ok(Self {
            target_plugin_id: Some(validate_target_plugin_id(target_plugin_id.as_ref())?),
            kind: ScriptEventKind::PlayerZoneExited {
                player_id,
                context,
                zone_id: zone.id().to_owned(),
            },
            origin_generation: None,
        })
    }

    pub(crate) fn zone_command_result(
        target_plugin_id: impl AsRef<str>,
        zone_id: impl AsRef<str>,
        accepted: bool,
    ) -> Result<Self, ScriptDtoError> {
        Ok(Self {
            target_plugin_id: Some(validate_target_plugin_id(target_plugin_id.as_ref())?),
            kind: ScriptEventKind::ZoneCommandResult {
                zone_id: validate_script_id(zone_id.as_ref())?,
                accepted,
            },
            origin_generation: None,
        })
    }
}
