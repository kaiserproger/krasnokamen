//! Player gameplay event snapshots built after an authoritative commit.

use crate::*;

impl ScriptEvent {
    /// Build a player-joined event snapshot with server-authoritative context.
    pub fn player_joined_with_context(
        player_id: ScriptPlayerId,
        context: ScriptPlayerContext,
    ) -> Self {
        context
            .validate()
            .expect("server-authored player context must be bounded");
        Self {
            target_plugin_id: None,
            kind: ScriptEventKind::PlayerJoined {
                player_id,
                username: context.username().to_owned(),
                context,
            },
            origin_generation: None,
        }
    }

    /// Build a player-left event snapshot.
    pub fn player_left(player_id: ScriptPlayerId, reason: impl AsRef<str>) -> Self {
        let reason = reason.as_ref();
        validate_bounded_nonempty(
            "player-left reason",
            reason,
            MAX_SCRIPT_DISCONNECT_REASON_BYTES,
        )
        .expect("server-authored player-left reason must be bounded");
        Self {
            target_plugin_id: None,
            kind: ScriptEventKind::PlayerLeft {
                player_id,
                reason: reason.to_owned(),
            },
            origin_generation: None,
        }
    }

    /// Build a player-chat event snapshot with server-authoritative context.
    pub fn player_chat_with_context(
        player_id: ScriptPlayerId,
        message: impl AsRef<str>,
        context: ScriptPlayerContext,
    ) -> Self {
        let message = message.as_ref();
        validate_bounded_nonempty(
            "player chat message",
            message,
            MAX_SCRIPT_CHAT_MESSAGE_BYTES,
        )
        .expect("server-authored player chat must be bounded");
        context
            .validate()
            .expect("server-authored player context must be bounded");
        Self {
            target_plugin_id: None,
            kind: ScriptEventKind::PlayerChat {
                player_id,
                message: message.to_owned(),
                context,
            },
            origin_generation: None,
        }
    }

    /// Build a reliable block-break event after the authoritative world commit.
    #[allow(clippy::too_many_arguments)]
    pub fn try_player_block_broken_with_context(
        player_id: ScriptPlayerId,
        context: ScriptPlayerContext,
        dimension: impl AsRef<str>,
        block_id: impl AsRef<str>,
        x: i32,
        y: i32,
        z: i32,
        game_mode: ScriptGameMode,
    ) -> Result<Self, ScriptDtoError> {
        context.validate()?;
        Ok(Self {
            target_plugin_id: None,
            kind: ScriptEventKind::PlayerBlockBroken {
                player_id,
                context,
                dimension: validate_contract_resource_id(dimension.as_ref())?,
                block_id: validate_contract_resource_id(block_id.as_ref())?,
                x,
                y,
                z,
                game_mode,
            },
            origin_generation: None,
        })
    }

    /// Build a reliable block-place event after the authoritative world commit.
    #[allow(clippy::too_many_arguments)]
    pub fn try_player_block_placed_with_context(
        player_id: ScriptPlayerId,
        context: ScriptPlayerContext,
        dimension: impl AsRef<str>,
        block_id: impl AsRef<str>,
        x: i32,
        y: i32,
        z: i32,
        game_mode: ScriptGameMode,
    ) -> Result<Self, ScriptDtoError> {
        context.validate()?;
        Ok(Self {
            target_plugin_id: None,
            kind: ScriptEventKind::PlayerBlockPlaced {
                player_id,
                context,
                dimension: validate_contract_resource_id(dimension.as_ref())?,
                block_id: validate_contract_resource_id(block_id.as_ref())?,
                x,
                y,
                z,
                game_mode,
            },
            origin_generation: None,
        })
    }

    /// Build a reliable item-crafted event after the authoritative inventory commit.
    #[allow(clippy::too_many_arguments)]
    pub fn try_player_item_crafted_with_context(
        player_id: ScriptPlayerId,
        context: ScriptPlayerContext,
        dimension: impl AsRef<str>,
        item_id: impl AsRef<str>,
        count: u64,
        craft_count: u32,
        source: ScriptCraftingSource,
        game_mode: ScriptGameMode,
    ) -> Result<Self, ScriptDtoError> {
        context.validate()?;
        if count == 0 || craft_count == 0 {
            return Err(ScriptDtoError::InvalidAmount);
        }
        Ok(Self {
            target_plugin_id: None,
            kind: ScriptEventKind::PlayerItemCrafted {
                player_id,
                context,
                dimension: validate_contract_resource_id(dimension.as_ref())?,
                item_id: validate_contract_resource_id(item_id.as_ref())?,
                count,
                craft_count,
                source,
                game_mode,
            },
            origin_generation: None,
        })
    }

