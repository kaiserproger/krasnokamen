//! # mc-script
//!
//! Runtime-independent script contracts: the server-owned boundary, immutable
//! event snapshots entering runtimes, bounded command batches leaving them, and
//! the plugin metadata both sides share.
//!
//! No VM lives in this crate. A host crate owns guest execution and reaches this
//! boundary through the trusted host methods below: it submits events, takes
//! control input (a reload envelope whose payload only the host interprets), and
//! commits a candidate deployment through the same route and command admission
//! every other plugin command takes.

use std::num::NonZeroUsize;

use tokio::sync::mpsc;

#[cfg(test)]
use std::sync::Arc;

mod command_admission;
pub use command_admission::{
    AdmittedScriptCommand, HostCommandAdmission, ScriptCommandAcceptanceError,
    ScriptCommandProvenance, ScriptPluginTarget,
};

mod client_view;
pub use client_view::{
    MAX_CLIENT_VIEW_ACTIONS, MAX_CLIENT_VIEW_CELL_BYTES, MAX_CLIENT_VIEW_DENY_REASON_BYTES,
    MAX_CLIENT_VIEW_EXACT_INTEGER, MAX_CLIENT_VIEW_FIELDS, MAX_CLIENT_VIEW_ID_BYTES,
    MAX_CLIENT_VIEW_MARKER_RADIUS, MAX_CLIENT_VIEW_MARKERS, MAX_CLIENT_VIEW_PACKET_BYTES,
    MAX_CLIENT_VIEW_RANGE_LIMIT, MAX_CLIENT_VIEW_REASON_BYTES, MAX_CLIENT_VIEW_RESOURCES,
    MAX_CLIENT_VIEW_ROWS, MAX_CLIENT_VIEW_SELECTION_TTL_TICKS, MAX_CLIENT_VIEW_TABS,
    MAX_CLIENT_VIEW_TEXT_BYTES, MAX_CLIENT_VIEW_TITLE_BYTES, ScriptClientSelectionBegin,
    ScriptClientSelectionConstraints, ScriptClientViewAction, ScriptClientViewField,
    ScriptClientViewFieldValue, ScriptClientViewFormation, ScriptClientViewKey,
    ScriptClientViewMarker, ScriptClientViewMarkerPosition, ScriptClientViewModel,
    ScriptClientViewOpen, ScriptClientViewPresent, ScriptClientViewResourceEntry,
    ScriptClientViewRow, ScriptClientViewTab,
};
use command_admission::HostAdmissionLedger;

mod command_permissions;
pub use command_permissions::{
    MAX_COMMAND_PERMISSION_ENTRIES, MAX_COMMAND_PERMISSION_ENTRY_BYTES,
    MAX_COMMAND_PERMISSION_NODE_BYTES, ScriptCommandPermissionError, ScriptCommandPermissions,
};

mod client_sound;
pub use client_sound::{ScriptClientSound, ScriptClientSoundPlayback};

mod commit_events;
pub use commit_events::{
    ScriptCommitEnqueueError, ScriptCommitEventMonitor, ScriptCommitEventOutbox,
    ScriptCommitEventOutboxSnapshot, ScriptCommitEventReceiver,
};

mod inventory_operations;
pub use inventory_operations::{
    MAX_INVENTORY_RESOURCE_TYPES, MAX_INVENTORY_WORK_PORTIONS, MAX_NPC_CARRY_SLOTS,
    MAX_NPC_EQUIPMENT_SLOTS, MAX_OWNED_INVENTORY_ITEM_BUDGET, MAX_OWNED_INVENTORY_SLOTS,
    MAX_OWNED_INVENTORY_TRANSFERS, ScriptInventoryEnchantment, ScriptInventoryEndpoint,
    ScriptInventoryExpectedRevision, ScriptInventoryFence, ScriptInventoryItem,
    ScriptInventoryMaterial, ScriptInventoryPlacementSlot, ScriptInventoryReservationQuantity,
    ScriptInventoryReservationSnapshot, ScriptInventoryResourcePlan, ScriptInventorySlot,
    ScriptInventoryStewEffect, ScriptInventoryWorkPortion, ScriptOwnedInventoryOperation,
    ScriptOwnedInventoryResult, ScriptOwnedInventorySnapshot, ScriptOwnedItemTransfer,
};

