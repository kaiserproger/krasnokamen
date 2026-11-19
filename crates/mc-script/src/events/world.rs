//! Targeted results of admitted world, entity and Loader mutations.

use crate::*;

impl ScriptEvent {
    /// Build the targeted result of one admitted same-dimension player teleport.
    pub(crate) fn player_teleport_result(
        target_plugin_id: impl AsRef<str>,
        request: &ScriptPlayerTeleportRequest,
        failure: Option<ScriptPlayerTeleportFailure>,
    ) -> Result<Self, ScriptDtoError> {
        Ok(Self {
            target_plugin_id: Some(validate_target_plugin_id(target_plugin_id.as_ref())?),
            kind: ScriptEventKind::PlayerTeleportResult {
                request_id: request.request_id().to_owned(),
                player_id: request.player_id(),
                position: request.position(),
                failure,
            },
            origin_generation: None,
        })
    }

    /// Build the targeted result of one admitted authoritative world-time mutation.
    pub(crate) fn world_time_set_result(
        target_plugin_id: impl AsRef<str>,
        request: &ScriptWorldTimeSetRequest,
        failure: Option<ScriptWorldTimeSetFailure>,
    ) -> Result<Self, ScriptDtoError> {
        Ok(Self {
            target_plugin_id: Some(validate_target_plugin_id(target_plugin_id.as_ref())?),
            kind: ScriptEventKind::WorldTimeSetResult {
                request_id: request.request_id().to_owned(),
                world_time: request.world_time(),
                failure,
            },
            origin_generation: None,
        })
    }

    /// Build the targeted result of one admitted authoritative default-block mutation.
    pub(crate) fn world_block_set_result(
        target_plugin_id: impl AsRef<str>,
        request: &ScriptWorldBlockSetRequest,
        applied: bool,
        failure: Option<ScriptWorldBlockSetFailure>,
    ) -> Result<Self, ScriptDtoError> {
        if applied != failure.is_none() {
            return Err(ScriptDtoError::InconsistentResult {
                field: "world block set result",
            });
        }
        Ok(Self {
            target_plugin_id: Some(validate_target_plugin_id(target_plugin_id.as_ref())?),
            kind: ScriptEventKind::WorldBlockSetResult {
                request_id: request.request_id().to_owned(),
                dimension: request.dimension().to_owned(),
                block_id: request.block_id().to_owned(),
                x: request.x(),
                y: request.y(),
                z: request.z(),
                applied,
                failure,
            },
            origin_generation: None,
        })
    }

    /// Build the targeted result of one admitted Loader custom-block item grant.
    pub(crate) fn loader_item_grant_result(
        target_plugin_id: impl AsRef<str>,
        request: &ScriptLoaderItemGrantRequest,
        failure: Option<ScriptLoaderItemGrantFailure>,
    ) -> Result<Self, ScriptDtoError> {
        Ok(Self {
            target_plugin_id: Some(validate_target_plugin_id(target_plugin_id.as_ref())?),
            kind: ScriptEventKind::LoaderItemGrantResult {
                request_id: request.request_id().to_owned(),
                player_id: request.player_id(),
                block_id: request.block_id().to_owned(),
                count: request.count(),
                failure,
            },
            origin_generation: None,
        })
    }

    /// Build the targeted result of one admitted entity-spawn mutation.
    pub(crate) fn entity_spawn_result(
        target_plugin_id: impl AsRef<str>,
        request_id: impl AsRef<str>,
        actor: ScriptPlayerId,
        entity_type: impl AsRef<str>,
        position: ScriptPosition,
        failure: Option<ScriptEntitySpawnFailure>,
    ) -> Result<Self, ScriptDtoError> {
        Ok(Self {
            target_plugin_id: Some(validate_target_plugin_id(target_plugin_id.as_ref())?),
            kind: ScriptEventKind::EntitySpawnResult {
                request_id: validate_script_id(request_id.as_ref())?,
                player_id: actor,
                entity_type: validate_contract_resource_id(entity_type.as_ref())?,
                position,
                failure,
            },
            origin_generation: None,
        })
    }

    /// Build the targeted result of one admitted entity-damage mutation.
    pub(crate) fn entity_damage_result(
        target_plugin_id: impl AsRef<str>,
        request: &ScriptEntityDamageRequest,
        health: Option<f32>,
        killed: bool,
        failure: Option<ScriptEntityDamageFailure>,
    ) -> Result<Self, ScriptDtoError> {
        if health.is_some() != failure.is_none()
            || killed && health.is_none()
            || health.is_some_and(|health| !health.is_finite() || health < 0.0)
            || killed && health.is_some_and(|health| health > 0.0)
        {
            return Err(ScriptDtoError::InconsistentResult {
                field: "entity damage result",
            });
        }
        Ok(Self {
            target_plugin_id: Some(validate_target_plugin_id(target_plugin_id.as_ref())?),
            kind: ScriptEventKind::EntityDamageResult {
                request_id: request.request_id().to_owned(),
                entity_id: request.entity_id(),
                amount_bits: request.amount().to_bits(),
                health_bits: health.map(f32::to_bits),
                killed,
                failure,
            },
            origin_generation: None,
        })
    }

    /// Build the targeted result of one admitted connected-player query.
    pub(crate) fn online_players_result(
        target_plugin_id: impl AsRef<str>,
        request: &ScriptOnlinePlayersRequest,
        players: Vec<ScriptOnlinePlayerSnapshot>,
        truncated: bool,
    ) -> Result<Self, ScriptDtoError> {
        if players.len() > request.limit() {
            return Err(ScriptDtoError::TooManyEntries {
                field: "online player snapshots",
                max: request.limit(),
            });
        }
        for player in &players {
            player.validate()?;
        }
        Ok(Self {
            target_plugin_id: Some(validate_target_plugin_id(target_plugin_id.as_ref())?),
            kind: ScriptEventKind::OnlinePlayersResult {
                request_id: request.request_id().to_owned(),
                players,
                truncated,
            },
            origin_generation: None,
        })
    }
}
