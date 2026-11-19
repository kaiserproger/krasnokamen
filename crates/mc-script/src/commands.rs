//! Outbound command requests and the bounded batch leaving a runtime.

use super::*;

use std::sync::Arc;

/// Outbound command requests emitted by component code.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScriptCommand {
    /// A request emitted by a component and attested by its host. Guest code has
    /// no API for constructing `ScriptCommandProvenance` or selecting another
    /// plugin id.
    HostAttached {
        provenance: ScriptCommandProvenance,
        request: Arc<ScriptCommand>,
    },
    SendChatMessage {
        player_id: ScriptPlayerId,
        message: String,
    },
    BroadcastChatMessage {
        message: String,
    },
    DisconnectPlayer {
        player_id: ScriptPlayerId,
        reason: String,
    },
    UnlockSession {
        player_id: ScriptPlayerId,
    },
    SpawnEntity {
        request_id: String,
        actor: ScriptPlayerId,
        entity_type: String,
        position: ScriptPosition,
    },
    DamageEntity {
        request: ScriptEntityDamageRequest,
    },
    PluginStorageGet {
        request: ScriptPluginStorageGetRequest,
    },
    PluginStorageCompareAndSwap {
        request: ScriptPluginStorageCompareAndSwapRequest,
    },
    PluginStorageDelete {
        request: ScriptPluginStorageDeleteRequest,
    },
    Operation {
        request: ScriptOperationRequest,
    },
    OpenInventoryMenu {
        player_id: ScriptPlayerId,
        menu: ScriptInventoryMenu,
    },
    OpenClientView {
        request: ScriptClientViewOpen,
    },
    PresentClientView {
        request: ScriptClientViewPresent,
    },
    CloseClientView {
        player_id: ScriptPlayerId,
        view_instance_id: String,
    },
    BeginClientSelection {
        request_id: String,
        player_id: ScriptPlayerId,
        view_instance_id: String,
        view_revision: u64,
        action_id: String,
        constraints: ScriptClientSelectionConstraints,
    },
    CancelClientSelection {
        player_id: ScriptPlayerId,
        selection_context_id: String,
    },
    ClientSound {
        player_id: ScriptPlayerId,
        sound: ScriptClientSound,
    },
    GrantLoaderBlockItem {
        request: ScriptLoaderItemGrantRequest,
    },
    CloseInventoryMenu {
        player_id: ScriptPlayerId,
        menu_id: String,
    },
    InventoryStorageTransaction {
        transaction: ScriptInventoryStorageTransaction,
    },
    PlayerInventoryTransaction {
        transaction: ScriptPlayerInventoryTransaction,
    },
    UpsertZone {
        zone: ScriptAxisAlignedZone,
    },
    RemoveZone {
        zone_id: String,
    },
    TeleportPlayer {
        request: ScriptPlayerTeleportRequest,
    },
    SetWorldTime {
        request: ScriptWorldTimeSetRequest,
    },
    SetWorldBlock {
        request: ScriptWorldBlockSetRequest,
    },
    ListOnlinePlayers {
        request: ScriptOnlinePlayersRequest,
    },
    SendCustomPayload {
        player_id: ScriptPlayerId,
        channel: String,
        payload: Vec<u8>,
    },
}

/// Bounded state returned when the script event queue cannot accept an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScriptQueueError {
    Full,
    Closed,
}

/// Rejection returned by the public raw-command submission path.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScriptCommandSubmissionError {
    ProvenanceRejected,
    PermissionDenied {
        capability: ScriptCommandCapabilityKind,
    },
    InvalidCommand {
        error: ScriptDtoError,
    },
    QueueFull,
    QueueClosed,
}

/// Why a plugin's already-validated batch did not reach the command queue.
///
/// The batch comes back inside the error, so a caller can report exactly which
/// commands were refused instead of losing them.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScriptBatchSubmissionError {
    Full(CommandBatch),
    Closed(CommandBatch),
    Rejected {
        batch: CommandBatch,
        error: CommandBatchError,
    },
}

/// Why a candidate deployment did not reach the live boundary.
///
/// Only the trusted host sees this: the commit path keeps readiness, route
/// ownership, the caller's swap and command delivery in one order for every
/// caller, so a refused reload leaves the running generation untouched.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScriptReloadCommitError {
    QueueFull,
    QueueClosed,
    Rejected { error: CommandBatchError },
    Ownership { error: ScriptRouteRegistrationError },
}

/// Error returned when a command batch cannot accept another command.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CommandBatchError {
    Full {
        limit: NonZeroUsize,
    },
    PermissionDenied {
        capability: ScriptCommandCapabilityKind,
    },
    ProvenanceRejected,
    InvalidCommand {
        error: ScriptDtoError,
    },
    AdmissionUnavailable,
}

impl CommandBatch {
    /// Create an empty command batch with a fixed command count limit.
    pub fn new(limit: NonZeroUsize) -> Self {
        let limit = NonZeroUsize::new(limit.get().min(MAX_SCRIPT_COMMAND_BATCH))
            .expect("script command batch limit is non-zero");
        Self {
            limit,
            commands: Vec::new(),
        }
    }

    /// Return the maximum number of commands this batch may contain.
    pub fn limit(&self) -> NonZeroUsize {
        self.limit
    }

    /// Return queued commands as an immutable slice.
    pub fn commands(&self) -> &[ScriptCommand] {
        &self.commands
    }

    /// Consume this batch and return the queued commands.
    pub fn into_commands(self) -> Vec<ScriptCommand> {
        self.commands
    }

    /// Try to append one command without exceeding the batch limit.
    pub fn try_push(&mut self, command: ScriptCommand) -> Result<(), CommandBatchError> {
        if matches!(command, ScriptCommand::HostAttached { .. }) {
            return Err(CommandBatchError::ProvenanceRejected);
        }
        command
            .validate_contract()
            .map_err(|error| CommandBatchError::InvalidCommand { error })?;
        if let Some(capability) = command.required_capability_kind() {
            return Err(CommandBatchError::PermissionDenied { capability });
        }
        self.try_push_validated(command)
    }

    fn try_push_validated(&mut self, command: ScriptCommand) -> Result<(), CommandBatchError> {
        if self.commands.len() >= self.limit.get() {
            return Err(CommandBatchError::Full { limit: self.limit });
        }
        self.commands.push(command);
        Ok(())
    }

    /// Try to append one command if granted by the host capability allow-list.
    pub fn try_push_authorized(
        &mut self,
        command: ScriptCommand,
        capabilities: &CommandCapabilities,
    ) -> Result<(), CommandBatchError> {
        if matches!(command, ScriptCommand::HostAttached { .. }) {
            return Err(CommandBatchError::ProvenanceRejected);
        }
        command
            .validate_contract()
            .map_err(|error| CommandBatchError::InvalidCommand { error })?;
        if let Some(capability) = command.required_capability()
            && !capabilities.allows(capability)
        {
            return Err(CommandBatchError::PermissionDenied {
                capability: capability.kind(),
            });
        }
        self.try_push_validated(command)
    }
}