mod operations;
pub use operations::{
    MAX_STORAGE_SCAN_PAGE, ScriptOperation, ScriptOperationFailure, ScriptOperationOutcome,
    ScriptOperationPayload, ScriptOperationRequest, ScriptOperationState,
    ScriptPluginStorageVersionGuard, ScriptStorageChange, ScriptStorageEntry,
    validate_plugin_storage_version_guards,
};

mod native_birth_operations;
pub use native_birth_operations::{ScriptNativeBirthObservation, ScriptNativeBirthOrigin};

mod settlement_operations;
pub use settlement_operations::{
    MAX_HOUSEHOLD_ID_BYTES, MAX_HOUSEHOLD_POLICY_BYTES, MAX_SETTLEMENT_DISTRICT_BUILDINGS,
    MAX_SETTLEMENT_DISTRICT_REASON_BYTES, MAX_SETTLEMENT_NAME_BYTES, MAX_SETTLEMENT_PAGE,
    MAX_SETTLEMENT_QUERY_NAMES, ScriptSettlementBuildingRef, ScriptSettlementDistrictSnapshot,
    ScriptSettlementHouseholdSnapshot, ScriptSettlementOperation, ScriptSettlementPage,
    ScriptSettlementQuery, ScriptSettlementResult, ScriptSettlementSnapshot,
};

mod settlement_economy;
pub use settlement_economy::{
    MAX_SETTLEMENT_CARAVAN_QUANTITY, MAX_SETTLEMENT_CARAVAN_REASON_BYTES,
    MAX_SETTLEMENT_CARAVAN_TICKS, MAX_SETTLEMENT_CARAVAN_WAYPOINTS, ScriptSettlementEconomyAction,
    ScriptSettlementEconomySnapshot, ScriptSettlementEmploymentSnapshot,
    ScriptSettlementInventoryEndpoint, ScriptSettlementMandateSnapshot,
    ScriptSettlementProductionSnapshot, ScriptSettlementQuoteSnapshot,
    ScriptSettlementShipmentSnapshot,
};

mod npc_operations;
pub use npc_operations::{
    MAX_NPC_CARRIED_ITEMS, MAX_NPC_CLAIM_DISTANCE, MAX_NPC_ENTITY_UUID_BYTES,
    MAX_NPC_GENERATION_ID_BYTES, MAX_NPC_HANDLE_BYTES, MAX_NPC_PAGE, MAX_NPC_QUERY_HANDLES,
    ScriptNpcItemSummary, ScriptNpcLifecycle, ScriptNpcLoadedState, ScriptNpcOperation,
    ScriptNpcResult, ScriptNpcSnapshot, npc_entity_uuid, npc_handle_for_entity,
    npc_handle_for_generation, validate_entity_uuid, validate_generation_id, validate_npc_handle,
};

mod npc_order_operations;
pub use npc_order_operations::{
    MAX_COMBAT_EVENTS, MAX_ENGAGEMENT_RADIUS, MAX_FORMATION_SPACING, MAX_NPC_ORDER_HANDLES,
    MAX_ORDER_AFFILIATIONS, MAX_ORDER_TARGETS, MAX_ORDER_WAYPOINTS, MAX_RECIPE_ID_BYTES,
    MAX_SCRIPT_BLOCK_COORDINATE, MAX_TARGET_REF_BYTES, MAX_WORK_AREA_AXIS, MAX_WORK_RECEIPTS,
    MAX_WORK_UNITS, MIN_ENGAGEMENT_RADIUS, MIN_FORMATION_SPACING, MIN_ORDER_WAYPOINTS,
    ScriptBlockPosition, ScriptCombatEvent, ScriptCraftReceipt, ScriptCraftStack,
    ScriptCraftToolWear, ScriptEngagementPolicy, ScriptFormation, ScriptFormationKind,
    ScriptHostileCategory, ScriptItemChange, ScriptNpcCombatControl, ScriptNpcControlSnapshot,
    ScriptNpcItemUseControl, ScriptNpcOrder, ScriptNpcOrderOperation, ScriptNpcOrderResult,
    ScriptNpcPendingWork, ScriptNpcTargetObservation, ScriptNpcWorkOrder, ScriptOrderMemberOutcome,
    ScriptOrderMemberState, ScriptOrderTarget, ScriptOrderTargetRef, ScriptWorkArea,
    ScriptWorkAssignment, ScriptWorkPauseReason, ScriptWorkState, validate_recipe,
    validate_work_area_axis, validate_work_units,
};

