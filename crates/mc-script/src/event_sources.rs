//! Source enumerations carried by gameplay event snapshots.

/// Closed crafting source snapshot exposed by item-crafted events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ScriptCraftingSource {
    Inventory,
    CraftingTable,
}

impl ScriptCraftingSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Inventory => "inventory",
            Self::CraftingTable => "crafting_table",
        }
    }
}

/// Closed pickup source snapshot exposed by item-picked-up events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ScriptItemPickupSource {
    ItemEntity,
    Arrow,
}

impl ScriptItemPickupSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ItemEntity => "item_entity",
            Self::Arrow => "arrow",
        }
    }
}

/// Closed source snapshot exposed by player entity-kill events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ScriptEntityKillSource {
    Melee,
}

impl ScriptEntityKillSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Melee => "melee",
        }
    }
}

/// Closed hand snapshot exposed by player entity-interaction events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ScriptInteractionHand {
    MainHand,
    OffHand,
}

impl ScriptInteractionHand {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MainHand => "main_hand",
            Self::OffHand => "off_hand",
        }
    }
}

/// Closed game-mode snapshot exposed by gameplay events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ScriptGameMode {
    Survival,
    Creative,
    Adventure,
}

impl ScriptGameMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Survival => "survival",
            Self::Creative => "creative",
            Self::Adventure => "adventure",
        }
    }
}

/// Protocol phase that produced a client-originated custom payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ScriptProtocolPhase {
    Configuration,
    Play,
}

impl ScriptProtocolPhase {
    /// Return the stable component-visible phase name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Configuration => "configuration",
            Self::Play => "play",
        }
    }
}