    /// Build a reliable item-pickup event after the authoritative inventory commit.
    #[allow(clippy::too_many_arguments)]
    pub fn try_player_item_picked_up_with_context(
        player_id: ScriptPlayerId,
        context: ScriptPlayerContext,
        dimension: impl AsRef<str>,
        item_id: impl AsRef<str>,
        count: u64,
        source: ScriptItemPickupSource,
        game_mode: ScriptGameMode,
    ) -> Result<Self, ScriptDtoError> {
        context.validate()?;
        if count == 0 {
            return Err(ScriptDtoError::InvalidAmount);
        }
        Ok(Self {
            target_plugin_id: None,
            kind: ScriptEventKind::PlayerItemPickedUp {
                player_id,
                context,
                dimension: validate_contract_resource_id(dimension.as_ref())?,
                item_id: validate_contract_resource_id(item_id.as_ref())?,
                count,
                source,
                game_mode,
            },
            origin_generation: None,
        })
    }

    /// Build a reliable direct player-melee kill event after the entity commit.
    pub fn try_player_entity_killed_with_context(
        player_id: ScriptPlayerId,
        context: ScriptPlayerContext,
        dimension: impl AsRef<str>,
        entity_id: ScriptEntityId,
        entity_type: impl AsRef<str>,
        source: ScriptEntityKillSource,
        game_mode: ScriptGameMode,
    ) -> Result<Self, ScriptDtoError> {
        context.validate()?;
        Ok(Self {
            target_plugin_id: None,
            kind: ScriptEventKind::PlayerEntityKilled {
                player_id,
                context,
                dimension: validate_contract_resource_id(dimension.as_ref())?,
                entity_id,
                entity_type: validate_contract_resource_id(entity_type.as_ref())?,
                source,
                game_mode,
            },
            origin_generation: None,
        })
    }

    /// Build a broadcast event after an authoritative entity interaction is accepted.
    #[allow(clippy::too_many_arguments)]
    pub fn try_player_entity_interacted_with_context(
        player_id: ScriptPlayerId,
        context: ScriptPlayerContext,
        dimension: impl AsRef<str>,
        entity_id: ScriptEntityId,
        entity_type: impl AsRef<str>,
        hand: ScriptInteractionHand,
        secondary_action: bool,
        game_mode: ScriptGameMode,
    ) -> Result<Self, ScriptDtoError> {
        context.validate()?;
        Ok(Self {
            target_plugin_id: None,
            kind: ScriptEventKind::PlayerEntityInteracted {
                player_id,
                context,
                dimension: validate_contract_resource_id(dimension.as_ref())?,
                entity_id,
                entity_type: validate_contract_resource_id(entity_type.as_ref())?,
                hand,
                secondary_action,
                game_mode,
            },
            origin_generation: None,
        })
    }

    /// Build a reliable player-death event after the authoritative survival commit.
    pub fn try_player_died_with_context(
        player_id: ScriptPlayerId,
        context: ScriptPlayerContext,
        dimension: impl AsRef<str>,
        game_mode: ScriptGameMode,
    ) -> Result<Self, ScriptDtoError> {
        context.validate()?;
        Ok(Self {
            target_plugin_id: None,
            kind: ScriptEventKind::PlayerDied {
                player_id,
                context,
                dimension: validate_contract_resource_id(dimension.as_ref())?,
                game_mode,
            },
            origin_generation: None,
        })
    }

    /// Build the targeted player-grave observation after the death commit.
    ///
    /// The event is addressed to the one package that declared the
    /// `death_graves` capability, so a deployment without one publishes nothing
    /// of this kind.
    #[allow(clippy::too_many_arguments)]
    pub fn try_player_grave_created_with_context(
        target_plugin_id: impl AsRef<str>,
        player_id: ScriptPlayerId,
        context: ScriptPlayerContext,
        dimension: impl AsRef<str>,
        x: i32,
        y: i32,
        z: i32,
        items: u32,
        spilled: u32,
    ) -> Result<Self, ScriptDtoError> {
        context.validate()?;
        let event = Self {
            target_plugin_id: Some(validate_target_plugin_id(target_plugin_id.as_ref())?),
            kind: ScriptEventKind::PlayerGraveCreated {
                player_id,
                context,
                dimension: validate_contract_resource_id(dimension.as_ref())?,
                x,
                y,
                z,
                items,
                spilled,
            },
            origin_generation: None,
        };
        event.validate()?;
        Ok(event)
    }

    /// Build a bounded player command event with server-authoritative context.
    pub fn try_player_command_with_context(
        target_plugin_id: impl AsRef<str>,
        player_id: ScriptPlayerId,
        context: ScriptPlayerContext,
        root: impl AsRef<str>,
        arguments: impl AsRef<str>,
    ) -> Result<Self, ScriptDtoError> {
        let target_plugin_id = validate_target_plugin_id(target_plugin_id.as_ref())?;
        let root = root.as_ref();
        let arguments = arguments.as_ref();
        validate_bounded_nonempty("player command root", root, MAX_PLAYER_COMMAND_ROOT_BYTES)?;
        validate_bounded_value(
            "player command arguments",
            arguments,
            MAX_SCRIPT_CHAT_MESSAGE_BYTES,
        )?;
        context.validate()?;
        Ok(Self {
            target_plugin_id: Some(target_plugin_id),
            kind: ScriptEventKind::PlayerCommand {
                player_id,
                username: context.username().to_owned(),
                root: root.to_owned(),
                arguments: arguments.to_owned(),
                context,
            },
            origin_generation: None,
        })
    }
}
