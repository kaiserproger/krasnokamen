//! Loader client view, selection, brand and view-action snapshots.

use crate::*;

impl ScriptEvent {
    /// Build one explicit key-driven view request event targeted to the owner.
    pub fn loader_view_request(
        target_plugin_id: impl AsRef<str>,
        player_id: ScriptPlayerId,
        request_kind: ScriptClientViewKey,
    ) -> Result<Self, ScriptDtoError> {
        Ok(Self {
            target_plugin_id: Some(validate_target_plugin_id(target_plugin_id.as_ref())?),
            kind: ScriptEventKind::LoaderViewRequest {
                player_id,
                request_kind,
            },
            origin_generation: None,
        })
    }

    /// Build one admitted client view action snapshot targeted to the owner.
    #[allow(clippy::too_many_arguments)]
    pub fn loader_view_action(
        target_plugin_id: impl AsRef<str>,
        player_id: ScriptPlayerId,
        view_instance_id: impl AsRef<str>,
        view_revision: u64,
        action_id: impl AsRef<str>,
        action_sequence: u64,
        fields: Vec<ScriptClientViewField>,
        selection_token: Option<String>,
    ) -> Result<Self, ScriptDtoError> {
        let view_instance_id = validate_contract_resource_id(view_instance_id.as_ref())?;
        let action_id = validate_contract_resource_id(action_id.as_ref())?;
        if view_revision > MAX_CLIENT_VIEW_EXACT_INTEGER
            || action_sequence > MAX_CLIENT_VIEW_EXACT_INTEGER
        {
            return Err(ScriptDtoError::InvalidBounds);
        }
        if fields.len() > MAX_CLIENT_VIEW_FIELDS {
            return Err(ScriptDtoError::TooManyEntries {
                field: "view action fields",
                max: MAX_CLIENT_VIEW_FIELDS,
            });
        }
        if let Some(token) = selection_token.as_deref() {
            validate_bounded_nonempty("view selection token", token, MAX_CLIENT_VIEW_ID_BYTES)?;
        }
        Ok(Self {
            target_plugin_id: Some(validate_target_plugin_id(target_plugin_id.as_ref())?),
            kind: ScriptEventKind::LoaderViewAction {
                player_id,
                view_instance_id,
                view_revision,
                action_id,
                action_sequence,
                fields,
                selection_token,
            },
            origin_generation: None,
        })
    }

    /// Build the outcome of one `open_client_view` request for the caller.
    pub fn client_view_opened(
        target_plugin_id: impl AsRef<str>,
        request_id: impl AsRef<str>,
        player_id: ScriptPlayerId,
        view_instance_id: Option<String>,
        revision: Option<u64>,
        failure: Option<ScriptClientViewFailure>,
    ) -> Result<Self, ScriptDtoError> {
        Ok(Self {
            target_plugin_id: Some(validate_target_plugin_id(target_plugin_id.as_ref())?),
            kind: ScriptEventKind::ClientViewOpened {
                request_id: validate_script_id(request_id.as_ref())?,
                player_id,
                view_instance_id,
                revision,
                failure,
            },
            origin_generation: None,
        })
    }

    /// Build the outcome of one `begin_client_selection` request for the caller.
    pub fn client_selection_started(
        target_plugin_id: impl AsRef<str>,
        request_id: impl AsRef<str>,
        player_id: ScriptPlayerId,
        selection_context_id: Option<String>,
        expires_at_tick: Option<u64>,
        failure: Option<ScriptClientViewFailure>,
    ) -> Result<Self, ScriptDtoError> {
        Ok(Self {
            target_plugin_id: Some(validate_target_plugin_id(target_plugin_id.as_ref())?),
            kind: ScriptEventKind::ClientSelectionStarted {
                request_id: validate_script_id(request_id.as_ref())?,
                player_id,
                selection_context_id,
                expires_at_tick,
                failure,
            },
            origin_generation: None,
        })
    }

    /// Build a client-brand event snapshot. Empty brands are preserved.
    pub fn client_brand(
        player_id: ScriptPlayerId,
        brand: impl AsRef<str>,
    ) -> Result<Self, ScriptDtoError> {
        let brand = brand.as_ref();
        validate_bounded_value("client brand", brand, MAX_SCRIPT_CUSTOM_PAYLOAD_BYTES)?;
        Ok(Self {
            target_plugin_id: None,
            kind: ScriptEventKind::ClientBrand {
                player_id,
                brand: brand.to_owned(),
            },
            origin_generation: None,
        })
    }
}
