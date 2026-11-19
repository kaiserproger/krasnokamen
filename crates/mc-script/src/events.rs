//! Event accessors: origin generation, routing target and subscription name.

use super::*;

mod client;
mod inventory;
mod kind;
mod operation;
mod payload;
mod player;
mod server;
mod validate;
mod world;
mod zones;

pub use kind::{ScriptClientViewFailure, ScriptEventKind};

impl ScriptEvent {
    /// Keep the registration that issued a request even when its owner replies
    /// after a deployment reload. Unsolicited events use admission-time routing.
    pub(crate) fn with_origin_generation(mut self, generation: Option<u64>) -> Self {
        self.origin_generation = generation;
        self
    }

    /// Return the plugin id for an event that must not be broadcast to other runtimes.
    pub fn target_plugin_id(&self) -> Option<&str> {
        self.target_plugin_id.as_deref()
    }

    /// Return the immutable event kind.
    pub fn kind(&self) -> &ScriptEventKind {
        &self.kind
    }

    /// Return the stable manifest subscription name for this event.
    pub fn event_name(&self) -> &'static str {
        match self.kind {
            ScriptEventKind::ServerStarted => "server.started",
            ScriptEventKind::ServerStopping { .. } => "server.stopping",
            ScriptEventKind::PlayerJoined { .. } => "player.joined",
            ScriptEventKind::PlayerLeft { .. } => "player.left",
            ScriptEventKind::PlayerChat { .. } => "player.chat",
            ScriptEventKind::PlayerBlockBroken { .. } => "player.block_broken",
            ScriptEventKind::PlayerBlockPlaced { .. } => "player.block_placed",
            ScriptEventKind::PlayerItemCrafted { .. } => "player.item_crafted",
            ScriptEventKind::PlayerItemPickedUp { .. } => "player.item_picked_up",
            ScriptEventKind::PlayerEntityKilled { .. } => "player.entity_killed",
            ScriptEventKind::PlayerEntityInteracted { .. } => "player.entity_interacted",
            ScriptEventKind::PlayerDied { .. } => "player.died",
            ScriptEventKind::PlayerGraveCreated { .. } => "player.grave_created",
            ScriptEventKind::PlayerCommand { .. } => "player.command",
            ScriptEventKind::ServerTick { .. } => "server.tick",
            ScriptEventKind::PluginStorageGetResult { .. } => "plugin.storage.get_result",
            ScriptEventKind::PluginStorageCasResult { .. } => "plugin.storage.cas_result",
            ScriptEventKind::PluginStorageDeleteResult { .. } => "plugin.storage.delete_result",
            ScriptEventKind::InventoryMenuClicked { .. } => "inventory.menu.clicked",
            ScriptEventKind::InventoryStorageTransactionResult { .. } => {
                "inventory.storage_transaction.result"
            }
            ScriptEventKind::OperationResult { .. } => "operation.result",
            ScriptEventKind::PlayerInventoryTransactionResult { .. } => {
                "player.inventory_transaction_result"
            }
            ScriptEventKind::PlayerZoneEntered { .. } => "player.zone_entered",
            ScriptEventKind::PlayerZoneExited { .. } => "player.zone_exited",
            ScriptEventKind::ZoneCommandResult { .. } => "zone.command_result",
            ScriptEventKind::PlayerTeleportResult { .. } => "player.teleport_result",
            ScriptEventKind::WorldTimeSetResult { .. } => "world.time_set_result",
            ScriptEventKind::WorldBlockSetResult { .. } => "world.block_set_result",
            ScriptEventKind::LoaderItemGrantResult { .. } => "loader.item_grant_result",
            ScriptEventKind::EntitySpawnResult { .. } => "entity.spawn_result",
            ScriptEventKind::EntityDamageResult { .. } => "entity.damage_result",
            ScriptEventKind::OnlinePlayersResult { .. } => "player.online_result",
            ScriptEventKind::LoaderViewRequest { .. } => "loader.view_request",
            ScriptEventKind::LoaderViewAction { .. } => "loader.view_action",
            ScriptEventKind::ClientViewOpened { .. } => "loader.view_opened",
            ScriptEventKind::ClientSelectionStarted { .. } => "loader.selection_started",
            ScriptEventKind::ClientBrand { .. } => "player.client_brand",
            ScriptEventKind::CustomPayload { .. } => "player.custom_payload",
        }
    }
}
