//! Published bound constants for the whole script boundary.

/// Maximum entity types one plugin may allow-list for spawning.
pub const MAX_SPAWN_ENTITY_TYPES: usize = 32;

/// Maximum byte length of a script-visible namespaced resource identifier.
pub const MAX_SCRIPT_RESOURCE_ID_BYTES: usize = 128;

/// Largest integer preserved exactly by the component command boundary.
pub const MAX_SCRIPT_WORLD_TIME: u64 = (1_u64 << 53) - 1;

/// Maximum raw damage accepted by one bounded plugin combat request.
pub const MAX_SCRIPT_ENTITY_DAMAGE: f32 = 1_000_000.0;

/// Maximum byte length of a plugin-scoped identifier or request correlation id.
pub const MAX_SCRIPT_ID_BYTES: usize = 64;

pub const MAX_SCRIPT_PLAYER_UUID_BYTES: usize = 64;

pub const MAX_SCRIPT_PLAYER_NAME_BYTES: usize = 16;

pub const MAX_SCRIPT_CHAT_MESSAGE_BYTES: usize = 4_096;

pub const MAX_SCRIPT_DISCONNECT_REASON_BYTES: usize = 1_024;

/// Maximum raw custom-payload body accepted through the script boundary.
pub const MAX_SCRIPT_CUSTOM_PAYLOAD_BYTES: usize = 32_768;

pub const MAX_SCRIPT_COMMAND_BATCH: usize = 32;

pub const MAX_SCRIPT_EVENT_QUEUE_CAPACITY: usize = 1_024;

pub const MAX_SCRIPT_COMMAND_QUEUE_CAPACITY: usize = 256;

/// Maximum connected-player snapshots returned by one plugin query.
pub const MAX_ONLINE_PLAYER_QUERY_LIMIT: usize = 256;

pub const MAX_PLUGIN_ID_BYTES: usize = 64;

pub const MAX_PLUGIN_DISPLAY_NAME_BYTES: usize = 128;

pub const MAX_PLUGIN_VERSION_BYTES: usize = 64;

pub const MAX_MANIFEST_EVENT_SUBSCRIPTIONS: usize = 64;

pub const MAX_MANIFEST_DEPENDENCIES: usize = 64;

pub const MAX_MANIFEST_CAPABILITIES: usize = 128;

/// Largest accepted `plugin.toml` for a component deployment.
pub const MAX_PLUGIN_MANIFEST_BYTES: usize = 64 * 1024;

/// Largest accepted `api = "MAJOR.MINOR.PATCH"` string.
pub const MAX_API_VERSION_BYTES: usize = 16;

pub const MAX_MANIFEST_PERMISSIONS: usize = 64;

pub const MAX_MANIFEST_COMMAND_PERMISSIONS: usize = 256;

pub const MAX_MANIFEST_FIELD_BYTES: usize = 128;

/// Maximum byte length of a plugin storage key.
pub const MAX_PLUGIN_STORAGE_KEY_BYTES: usize = 128;

/// Maximum byte length of a plugin storage value.
pub const MAX_PLUGIN_STORAGE_VALUE_BYTES: usize = 4_096;

/// Maximum number of server-owned menu slots a plugin may describe.
pub const MAX_INVENTORY_MENU_SLOTS: usize = 54;

/// Maximum atomic inventory and storage mutations in one plugin request.
pub const MAX_INVENTORY_STORAGE_MUTATIONS: usize = 16;

/// Maximum absolute resource count changed by one inventory delta.
pub const MAX_INVENTORY_RESOURCE_DELTA: i16 = 64;

/// Maximum byte length of a server-rendered inventory menu title.
pub const MAX_INVENTORY_MENU_TITLE_BYTES: usize = 128;

/// Maximum absolute horizontal coordinate accepted from a component command.
pub const SCRIPT_HORIZONTAL_COORDINATE_LIMIT: f64 = 30_000_000.0;

/// Maximum absolute vertical coordinate accepted from a component command.
pub const SCRIPT_VERTICAL_COORDINATE_LIMIT: f64 = 20_000_000.0;

/// Maximum byte length of one ASCII plugin player-command root.
pub const MAX_PLAYER_COMMAND_ROOT_BYTES: usize = 64;

/// Maximum number of active plugin player-command roots across the server.
pub const MAX_PLAYER_COMMAND_ROOTS: usize = 128;

/// Maximum number of exclusive custom-payload channels owned across the server.
pub const MAX_PLUGIN_PAYLOAD_CHANNELS: usize = 128;

/// Maximum command-tree nodes added by active plugin roots.
pub const MAX_PLAYER_COMMAND_TREE_NODES: usize = MAX_PLAYER_COMMAND_ROOTS * 2;

/// Player command roots reserved by Krasnokamen' built-in command parser.
pub const BUILT_IN_PLAYER_COMMAND_ROOTS: &[&str] = &[
    "debug",
    "defaultgamemode",
    "gamemode",
    "gamerule",
    "give",
    "kill",
    "save-all",
    "settlement",
    "status",
    "stop",
    "summon",
    "teleport",
    "time",
    "tp",
];

/// Maximum player uuids one protected zone's allow-list may name.
pub const MAX_SCRIPT_ZONE_ACTORS: usize = 32;
