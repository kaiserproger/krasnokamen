//! Targeted inventory menu click and transaction result snapshots.

use crate::*;

impl ScriptEvent {
    /// Build a targeted click event for one server-owned inventory menu.
    pub(crate) fn inventory_menu_clicked(
        target_plugin_id: impl AsRef<str>,
        player_id: ScriptPlayerId,
        context: ScriptPlayerContext,
        menu: &ScriptInventoryMenu,
        slot: u8,
        click: ScriptInventoryClick,
    ) -> Result<Self, ScriptDtoError> {
        if usize::from(slot) >= MAX_INVENTORY_MENU_SLOTS {
            return Err(ScriptDtoError::InvalidBounds);
        }
        Ok(Self {
            target_plugin_id: Some(validate_target_plugin_id(target_plugin_id.as_ref())?),
            kind: ScriptEventKind::InventoryMenuClicked {
                player_id,
                context,
                menu_id: menu.id().to_owned(),
                slot,
                click,
            },
            origin_generation: None,
        })
    }

    /// Build a targeted completion event for an atomic inventory and storage request.
    pub(crate) fn inventory_storage_transaction_result(
        target_plugin_id: impl AsRef<str>,
        transaction: &ScriptInventoryStorageTransaction,
        committed: bool,
    ) -> Result<Self, ScriptDtoError> {
        Ok(Self {
            target_plugin_id: Some(validate_target_plugin_id(target_plugin_id.as_ref())?),
            kind: ScriptEventKind::InventoryStorageTransactionResult {
                request_id: transaction.id().to_owned(),
                committed,
            },
            origin_generation: None,
        })
    }

    /// Build the targeted result of one admitted player inventory transaction.
    pub(crate) fn player_inventory_transaction_result(
        target_plugin_id: impl AsRef<str>,
        transaction: &ScriptPlayerInventoryTransaction,
        failure: Option<ScriptPlayerInventoryFailure>,
    ) -> Result<Self, ScriptDtoError> {
        Ok(Self {
            target_plugin_id: Some(validate_target_plugin_id(target_plugin_id.as_ref())?),
            kind: ScriptEventKind::PlayerInventoryTransactionResult {
                request_id: transaction.request_id().to_owned(),
                player_id: transaction.player_id(),
                failure,
            },
            origin_generation: None,
        })
    }
}