mod npc_route_operations;
pub use npc_route_operations::{
    MAX_NPC_ROUTE_REF_BYTES, MAX_NPC_ROUTE_RESERVATIONS, MAX_NPC_ROUTE_SEGMENT_CELLS,
    MAX_NPC_ROUTE_TICKS, MAX_NPC_ROUTE_WAYPOINTS, ScriptNpcExecutionPermit, ScriptNpcExecutorMode,
    ScriptNpcRouteAdmission, ScriptNpcRouteArrival, ScriptNpcRouteBlockReason,
    ScriptNpcRouteHandoff, ScriptNpcRouteOperation, ScriptNpcRouteResult, ScriptNpcRouteState,
    ScriptNpcRouteStatus, validate_route_ref,
};

mod npc_death_receipts;
pub use npc_death_receipts::{
    MAX_NPC_DEATH_LOOT, ScriptNpcDeathCargoReceipt, ScriptNpcDeathInventory, ScriptNpcDeathLoot,
};

mod world_structure_operations;
pub use world_structure_operations::{
    MAX_BLUEPRINT_FOOTPRINT_AXIS, MAX_BLUEPRINT_ID_BYTES, MAX_STRUCTURE_ACTIVE_PER_PLUGIN,
    MAX_STRUCTURE_ID_BYTES, MAX_STRUCTURE_RESOURCE_TYPES, MAX_STRUCTURE_STAGES,
    MAX_SURVEY_BOUNDS_AXIS, MAX_SURVEY_TAGS, MAX_SURVEY_TOKEN_BYTES, MAX_WORKSITE_POINTS,
    MAX_WORLD_BLOCK_PROPERTIES, MAX_WORLD_BLOCK_PROPERTY_BYTES, MAX_WORLD_BLOCK_QUERY_POSITIONS,
    MAX_WORLD_BOUNDS_AXIS, MAX_WORLD_COMMIT_PORTION, MAX_WORLD_FEATURE_AXIS,
    MAX_WORLD_FEATURE_BLOCKS, MAX_WORLD_FEATURE_ENTITIES, MAX_WORLD_FEATURE_ID_BYTES,
    MAX_WORLD_FEATURE_PAGE, ScriptChunkAvailability, ScriptStructureCheck, ScriptStructureMaterial,
    ScriptStructureReceipt, ScriptStructureSnapshot, ScriptStructureStagePlan,
    ScriptStructureState, ScriptSurveySnapshot, ScriptWorksiteCheck, ScriptWorksiteFailure,
    ScriptWorksiteInteractionPoint, ScriptWorksitePointFailure, ScriptWorksiteRequest,
    ScriptWorksiteSnapshot, ScriptWorldBlockFacts, ScriptWorldBlockProperty, ScriptWorldBlockQuery,
    ScriptWorldBounds, ScriptWorldFeature, ScriptWorldFeatureBlock, ScriptWorldFeaturePage,
    ScriptWorldStructureOperation, ScriptWorldStructureResult,
};

mod plugin_metadata;
mod route_authority;
pub use plugin_metadata::{
    ClientBundle, ClientBundleDiscovery, ClientContentKind, ClientLoader, ClientPermission,
    PluginDeployment, PluginDiscovery, PluginWorldgenOreProfile,
};

pub mod precommit;

mod gameplay_rules;
mod manifest;
pub use gameplay_rules::{
    BiomeSpawns, ClayRule, GameplayRules, GameplayRulesError, SpawnEntry, SpawnPlacement, TreeRule,
};
pub use manifest::{
    CommandCapabilities, ScriptCommandCapability, ScriptCommandCapabilityKind,
    ScriptEventSubscription, ScriptPluginDependency, ScriptPluginDependencyRelation,
    ScriptPluginLoadPhase, ScriptPluginManifest, ScriptPluginManifestError,
    ValidatedScriptPluginManifest, validate_script_resource_id,
};

