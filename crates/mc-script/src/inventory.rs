//! Server-owned inventory menus, resource deltas and atomic player transactions.

use super::*;

use std::collections::BTreeMap;

const fn decimal_u8_len(value: u8) -> usize {
    if value >= 100 {
        3
    } else if value >= 10 {
        2
    } else {
        1
    }
}

/// One immutable item descriptor used in a server-owned inventory menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptInventoryMenuItem {
    resource_id: String,
    count: u8,
    label: Option<String>,
}

impl ScriptInventoryMenuItem {
    pub fn try_new(
        resource_id: impl AsRef<str>,
        count: u8,
        label: Option<String>,
    ) -> Result<Self, ScriptDtoError> {
        if count == 0 {
            return Err(ScriptDtoError::InvalidAmount);
        }
        if let Some(label) = &label
            && label.len() > MAX_INVENTORY_MENU_TITLE_BYTES
        {
            return Err(ScriptDtoError::ValueTooLong {
                field: "inventory menu item label",
                max_bytes: MAX_INVENTORY_MENU_TITLE_BYTES,
                actual_bytes: label.len(),
            });
        }
        Ok(Self {
            resource_id: validate_contract_resource_id(resource_id.as_ref())?,
            count,
            label,
        })
    }

    pub fn resource_id(&self) -> &str {
        &self.resource_id
    }

    pub const fn count(&self) -> u8 {
        self.count
    }

    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }
}

/// One fixed slot in a server-owned inventory menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptInventoryMenuSlot {
    index: u8,
    item: ScriptInventoryMenuItem,
}

impl ScriptInventoryMenuSlot {
    pub const fn new(index: u8, item: ScriptInventoryMenuItem) -> Self {
        Self { index, item }
    }

    pub const fn index(&self) -> u8 {
        self.index
    }

    pub fn item(&self) -> &ScriptInventoryMenuItem {
        &self.item
    }
}

/// Immutable menu description requested by a plugin and owned by the server after admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptInventoryMenu {
    id: String,
    title: String,
    slots: Vec<ScriptInventoryMenuSlot>,
}

