//! Bounded protected-zone definitions owned by their registering plugin.

use super::*;

/// Generic actor policy attached to a protected zone by its owning plugin: the
/// players whose actions the server admits inside the zone (owner and trusted
/// members alike), plus whether everyone may use doors, trapdoors, fence gates,
/// buttons and levers. Every other build, container, ignition, explosion,
/// entity-interaction and damage action inside the zone is native-checked
/// against this list; operators are always admitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptZoneProtection {
    allowed_actor_uuids: Vec<String>,
    public_use: bool,
}

impl ScriptZoneProtection {
    /// A non-empty allow-list of at most [`MAX_SCRIPT_ZONE_ACTORS`] distinct
    /// player uuids, normalized and sorted.
    pub fn try_new<I, S>(allowed_actor_uuids: I, public_use: bool) -> Result<Self, ScriptDtoError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut actors = std::collections::BTreeSet::new();
        for uuid in allowed_actor_uuids {
            if actors.len() == MAX_SCRIPT_ZONE_ACTORS {
                return Err(ScriptDtoError::TooManyEntries {
                    field: "protection actor uuids",
                    max: MAX_SCRIPT_ZONE_ACTORS,
                });
            }
            actors.insert(normalize_player_uuid(uuid.as_ref())?);
        }
        if actors.is_empty() {
            return Err(ScriptDtoError::EmptyValue {
                field: "protection actor uuids",
            });
        }
        Ok(Self {
            allowed_actor_uuids: actors.into_iter().collect(),
            public_use,
        })
    }

    pub fn allowed_actor_uuids(&self) -> &[String] {
        &self.allowed_actor_uuids
    }

    pub fn public_use(&self) -> bool {
        self.public_use
    }

    pub fn allows_actor(&self, actor_uuid: &str, operator: bool) -> bool {
        operator
            || normalize_player_uuid(actor_uuid)
                .is_ok_and(|actor_uuid| self.allowed_actor_uuids.binary_search(&actor_uuid).is_ok())
    }
}

/// A bounded axis-aligned zone definition. The server owns its lifecycle and membership checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptAxisAlignedZone {
    id: String,
    dimension: String,
    minimum: ScriptPosition,
    maximum: ScriptPosition,
    protection: Option<ScriptZoneProtection>,
}

impl ScriptAxisAlignedZone {
    pub fn try_new(
        id: impl AsRef<str>,
        dimension: impl AsRef<str>,
        minimum: ScriptPosition,
        maximum: ScriptPosition,
    ) -> Result<Self, ScriptDtoError> {
        Self::try_new_with_protection(id, dimension, minimum, maximum, None)
    }

    pub fn try_new_with_protection(
        id: impl AsRef<str>,
        dimension: impl AsRef<str>,
        minimum: ScriptPosition,
        maximum: ScriptPosition,
        protection: Option<ScriptZoneProtection>,
    ) -> Result<Self, ScriptDtoError> {
        if minimum.x() > maximum.x() || minimum.y() > maximum.y() || minimum.z() > maximum.z() {
            return Err(ScriptDtoError::InvalidBounds);
        }
        Ok(Self {
            id: validate_script_id(id.as_ref())?,
            dimension: validate_contract_resource_id(dimension.as_ref())?,
            minimum,
            maximum,
            protection,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn dimension(&self) -> &str {
        &self.dimension
    }

    pub fn minimum(&self) -> ScriptPosition {
        self.minimum
    }

    pub fn maximum(&self) -> ScriptPosition {
        self.maximum
    }

    pub fn protection(&self) -> Option<&ScriptZoneProtection> {
        self.protection.as_ref()
    }
}
