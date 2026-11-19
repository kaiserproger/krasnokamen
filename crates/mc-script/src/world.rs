//! World, entity and Loader mutation requests and their failure categories.

use super::*;

/// Typed request to set the authoritative overworld simulation time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptWorldTimeSetRequest {
    request_id: String,
    world_time: u64,
}

impl ScriptWorldTimeSetRequest {
    pub fn try_new(request_id: impl AsRef<str>, world_time: u64) -> Result<Self, ScriptDtoError> {
        if world_time > MAX_SCRIPT_WORLD_TIME {
            return Err(ScriptDtoError::InvalidBounds);
        }
        Ok(Self {
            request_id: validate_script_id(request_id.as_ref())?,
            world_time,
        })
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub const fn world_time(&self) -> u64 {
        self.world_time
    }
}

/// Exact public category for a world-time request that did not commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScriptWorldTimeSetFailure {
    Busy,
    RuntimeUnavailable,
    Rejected,
}

impl ScriptWorldTimeSetFailure {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Busy => "busy",
            Self::RuntimeUnavailable => "runtime_unavailable",
            Self::Rejected => "rejected",
        }
    }
}

/// Typed request to set one default block state in the authoritative world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptWorldBlockSetRequest {
    request_id: String,
    dimension: String,
    block_id: String,
    x: i32,
    y: i32,
    z: i32,
}

impl ScriptWorldBlockSetRequest {
    pub fn try_new(
        request_id: impl AsRef<str>,
        dimension: impl AsRef<str>,
        block_id: impl AsRef<str>,
        x: i32,
        y: i32,
        z: i32,
    ) -> Result<Self, ScriptDtoError> {
        Ok(Self {
            request_id: validate_script_id(request_id.as_ref())?,
            dimension: validate_contract_resource_id(dimension.as_ref())?,
            block_id: validate_contract_resource_id(block_id.as_ref())?,
            x,
            y,
            z,
        })
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub fn dimension(&self) -> &str {
        &self.dimension
    }

    pub fn block_id(&self) -> &str {
        &self.block_id
    }

    pub const fn x(&self) -> i32 {
        self.x
    }

    pub const fn y(&self) -> i32 {
        self.y
    }

    pub const fn z(&self) -> i32 {
        self.z
    }
}

/// Exact public category for a world-block request that did not commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScriptWorldBlockSetFailure {
    UnknownBlock,
    UnsupportedDimension,
    OutOfWorld,
    Busy,
    RuntimeUnavailable,
    Rejected,
}

impl ScriptWorldBlockSetFailure {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnknownBlock => "unknown_block",
            Self::UnsupportedDimension => "unsupported_dimension",
            Self::OutOfWorld => "out_of_world",
            Self::Busy => "busy",
            Self::RuntimeUnavailable => "runtime_unavailable",
            Self::Rejected => "rejected",
        }
    }
}

/// Typed request to grant one Loader-owned custom block item through player inventory authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptLoaderItemGrantRequest {
    request_id: String,
    player_id: ScriptPlayerId,
    block_id: String,
    count: u8,
}

impl ScriptLoaderItemGrantRequest {
    pub fn try_new(
        request_id: impl AsRef<str>,
        player_id: ScriptPlayerId,
        block_id: impl AsRef<str>,
        count: u8,
    ) -> Result<Self, ScriptDtoError> {
        if !(1..=64).contains(&count) {
            return Err(ScriptDtoError::InvalidBounds);
        }
        Ok(Self {
            request_id: validate_script_id(request_id.as_ref())?,
            player_id,
            block_id: validate_contract_resource_id(block_id.as_ref())?,
            count,
        })
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub const fn player_id(&self) -> ScriptPlayerId {
        self.player_id
    }

    pub fn block_id(&self) -> &str {
        &self.block_id
    }

    pub const fn count(&self) -> u8 {
        self.count
    }
}

/// Exact public category for a Loader custom-block item grant that did not commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScriptLoaderItemGrantFailure {
    LoaderUnavailable,
    NotOwned,
    PlayerUnavailable,
    InventoryFull,
    RuntimeUnavailable,
    Rejected,
}

impl ScriptLoaderItemGrantFailure {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LoaderUnavailable => "loader_unavailable",
            Self::NotOwned => "not_owned",
            Self::PlayerUnavailable => "player_unavailable",
            Self::InventoryFull => "inventory_full",
            Self::RuntimeUnavailable => "runtime_unavailable",
            Self::Rejected => "rejected",
        }
    }
}

/// Exact public category for an entity-spawn request that did not commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScriptEntitySpawnFailure {
    UnknownEntityType,
    ActorUnavailable,
    Busy,
    RuntimeUnavailable,
    Rejected,
}

impl ScriptEntitySpawnFailure {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnknownEntityType => "unknown_entity_type",
            Self::ActorUnavailable => "actor_unavailable",
            Self::Busy => "busy",
            Self::RuntimeUnavailable => "runtime_unavailable",
            Self::Rejected => "rejected",
        }
    }
}

/// Typed request to apply one bounded server-owned damage mutation to a non-player entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptEntityDamageRequest {
    request_id: String,
    entity_id: ScriptEntityId,
    amount_bits: u32,
}

impl ScriptEntityDamageRequest {
    pub fn try_new(
        request_id: impl AsRef<str>,
        entity_id: ScriptEntityId,
        amount: f32,
    ) -> Result<Self, ScriptDtoError> {
        if entity_id.value() > i32::MAX as u64
            || !amount.is_finite()
            || !(0.0..=MAX_SCRIPT_ENTITY_DAMAGE).contains(&amount)
            || amount == 0.0
        {
            return Err(ScriptDtoError::InvalidBounds);
        }
        Ok(Self {
            request_id: validate_script_id(request_id.as_ref())?,
            entity_id,
            amount_bits: amount.to_bits(),
        })
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub const fn entity_id(&self) -> ScriptEntityId {
        self.entity_id
    }

    pub const fn amount(&self) -> f32 {
        f32::from_bits(self.amount_bits)
    }
}

/// Exact public category for an entity-damage request that did not apply damage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScriptEntityDamageFailure {
    Busy,
    RuntimeUnavailable,
    Rejected,
}

impl ScriptEntityDamageFailure {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Busy => "busy",
            Self::RuntimeUnavailable => "runtime_unavailable",
            Self::Rejected => "rejected",
        }
    }
}
