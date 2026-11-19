//! Script-visible event variants and the view-failure category.

use crate::*;

/// Why a view lifecycle request was refused before it reached the owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScriptClientViewFailure {
    /// The owner does not own the requested view or lacks the permission pair.
    Refused,
    /// The target player has no live Loader session.
    PlayerUnavailable,
    /// The referenced instance is not open (or already retired).
    UnknownInstance,
    /// The caller's expected revision no longer matches the live instance.
    StaleRevision,
    /// The model exceeded a hard bound.
    TooLarge,
}

impl ScriptClientViewFailure {
    #[must_use]
    pub const fn contract_name(self) -> &'static str {
        match self {
            Self::Refused => "refused",
            Self::PlayerUnavailable => "player_unavailable",
            Self::UnknownInstance => "unknown_instance",
            Self::StaleRevision => "stale_revision",
            Self::TooLarge => "too_large",
        }
    }
}

/// Script-visible event variants.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScriptEventKind {
    ServerStarted,
    ServerStopping {
        reason: String,
    },
    PlayerJoined {
        player_id: ScriptPlayerId,
        username: String,
        context: ScriptPlayerContext,
    },
    PlayerLeft {
        player_id: ScriptPlayerId,
        reason: String,
    },
    PlayerChat {
        player_id: ScriptPlayerId,
        message: String,
        context: ScriptPlayerContext,
    },
    PlayerBlockBroken {
        player_id: ScriptPlayerId,
        context: ScriptPlayerContext,
        dimension: String,
        block_id: String,
        x: i32,
        y: i32,
        z: i32,
        game_mode: ScriptGameMode,
    },
    PlayerBlockPlaced {
        player_id: ScriptPlayerId,
        context: ScriptPlayerContext,
        dimension: String,
        block_id: String,
        x: i32,
        y: i32,
        z: i32,
        game_mode: ScriptGameMode,
    },
    PlayerItemCrafted {
        player_id: ScriptPlayerId,
        context: ScriptPlayerContext,
        dimension: String,
        item_id: String,
        count: u64,
        craft_count: u32,
        source: ScriptCraftingSource,
        game_mode: ScriptGameMode,
    },
    PlayerItemPickedUp {
        player_id: ScriptPlayerId,
        context: ScriptPlayerContext,
        dimension: String,
        item_id: String,
        count: u64,
        source: ScriptItemPickupSource,
        game_mode: ScriptGameMode,
    },
    PlayerEntityKilled {
        player_id: ScriptPlayerId,
        context: ScriptPlayerContext,
        dimension: String,
        entity_id: ScriptEntityId,
        entity_type: String,
        source: ScriptEntityKillSource,
        game_mode: ScriptGameMode,
    },
    PlayerEntityInteracted {
        player_id: ScriptPlayerId,
        context: ScriptPlayerContext,
        dimension: String,
        entity_id: ScriptEntityId,
        entity_type: String,
        hand: ScriptInteractionHand,
        secondary_action: bool,
        game_mode: ScriptGameMode,
    },
    PlayerDied {
        player_id: ScriptPlayerId,
        context: ScriptPlayerContext,
        dimension: String,
        game_mode: ScriptGameMode,
    },
    PlayerGraveCreated {
        player_id: ScriptPlayerId,
        context: ScriptPlayerContext,
        dimension: String,
        x: i32,
        y: i32,
        z: i32,
        items: u32,
        spilled: u32,
    },
    PlayerCommand {
        player_id: ScriptPlayerId,
        username: String,
        root: String,
        arguments: String,
        context: ScriptPlayerContext,
    },
    ServerTick {
        tick: u64,
    },
    PluginStorageGetResult {
        request_id: String,
        key: String,
        value: Option<String>,
        version: Option<u64>,
        failure: Option<ScriptPluginStorageFailure>,
    },
    PluginStorageCasResult {
        request_id: String,
        key: String,
        applied: bool,
        version: Option<u64>,
        failure: Option<ScriptPluginStorageFailure>,
    },
    PluginStorageDeleteResult {
        request_id: String,
        key: String,
        deleted: bool,
        version: Option<u64>,
        failure: Option<ScriptPluginStorageFailure>,
    },
    InventoryMenuClicked {
        player_id: ScriptPlayerId,
        context: ScriptPlayerContext,
        menu_id: String,
        slot: u8,
        click: ScriptInventoryClick,
    },
    InventoryStorageTransactionResult {
        request_id: String,
        committed: bool,
    },
    OperationResult {
        request_id: String,
        operation_id: Option<String>,
        outcome: ScriptOperationOutcome,
    },
    PlayerInventoryTransactionResult {
        request_id: String,
        player_id: ScriptPlayerId,
        failure: Option<ScriptPlayerInventoryFailure>,
    },
    PlayerZoneEntered {
        player_id: ScriptPlayerId,
        context: ScriptPlayerContext,
        zone_id: String,
    },
    PlayerZoneExited {
        player_id: ScriptPlayerId,
        context: ScriptPlayerContext,
        zone_id: String,
    },
    ZoneCommandResult {
        zone_id: String,
        accepted: bool,
    },
    PlayerTeleportResult {
        request_id: String,
        player_id: ScriptPlayerId,
        position: ScriptPosition,
        failure: Option<ScriptPlayerTeleportFailure>,
    },
    WorldTimeSetResult {
        request_id: String,
        world_time: u64,
        failure: Option<ScriptWorldTimeSetFailure>,
    },
    WorldBlockSetResult {
        request_id: String,
        dimension: String,
        block_id: String,
        x: i32,
        y: i32,
        z: i32,
        applied: bool,
        failure: Option<ScriptWorldBlockSetFailure>,
    },
    LoaderItemGrantResult {
        request_id: String,
        player_id: ScriptPlayerId,
        block_id: String,
        count: u8,
        failure: Option<ScriptLoaderItemGrantFailure>,
    },
    EntitySpawnResult {
        request_id: String,
        player_id: ScriptPlayerId,
        entity_type: String,
        position: ScriptPosition,
        failure: Option<ScriptEntitySpawnFailure>,
    },
    EntityDamageResult {
        request_id: String,
        entity_id: ScriptEntityId,
        amount_bits: u32,
        health_bits: Option<u32>,
        killed: bool,
        failure: Option<ScriptEntityDamageFailure>,
    },
    OnlinePlayersResult {
        request_id: String,
        players: Vec<ScriptOnlinePlayerSnapshot>,
        truncated: bool,
    },
    /// One explicit key-driven view request admitted for the owning plugin.
    LoaderViewRequest {
        player_id: ScriptPlayerId,
        request_kind: ScriptClientViewKey,
    },
    /// One admitted client view action after the server re-read the instance.
    LoaderViewAction {
        player_id: ScriptPlayerId,
        view_instance_id: String,
        view_revision: u64,
        action_id: String,
        action_sequence: u64,
        fields: Vec<ScriptClientViewField>,
        selection_token: Option<String>,
    },
    /// Outcome of one `open_client_view` request.
    ClientViewOpened {
        request_id: String,
        player_id: ScriptPlayerId,
        view_instance_id: Option<String>,
        revision: Option<u64>,
        failure: Option<ScriptClientViewFailure>,
    },
    /// Outcome of one `begin_client_selection` request.
    ClientSelectionStarted {
        request_id: String,
        player_id: ScriptPlayerId,
        selection_context_id: Option<String>,
        expires_at_tick: Option<u64>,
        failure: Option<ScriptClientViewFailure>,
    },
    ClientBrand {
        player_id: ScriptPlayerId,
        brand: String,
    },
    CustomPayload {
        player_id: ScriptPlayerId,
        phase: ScriptProtocolPhase,
        channel: String,
        payload: Vec<u8>,
    },
}
