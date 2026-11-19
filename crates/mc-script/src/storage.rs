//! Plugin-scoped storage mutations and their request/result correlation.

use super::*;

/// One plugin-storage mutation in an atomic inventory and storage transaction.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "operation", deny_unknown_fields)]
#[non_exhaustive]
pub enum ScriptStorageMutation {
    #[serde(rename = "cas")]
    CompareAndSwap {
        key: String,
        expected_version: Option<u64>,
        value: String,
    },
    #[serde(rename = "delete")]
    Delete {
        key: String,
        expected_version: Option<u64>,
    },
}

impl ScriptStorageMutation {
    pub fn compare_and_swap(
        key: impl AsRef<str>,
        expected_version: Option<u64>,
        value: impl AsRef<str>,
    ) -> Result<Self, ScriptDtoError> {
        let value = value.as_ref();
        validate_plugin_storage_value(value)?;
        Ok(Self::CompareAndSwap {
            key: validate_plugin_storage_key(key.as_ref())?,
            expected_version,
            value: value.to_owned(),
        })
    }

    pub fn delete(
        key: impl AsRef<str>,
        expected_version: Option<u64>,
    ) -> Result<Self, ScriptDtoError> {
        Ok(Self::Delete {
            key: validate_plugin_storage_key(key.as_ref())?,
            expected_version,
        })
    }

    pub fn key(&self) -> &str {
        match self {
            Self::CompareAndSwap { key, .. } | Self::Delete { key, .. } => key,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), ScriptDtoError> {
        match self {
            Self::CompareAndSwap { key, value, .. } => {
                validate_plugin_storage_key(key)?;
                validate_plugin_storage_value(value)
            }
            Self::Delete { key, .. } => validate_plugin_storage_key(key).map(drop),
        }
    }
}

/// Validated plugin-storage read request used to correlate its targeted result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptPluginStorageGetRequest {
    request_id: String,
    key: String,
}

impl ScriptPluginStorageGetRequest {
    pub fn try_new(
        request_id: impl AsRef<str>,
        key: impl AsRef<str>,
    ) -> Result<Self, ScriptDtoError> {
        Ok(Self {
            request_id: validate_script_id(request_id.as_ref())?,
            key: validate_plugin_storage_key(key.as_ref())?,
        })
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub fn key(&self) -> &str {
        &self.key
    }
}

/// Validated plugin-storage compare-and-swap request and result correlation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptPluginStorageCompareAndSwapRequest {
    request_id: String,
    key: String,
    expected_version: Option<u64>,
    value: String,
}

impl ScriptPluginStorageCompareAndSwapRequest {
    pub fn try_new(
        request_id: impl AsRef<str>,
        key: impl AsRef<str>,
        expected_version: Option<u64>,
        value: impl AsRef<str>,
    ) -> Result<Self, ScriptDtoError> {
        let value = value.as_ref();
        validate_plugin_storage_value(value)?;
        Ok(Self {
            request_id: validate_script_id(request_id.as_ref())?,
            key: validate_plugin_storage_key(key.as_ref())?,
            expected_version,
            value: value.to_owned(),
        })
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub const fn expected_version(&self) -> Option<u64> {
        self.expected_version
    }

    pub fn value(&self) -> &str {
        &self.value
    }
}

/// Validated plugin-storage delete request used to correlate its targeted result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptPluginStorageDeleteRequest {
    request_id: String,
    key: String,
    expected_version: Option<u64>,
}

/// Explicit terminal reason for a plugin-storage request that could not run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScriptPluginStorageFailure {
    /// This server has no persistent world-backed plugin storage.
    Unavailable,
    /// The storage actor stopped after a durable write or synchronization failure.
    DurabilityFailed,
}

impl ScriptPluginStorageFailure {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::DurabilityFailed => "durability_failed",
        }
    }
}

impl ScriptPluginStorageDeleteRequest {
    pub fn try_new(
        request_id: impl AsRef<str>,
        key: impl AsRef<str>,
        expected_version: Option<u64>,
    ) -> Result<Self, ScriptDtoError> {
        Ok(Self {
            request_id: validate_script_id(request_id.as_ref())?,
            key: validate_plugin_storage_key(key.as_ref())?,
            expected_version,
        })
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub const fn expected_version(&self) -> Option<u64> {
        self.expected_version
    }
}