impl ScriptInventoryMenu {
    pub fn try_new(
        id: impl AsRef<str>,
        title: impl AsRef<str>,
        slots: Vec<ScriptInventoryMenuSlot>,
    ) -> Result<Self, ScriptDtoError> {
        let title = title.as_ref();
        validate_bounded_nonempty(
            "inventory menu title",
            title,
            MAX_INVENTORY_MENU_TITLE_BYTES,
        )?;
        if slots.len() > MAX_INVENTORY_MENU_SLOTS {
            return Err(ScriptDtoError::TooManyEntries {
                field: "inventory menu slots",
                max: MAX_INVENTORY_MENU_SLOTS,
            });
        }
        let mut indexes = BTreeMap::new();
        for slot in &slots {
            if usize::from(slot.index) >= MAX_INVENTORY_MENU_SLOTS {
                return Err(ScriptDtoError::InvalidBounds);
            }
            if indexes.insert(slot.index, ()).is_some() {
                return Err(ScriptDtoError::DuplicateId {
                    field: "inventory menu slot index",
                    actual_bytes: decimal_u8_len(slot.index),
                });
            }
        }
        Ok(Self {
            id: validate_script_id(id.as_ref())?,
            title: title.to_owned(),
            slots,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn slots(&self) -> &[ScriptInventoryMenuSlot] {
        &self.slots
    }
}

/// One logical player-inventory resource delta in an atomic transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptInventoryResourceDelta {
    resource_id: String,
    delta: i16,
}

impl ScriptInventoryResourceDelta {
    pub fn try_new(resource_id: impl AsRef<str>, delta: i16) -> Result<Self, ScriptDtoError> {
        if delta == 0 || delta.unsigned_abs() > MAX_INVENTORY_RESOURCE_DELTA as u16 {
            return Err(ScriptDtoError::InvalidAmount);
        }
        Ok(Self {
            resource_id: validate_contract_resource_id(resource_id.as_ref())?,
            delta,
        })
    }

    pub fn resource_id(&self) -> &str {
        &self.resource_id
    }

    pub const fn delta(&self) -> i16 {
        self.delta
    }
}

/// A bounded request the server must commit or reject as one inventory and storage mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptInventoryStorageTransaction {
    id: String,
    player_id: ScriptPlayerId,
    inventory: Vec<ScriptInventoryResourceDelta>,
    storage: Vec<ScriptStorageMutation>,
}

impl ScriptInventoryStorageTransaction {
    pub fn try_new(
        id: impl AsRef<str>,
        player_id: ScriptPlayerId,
        inventory: Vec<ScriptInventoryResourceDelta>,
        storage: Vec<ScriptStorageMutation>,
    ) -> Result<Self, ScriptDtoError> {
        if inventory.is_empty() || storage.is_empty() {
            return Err(ScriptDtoError::EmptyTransaction);
        }
        if inventory.len() > MAX_INVENTORY_STORAGE_MUTATIONS
            || storage.len() > MAX_INVENTORY_STORAGE_MUTATIONS
        {
            return Err(ScriptDtoError::TooManyEntries {
                field: "inventory storage transaction",
                max: MAX_INVENTORY_STORAGE_MUTATIONS,
            });
        }
        let mut inventory_ids = BTreeMap::new();
        for delta in &inventory {
            if inventory_ids.insert(delta.resource_id(), ()).is_some() {
                return Err(ScriptDtoError::DuplicateId {
                    field: "inventory resource id",
                    actual_bytes: delta.resource_id().len(),
                });
            }
        }
        let mut storage_keys = BTreeMap::new();
        for mutation in &storage {
            mutation.validate()?;
            if storage_keys.insert(mutation.key(), ()).is_some() {
                return Err(ScriptDtoError::DuplicateId {
                    field: "plugin storage key",
                    actual_bytes: mutation.key().len(),
                });
            }
        }
        Ok(Self {
            id: validate_script_id(id.as_ref())?,
            player_id,
            inventory,
            storage,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub const fn player_id(&self) -> ScriptPlayerId {
        self.player_id
    }

    pub fn inventory(&self) -> &[ScriptInventoryResourceDelta] {
        &self.inventory
    }

    pub fn storage(&self) -> &[ScriptStorageMutation] {
        &self.storage
    }
}

/// Bounded atomic mutation of one connected player's main inventory and hotbar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptPlayerInventoryTransaction {
    request_id: String,
    player_id: ScriptPlayerId,
    deltas: Vec<ScriptInventoryResourceDelta>,
}

impl ScriptPlayerInventoryTransaction {
    pub fn try_new(
        request_id: impl AsRef<str>,
        player_id: ScriptPlayerId,
        deltas: Vec<ScriptInventoryResourceDelta>,
    ) -> Result<Self, ScriptDtoError> {
        if deltas.is_empty() {
            return Err(ScriptDtoError::EmptyTransaction);
        }
        if deltas.len() > MAX_INVENTORY_STORAGE_MUTATIONS {
            return Err(ScriptDtoError::TooManyEntries {
                field: "player inventory transaction",
                max: MAX_INVENTORY_STORAGE_MUTATIONS,
            });
        }
        let mut resource_ids = BTreeMap::new();
        for delta in &deltas {
            if resource_ids.insert(delta.resource_id(), ()).is_some() {
                return Err(ScriptDtoError::DuplicateId {
                    field: "inventory resource id",
                    actual_bytes: delta.resource_id().len(),
                });
            }
        }
        Ok(Self {
            request_id: validate_script_id(request_id.as_ref())?,
            player_id,
            deltas,
        })
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub const fn player_id(&self) -> ScriptPlayerId {
        self.player_id
    }

    pub fn deltas(&self) -> &[ScriptInventoryResourceDelta] {
        &self.deltas
    }
}

/// Exact reason why an admitted player inventory transaction did not commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScriptPlayerInventoryFailure {
    PlayerUnavailable,
    RuntimeUnavailable,
    UnknownResource,
    InsufficientResource,
    InventoryFull,
}

impl ScriptPlayerInventoryFailure {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PlayerUnavailable => "player_unavailable",
            Self::RuntimeUnavailable => "runtime_unavailable",
            Self::UnknownResource => "unknown_resource",
            Self::InsufficientResource => "insufficient_resource",
            Self::InventoryFull => "inventory_full",
        }
    }
}

/// Server-normalized inventory click kind. Plugins never receive slot stacks or packet state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScriptInventoryClick {
    Primary,
    Secondary,
    ShiftPrimary,
    ShiftSecondary,
}
