//! Revalidation of an immutable event snapshot handed back by a runtime.

use crate::*;

impl ScriptEvent {
    pub fn validate(&self) -> Result<(), ScriptDtoError> {
        if let Some(plugin_id) = &self.target_plugin_id {
            validate_target_plugin_id(plugin_id)?;
        }
        match &self.kind {
            ScriptEventKind::ServerStarted | ScriptEventKind::ServerTick { .. } => Ok(()),
            ScriptEventKind::ServerStopping { reason }
            | ScriptEventKind::PlayerLeft { reason, .. } => validate_bounded_nonempty(
                "event reason",
                reason,
                MAX_SCRIPT_DISCONNECT_REASON_BYTES,
            ),
            ScriptEventKind::PlayerJoined {
                username, context, ..
            } => {
                validate_bounded_nonempty(
                    "event username",
                    username,
                    MAX_SCRIPT_PLAYER_NAME_BYTES,
                )?;
                context.validate()
            }
            ScriptEventKind::PlayerChat {
                message, context, ..
            } => {
                validate_bounded_nonempty(
                    "event chat message",
                    message,
                    MAX_SCRIPT_CHAT_MESSAGE_BYTES,
                )?;
                context.validate()
            }
            ScriptEventKind::PlayerBlockBroken {
                context,
                dimension,
                block_id,
                ..
            }
            | ScriptEventKind::PlayerBlockPlaced {
                context,
                dimension,
                block_id,
                ..
            } => {
                context.validate()?;
                validate_contract_resource_id(dimension)?;
                validate_contract_resource_id(block_id).map(drop)
            }
            ScriptEventKind::PlayerItemCrafted {
                context,
                dimension,
                item_id,
                count,
                craft_count,
                ..
            } => {
                context.validate()?;
                validate_contract_resource_id(dimension)?;
                validate_contract_resource_id(item_id)?;
                if *count == 0 || *craft_count == 0 {
                    return Err(ScriptDtoError::InvalidAmount);
                }
                Ok(())
            }
            ScriptEventKind::PlayerItemPickedUp {
                context,
                dimension,
                item_id,
                count,
                ..
            } => {
                context.validate()?;
                validate_contract_resource_id(dimension)?;
                validate_contract_resource_id(item_id)?;
                if *count == 0 {
                    return Err(ScriptDtoError::InvalidAmount);
                }
                Ok(())
            }
            ScriptEventKind::PlayerEntityKilled {
                context,
                dimension,
                entity_type,
                ..
            }
            | ScriptEventKind::PlayerEntityInteracted {
                context,
                dimension,
                entity_type,
                ..
            } => {
                context.validate()?;
                validate_contract_resource_id(dimension)?;
                validate_contract_resource_id(entity_type).map(drop)
            }
            ScriptEventKind::PlayerDied {
                context, dimension, ..
            } => {
                context.validate()?;
                validate_contract_resource_id(dimension)?;
                Ok(())
            }
            ScriptEventKind::PlayerGraveCreated {
                context,
                dimension,
                x,
                y,
                z,
                items,
                spilled,
                ..
            } => {
                context.validate()?;
                validate_contract_resource_id(dimension)?;
                if f64::from(*x).abs() > SCRIPT_HORIZONTAL_COORDINATE_LIMIT
                    || f64::from(*y).abs() > SCRIPT_VERTICAL_COORDINATE_LIMIT
                    || f64::from(*z).abs() > SCRIPT_HORIZONTAL_COORDINATE_LIMIT
                    || *items > MAX_OWNED_INVENTORY_SLOTS as u32
                    || *spilled > MAX_OWNED_INVENTORY_SLOTS as u32
                {
                    return Err(ScriptDtoError::InvalidBounds);
                }
                Ok(())
            }
            ScriptEventKind::PlayerCommand {
                username,
                root,
                arguments,
                context,
                ..
            } => {
                validate_bounded_nonempty(
                    "event username",
                    username,
                    MAX_SCRIPT_PLAYER_NAME_BYTES,
                )?;
                validate_bounded_nonempty(
                    "event command root",
                    root,
                    MAX_PLAYER_COMMAND_ROOT_BYTES,
                )?;
                validate_bounded_value(
                    "event command arguments",
                    arguments,
                    MAX_SCRIPT_CHAT_MESSAGE_BYTES,
                )?;
                context.validate()
            }
            ScriptEventKind::PluginStorageGetResult {
                request_id,
                key,
                value,
                version,
                failure,
            } => {
                validate_script_id(request_id)?;
                validate_plugin_storage_key(key)?;
                if value.is_some() != version.is_some() {
                    return Err(ScriptDtoError::InconsistentResult {
                        field: "plugin storage get value/version",
                    });
                }
                if let Some(value) = value {
                    validate_plugin_storage_value(value)?;
                }
                if failure.is_some() && (value.is_some() || version.is_some()) {
                    return Err(ScriptDtoError::InconsistentResult {
                        field: "plugin storage get failure/result",
                    });
                }
                Ok(())
            }
            ScriptEventKind::PluginStorageCasResult {
                request_id,
                key,
                applied,
                version,
                failure,
            }
            | ScriptEventKind::PluginStorageDeleteResult {
                request_id,
                key,
                deleted: applied,
                version,
                failure,
            } => {
                validate_script_id(request_id)?;
                validate_plugin_storage_key(key)?;
                if *applied && version.is_none() {
                    return Err(ScriptDtoError::InconsistentResult {
                        field: "plugin storage mutation success/version",
                    });
                }
                if failure.is_some() && (*applied || version.is_some()) {
                    return Err(ScriptDtoError::InconsistentResult {
                        field: "plugin storage mutation failure/result",
                    });
                }
                Ok(())
            }
            ScriptEventKind::InventoryMenuClicked {
                context,
                menu_id,
                slot,
                ..
            } => {
                context.validate()?;
                validate_script_id(menu_id)?;
                if usize::from(*slot) >= MAX_INVENTORY_MENU_SLOTS {
                    return Err(ScriptDtoError::InvalidBounds);
                }
                Ok(())
            }
            ScriptEventKind::InventoryStorageTransactionResult { request_id, .. } => {
                validate_script_id(request_id).map(drop)
            }
            ScriptEventKind::OperationResult {
                request_id,
                operation_id,
                outcome,
            } => {
                validate_script_id_value(request_id)?;
                if let Some(operation_id) = operation_id {
                    validate_script_id_value(operation_id)?;
                }
                outcome.validate()
            }
            ScriptEventKind::PlayerZoneEntered {
                context, zone_id, ..
            }
            | ScriptEventKind::PlayerZoneExited {
                context, zone_id, ..
            } => {
                context.validate()?;
                validate_script_id(zone_id).map(drop)
            }
            ScriptEventKind::ZoneCommandResult { zone_id, .. } => {
                validate_script_id(zone_id).map(drop)
            }
            ScriptEventKind::PlayerInventoryTransactionResult { request_id, .. }
            | ScriptEventKind::PlayerTeleportResult { request_id, .. }
            | ScriptEventKind::WorldTimeSetResult { request_id, .. } => {
                validate_script_id(request_id).map(drop)
            }
            ScriptEventKind::WorldBlockSetResult {
                request_id,
                dimension,
                block_id,
                applied,
                failure,
                ..
            } => {
                validate_script_id(request_id)?;
                validate_contract_resource_id(dimension)?;
                validate_contract_resource_id(block_id)?;
                if *applied != failure.is_none() {
                    return Err(ScriptDtoError::InconsistentResult {
                        field: "world block set result",
                    });
                }
                Ok(())
            }
            ScriptEventKind::LoaderItemGrantResult {
                request_id,
                player_id,
                block_id,
                count,
                ..
            } => ScriptLoaderItemGrantRequest::try_new(request_id, *player_id, block_id, *count)
                .map(drop),
            ScriptEventKind::EntitySpawnResult {
                request_id,
                entity_type,
                position,
                ..
            } => {
                validate_script_id(request_id)?;
                validate_contract_resource_id(entity_type)?;
                ScriptPosition::try_new(position.x(), position.y(), position.z())
                    .ok_or(ScriptDtoError::InvalidBounds)
                    .map(drop)
            }
            ScriptEventKind::EntityDamageResult {
                request_id,
                entity_id,
                amount_bits,
                health_bits,
                killed,
                failure,
            } => {
                validate_script_id(request_id)?;
                ScriptEntityDamageRequest::try_new(
                    request_id,
                    *entity_id,
                    f32::from_bits(*amount_bits),
                )?;
                let health = health_bits.map(f32::from_bits);
                if health.is_some() != failure.is_none()
                    || *killed && health.is_none()
                    || health.is_some_and(|health| !health.is_finite() || health < 0.0)
                    || *killed && health.is_some_and(|health| health > 0.0)
                {
                    return Err(ScriptDtoError::InconsistentResult {
                        field: "entity damage result",
                    });
                }
                Ok(())
            }
            ScriptEventKind::OnlinePlayersResult {
                request_id,
                players,
                ..
            } => {
                validate_script_id(request_id)?;
                if players.len() > MAX_ONLINE_PLAYER_QUERY_LIMIT {
                    return Err(ScriptDtoError::TooManyEntries {
                        field: "online player snapshots",
                        max: MAX_ONLINE_PLAYER_QUERY_LIMIT,
                    });
                }
                for player in players {
                    player.validate()?;
                }
                Ok(())
            }
            ScriptEventKind::LoaderViewRequest { .. } => Ok(()),
            ScriptEventKind::LoaderViewAction {
                view_instance_id,
                view_revision,
                action_id,
                action_sequence,
                fields,
                selection_token,
                ..
            } => {
                validate_contract_resource_id(view_instance_id)?;
                validate_contract_resource_id(action_id)?;
                if *view_revision > MAX_CLIENT_VIEW_EXACT_INTEGER
                    || *action_sequence > MAX_CLIENT_VIEW_EXACT_INTEGER
                {
                    return Err(ScriptDtoError::InvalidBounds);
                }
                if fields.len() > MAX_CLIENT_VIEW_FIELDS {
                    return Err(ScriptDtoError::TooManyEntries {
                        field: "view action fields",
                        max: MAX_CLIENT_VIEW_FIELDS,
                    });
                }
                let mut ids = std::collections::BTreeSet::new();
                for field in fields {
                    if !ids.insert(field.id()) {
                        return Err(ScriptDtoError::DuplicateId {
                            field: "view action field",
                            actual_bytes: field.id().len(),
                        });
                    }
                }
                if let Some(token) = selection_token.as_deref() {
                    validate_bounded_nonempty(
                        "view selection token",
                        token,
                        MAX_CLIENT_VIEW_ID_BYTES,
                    )?;
                }
                Ok(())
            }
            ScriptEventKind::ClientViewOpened {
                request_id,
                view_instance_id,
                revision,
                failure,
                ..
            } => {
                validate_script_id(request_id)?;
                if view_instance_id.is_some() != failure.is_none()
                    || revision.is_some() != view_instance_id.is_some()
                    || revision.is_some_and(|revision| {
                        revision == 0 || revision > MAX_CLIENT_VIEW_EXACT_INTEGER
                    })
                {
                    return Err(ScriptDtoError::InconsistentResult {
                        field: "client view opened result",
                    });
                }
                if let Some(id) = view_instance_id.as_deref() {
                    validate_contract_resource_id(id)?;
                }
                Ok(())
            }
            ScriptEventKind::ClientSelectionStarted {
                request_id,
                selection_context_id,
                expires_at_tick,
                failure,
                ..
            } => {
                validate_script_id(request_id)?;
                if selection_context_id.is_some() != expires_at_tick.is_some()
                    || selection_context_id.is_some() != failure.is_none()
                {
                    return Err(ScriptDtoError::InconsistentResult {
                        field: "client selection started result",
                    });
                }
                if let Some(id) = selection_context_id.as_deref() {
                    validate_contract_resource_id(id)?;
                }
                Ok(())
            }
            ScriptEventKind::ClientBrand { brand, .. } => {
                validate_bounded_value("client brand", brand, MAX_SCRIPT_CUSTOM_PAYLOAD_BYTES)
            }
            ScriptEventKind::CustomPayload {
                channel, payload, ..
            } => {
                validate_contract_resource_id(channel)?;
                if payload.len() > MAX_SCRIPT_CUSTOM_PAYLOAD_BYTES {
                    return Err(ScriptDtoError::ValueTooLong {
                        field: "custom payload",
                        max_bytes: MAX_SCRIPT_CUSTOM_PAYLOAD_BYTES,
                        actual_bytes: payload.len(),
                    });
                }
                Ok(())
            }
        }
    }
}
