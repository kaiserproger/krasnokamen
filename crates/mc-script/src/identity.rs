//! Stable script-visible player and entity identity handles.

/// Stable player identifier snapshot for script-visible DTOs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ScriptPlayerId(u64);

impl ScriptPlayerId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}

/// Stable entity identifier snapshot for script-visible DTOs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ScriptEntityId(u64);

impl ScriptEntityId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}