mod errors;
pub use errors::ScriptDtoError;

mod version;
pub use version::{
    COMPONENT_PLUGIN_API_VERSION, ScriptApiVersion, VERSION, supports_plugin_api_version,
};

mod limits;
pub use limits::{
    BUILT_IN_PLAYER_COMMAND_ROOTS, MAX_API_VERSION_BYTES, MAX_INVENTORY_MENU_SLOTS,
    MAX_INVENTORY_MENU_TITLE_BYTES, MAX_INVENTORY_RESOURCE_DELTA, MAX_INVENTORY_STORAGE_MUTATIONS,
    MAX_MANIFEST_CAPABILITIES, MAX_MANIFEST_COMMAND_PERMISSIONS, MAX_MANIFEST_DEPENDENCIES,
    MAX_MANIFEST_EVENT_SUBSCRIPTIONS, MAX_MANIFEST_FIELD_BYTES, MAX_MANIFEST_PERMISSIONS,
    MAX_ONLINE_PLAYER_QUERY_LIMIT, MAX_PLAYER_COMMAND_ROOT_BYTES, MAX_PLAYER_COMMAND_ROOTS,
    MAX_PLAYER_COMMAND_TREE_NODES, MAX_PLUGIN_DISPLAY_NAME_BYTES, MAX_PLUGIN_ID_BYTES,
    MAX_PLUGIN_MANIFEST_BYTES, MAX_PLUGIN_PAYLOAD_CHANNELS, MAX_PLUGIN_STORAGE_KEY_BYTES,
    MAX_PLUGIN_STORAGE_VALUE_BYTES, MAX_PLUGIN_VERSION_BYTES, MAX_SCRIPT_CHAT_MESSAGE_BYTES,
    MAX_SCRIPT_COMMAND_BATCH, MAX_SCRIPT_COMMAND_QUEUE_CAPACITY, MAX_SCRIPT_CUSTOM_PAYLOAD_BYTES,
    MAX_SCRIPT_DISCONNECT_REASON_BYTES, MAX_SCRIPT_ENTITY_DAMAGE, MAX_SCRIPT_EVENT_QUEUE_CAPACITY,
    MAX_SCRIPT_ID_BYTES, MAX_SCRIPT_PLAYER_NAME_BYTES, MAX_SCRIPT_PLAYER_UUID_BYTES,
    MAX_SCRIPT_RESOURCE_ID_BYTES, MAX_SCRIPT_WORLD_TIME, MAX_SCRIPT_ZONE_ACTORS,
    MAX_SPAWN_ENTITY_TYPES, SCRIPT_HORIZONTAL_COORDINATE_LIMIT, SCRIPT_VERTICAL_COORDINATE_LIMIT,
};

mod identity;
pub use identity::{ScriptEntityId, ScriptPlayerId};

mod player;
pub use player::{
    PlayerCommandAdmission, PreparedPlayerCommand, ScriptOnlinePlayerSnapshot,
    ScriptOnlinePlayersRequest, ScriptPlayerTeleportFailure, ScriptPlayerTeleportRequest,
    ScriptPosition,
};

mod world;
pub use world::{
    ScriptEntityDamageFailure, ScriptEntityDamageRequest, ScriptEntitySpawnFailure,
    ScriptLoaderItemGrantFailure, ScriptLoaderItemGrantRequest, ScriptWorldBlockSetFailure,
    ScriptWorldBlockSetRequest, ScriptWorldTimeSetFailure, ScriptWorldTimeSetRequest,
};

mod zones;
pub use zones::{ScriptAxisAlignedZone, ScriptZoneProtection};

mod inventory;
pub use inventory::{
    ScriptInventoryClick, ScriptInventoryMenu, ScriptInventoryMenuItem, ScriptInventoryMenuSlot,
    ScriptInventoryResourceDelta, ScriptInventoryStorageTransaction, ScriptPlayerInventoryFailure,
    ScriptPlayerInventoryTransaction,
};

mod storage;
pub use storage::{
    ScriptPluginStorageCompareAndSwapRequest, ScriptPluginStorageDeleteRequest,
    ScriptPluginStorageFailure, ScriptPluginStorageGetRequest, ScriptStorageMutation,
};

