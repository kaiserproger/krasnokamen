//! Identifier, byte-bound and plugin-metadata validators shared by the DTOs.

use super::*;

use crate::manifest::is_valid_plugin_id;

pub(crate) fn validate_script_id(value: &str) -> Result<String, ScriptDtoError> {
    validate_script_id_value(value)?;
    Ok(value.to_owned())
}

/// Validate the contract's own script-id shape without allocating.
///
/// The no-allocation form of the contract's script-id check, for a host that only
/// needs the refusal.
pub fn validate_script_id_value(value: &str) -> Result<(), ScriptDtoError> {
    if value.is_empty() {
        return Err(ScriptDtoError::EmptyValue { field: "script id" });
    }
    if value.len() > MAX_SCRIPT_ID_BYTES {
        return Err(ScriptDtoError::ValueTooLong {
            field: "script id",
            max_bytes: MAX_SCRIPT_ID_BYTES,
            actual_bytes: value.len(),
        });
    }
    if !value.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
    }) {
        return Err(ScriptDtoError::InvalidId {
            field: "script id",
            actual_bytes: value.len(),
        });
    }
    Ok(())
}

pub(crate) fn validate_bounded_value(
    field: &'static str,
    value: &str,
    max_bytes: usize,
) -> Result<(), ScriptDtoError> {
    if value.len() > max_bytes {
        return Err(ScriptDtoError::ValueTooLong {
            field,
            max_bytes,
            actual_bytes: value.len(),
        });
    }
    Ok(())
}

pub(crate) fn validate_bounded_nonempty(
    field: &'static str,
    value: &str,
    max_bytes: usize,
) -> Result<(), ScriptDtoError> {
    if value.is_empty() {
        return Err(ScriptDtoError::EmptyValue { field });
    }
    validate_bounded_value(field, value, max_bytes)
}

pub(crate) fn validate_player_identity(uuid: &str, username: &str) -> Result<(), ScriptDtoError> {
    if !uuid
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(ScriptDtoError::InvalidId {
            field: "player uuid",
            actual_bytes: uuid.len(),
        });
    }
    if !username
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(ScriptDtoError::InvalidId {
            field: "player username",
            actual_bytes: username.len(),
        });
    }
    Ok(())
}

pub(crate) fn normalize_player_uuid(uuid: &str) -> Result<String, ScriptDtoError> {
    let normalized = uuid
        .bytes()
        .filter(|byte| *byte != b'-')
        .map(char::from)
        .collect::<String>()
        .to_ascii_lowercase();
    if normalized.len() != 32 || !normalized.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ScriptDtoError::InvalidId {
            field: "protection actor uuid",
            actual_bytes: uuid.len(),
        });
    }
    Ok(normalized)
}

/// Validate one namespaced contract resource identifier, returning it on success.
pub fn validate_contract_resource_id(value: &str) -> Result<String, ScriptDtoError> {
    check_contract_resource_id(value)?;
    Ok(value.to_owned())
}

pub(crate) fn check_contract_resource_id(value: &str) -> Result<(), ScriptDtoError> {
    if value.len() > MAX_SCRIPT_RESOURCE_ID_BYTES {
        return Err(ScriptDtoError::ValueTooLong {
            field: "resource id",
            max_bytes: MAX_SCRIPT_RESOURCE_ID_BYTES,
            actual_bytes: value.len(),
        });
    }
    let Some((namespace, path)) = value.split_once(':') else {
        return Err(ScriptDtoError::InvalidResourceId {
            field: "resource id",
            actual_bytes: value.len(),
        });
    };
    if namespace.is_empty()
        || path.is_empty()
        || path.contains(':')
        || !namespace.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-' | b'.')
        })
        || !path.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'_' | b'-' | b'.' | b'/')
        })
    {
        return Err(ScriptDtoError::InvalidResourceId {
            field: "resource id",
            actual_bytes: value.len(),
        });
    }
    Ok(())
}

pub(crate) fn validate_plugin_storage_key(value: &str) -> Result<String, ScriptDtoError> {
    if value.is_empty() {
        return Err(ScriptDtoError::EmptyValue {
            field: "plugin storage key",
        });
    }
    if value.len() > MAX_PLUGIN_STORAGE_KEY_BYTES {
        return Err(ScriptDtoError::ValueTooLong {
            field: "plugin storage key",
            max_bytes: MAX_PLUGIN_STORAGE_KEY_BYTES,
            actual_bytes: value.len(),
        });
    }
    Ok(value.to_owned())
}

pub(crate) fn validate_plugin_storage_value(value: &str) -> Result<(), ScriptDtoError> {
    if value.is_empty() {
        return Err(ScriptDtoError::EmptyValue {
            field: "plugin storage value",
        });
    }
    if value.len() > MAX_PLUGIN_STORAGE_VALUE_BYTES {
        return Err(ScriptDtoError::ValueTooLong {
            field: "plugin storage value",
            max_bytes: MAX_PLUGIN_STORAGE_VALUE_BYTES,
            actual_bytes: value.len(),
        });
    }
    Ok(())
}

pub(crate) fn validate_target_plugin_id(value: &str) -> Result<String, ScriptDtoError> {
    if value.is_empty() {
        return Err(ScriptDtoError::EmptyValue {
            field: "target plugin id",
        });
    }
    if value.len() > MAX_SCRIPT_ID_BYTES {
        return Err(ScriptDtoError::ValueTooLong {
            field: "target plugin id",
            max_bytes: MAX_SCRIPT_ID_BYTES,
            actual_bytes: value.len(),
        });
    }
    if !is_valid_plugin_id(value) {
        return Err(ScriptDtoError::InvalidId {
            field: "target plugin id",
            actual_bytes: value.len(),
        });
    }
    Ok(value.to_owned())
}
