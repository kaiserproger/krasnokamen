//! Targeted results of admitted script operations and storage requests.

use crate::*;

impl ScriptEvent {
    /// Build a targeted plugin-storage read result.
    pub fn plugin_storage_get_result(
        target_plugin_id: impl AsRef<str>,
        request: &ScriptPluginStorageGetRequest,
        value: Option<String>,
        version: Option<u64>,
    ) -> Result<Self, ScriptDtoError> {
        if value.is_some() != version.is_some() {
            return Err(ScriptDtoError::InconsistentResult {
                field: "plugin storage get value/version",
            });
        }
        if let Some(value) = &value {
            validate_plugin_storage_value(value)?;
        }
        Ok(Self {
            target_plugin_id: Some(validate_target_plugin_id(target_plugin_id.as_ref())?),
            kind: ScriptEventKind::PluginStorageGetResult {
                request_id: request.request_id().to_owned(),
                key: request.key().to_owned(),
                value,
                version,
                failure: None,
            },
            origin_generation: None,
        })
    }

    /// Build a targeted plugin-storage compare-and-swap result.
    pub fn plugin_storage_cas_result(
        target_plugin_id: impl AsRef<str>,
        request: &ScriptPluginStorageCompareAndSwapRequest,
        applied: bool,
        version: Option<u64>,
    ) -> Result<Self, ScriptDtoError> {
        if applied && version.is_none() {
            return Err(ScriptDtoError::InconsistentResult {
                field: "plugin storage compare-and-swap success/version",
            });
        }
        Ok(Self {
            target_plugin_id: Some(validate_target_plugin_id(target_plugin_id.as_ref())?),
            kind: ScriptEventKind::PluginStorageCasResult {
                request_id: request.request_id().to_owned(),
                key: request.key().to_owned(),
                applied,
                version,
                failure: None,
            },
            origin_generation: None,
        })
    }

    /// Build a targeted plugin-storage delete result.
    pub fn plugin_storage_delete_result(
        target_plugin_id: impl AsRef<str>,
        request: &ScriptPluginStorageDeleteRequest,
        deleted: bool,
        version: Option<u64>,
    ) -> Result<Self, ScriptDtoError> {
        if deleted && version.is_none() {
            return Err(ScriptDtoError::InconsistentResult {
                field: "plugin storage delete success/version",
            });
        }
        Ok(Self {
            target_plugin_id: Some(validate_target_plugin_id(target_plugin_id.as_ref())?),
            kind: ScriptEventKind::PluginStorageDeleteResult {
                request_id: request.request_id().to_owned(),
                key: request.key().to_owned(),
                deleted,
                version,
                failure: None,
            },
            origin_generation: None,
        })
    }

    pub fn operation_result(
        target_plugin_id: impl AsRef<str>,
        request_id: &str,
        operation_id: Option<&str>,
        outcome: ScriptOperationOutcome,
    ) -> Result<Self, ScriptDtoError> {
        let event = Self {
            target_plugin_id: Some(validate_target_plugin_id(target_plugin_id.as_ref())?),
            kind: ScriptEventKind::OperationResult {
                request_id: validate_script_id(request_id)?,
                operation_id: operation_id.map(validate_script_id).transpose()?,
                outcome,
            },
            origin_generation: None,
        };
        event.validate()?;
        Ok(event)
    }
}