mod event_sources;
pub use event_sources::{
    ScriptCraftingSource, ScriptEntityKillSource, ScriptGameMode, ScriptInteractionHand,
    ScriptItemPickupSource, ScriptProtocolPhase,
};

mod commands;
pub use commands::{
    CommandBatchError, ScriptBatchSubmissionError, ScriptCommand, ScriptCommandSubmissionError,
    ScriptQueueError, ScriptReloadCommitError,
};

mod host_control;
pub(crate) use host_control::EventTargetGeneration;
pub use host_control::ScriptHostInput;

mod plugin_package;
pub use plugin_package::PluginPackage;

mod validation;
pub(crate) use validation::{
    check_contract_resource_id, normalize_player_uuid, validate_bounded_nonempty,
    validate_bounded_value, validate_player_identity, validate_plugin_storage_key,
    validate_plugin_storage_value, validate_script_id, validate_target_plugin_id,
};
pub use validation::{validate_contract_resource_id, validate_script_id_value};

mod events;
pub use events::{ScriptClientViewFailure, ScriptEventKind};

use manifest::RequiredCommandCapability;
#[cfg(test)]
use manifest::is_supported_event_name;

#[cfg(test)]
mod custom_payload_tests;
#[cfg(test)]
mod entity_interaction_tests;
#[cfg(test)]
mod entity_kill_tests;
#[cfg(test)]
mod host_control_tests;
#[cfg(test)]
mod inventory_operations_tests;
#[cfg(test)]
mod item_pickup_tests;
#[cfg(test)]
mod npc_operations_tests;
#[cfg(test)]
mod npc_order_operations_tests;
#[cfg(test)]
mod player_death_tests;
#[cfg(test)]
mod player_inventory_tests;
#[cfg(test)]
mod player_query_tests;
#[cfg(test)]
mod player_teleport_tests;
#[cfg(test)]
mod precommit_tests;
#[cfg(test)]
mod tick_delivery_tests;
#[cfg(test)]
mod world_structure_operations_tests;

/// Immutable server-authoritative player context attached to gameplay events.
///
/// This is a point-in-time value. It deliberately contains no connection or
/// network-address data and cannot be used to query live server state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptPlayerContext {
    snapshot: Box<ScriptPlayerContextSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScriptPlayerContextSnapshot {
    uuid: String,
    username: String,
    operator: bool,
    dimension: String,
    x_bits: u64,
    y_bits: u64,
    z_bits: u64,
}

/// Immutable inbound event snapshots visible to script runtimes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptEvent {
    target_plugin_id: Option<String>,
    /// Registration that issued an asynchronous request, if this is its result.
    /// The response must not be reattributed to a replacement with the same id.
    origin_generation: Option<u64>,
    kind: ScriptEventKind,
}

/// A server event admitted to the trusted host mailbox.
///
/// The wrapped event remains the guest-visible DTO. Target registration state is
/// host-private so an event admitted for a retired plugin generation cannot be
/// forged for its replacement.
#[derive(Debug, Clone)]
pub struct ScriptHostEvent {
    event: ScriptEvent,
    target_generation: EventTargetGeneration,
}

/// Trusted handle the host queues its own control input through.
///
/// It is deliberately narrow — a host can queue a reload, not fabricate events
/// or reach the queue — and deliberately weak: the boundary drops the ability to
/// queue control when it closes its host side, so a reload after shutdown reports
/// a closed queue instead of resurrecting the host. Control input shares the one
/// bounded mailbox and its FIFO with events; there is no second queue.
#[derive(Debug, Clone)]
pub struct ScriptHostInputSender {
    sender: mpsc::WeakSender<ScriptHostInput>,
}

/// Bounded list of commands produced by one script event invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandBatch {
    limit: NonZeroUsize,
    commands: Vec<ScriptCommand>,
}

mod runtime;
pub use route_authority::{
    ScriptBoundary, ScriptHostEndpoint, ScriptRouteRegistrationError, script_boundary_pair,
};
pub use runtime::{RuntimeContext, RuntimeControls, RuntimeError, RuntimeResult, ScriptRuntime};

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
