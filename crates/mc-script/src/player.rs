//! Player context, position and player-scoped request DTOs.

use super::*;

/// Result of admitting a player command to a plugin-owned root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlayerCommandAdmission {
    NotOwned,
    Enqueued,
    Dropped,
    PermissionDenied,
    OwnedRejected { error: ScriptDtoError },
}

/// An authenticated command event prepared for the native owner's commit FIFO.
/// Preparation does not publish an event or claim queue admission.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum PreparedPlayerCommand {
    NotOwned,
    PermissionDenied,
    OwnedRejected { error: ScriptDtoError },
    Event(ScriptEvent),
}

/// Validated immutable position carried by script commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[serde(
    try_from = "ScriptPositionCoordinates",
    into = "ScriptPositionCoordinates"
)]
pub struct ScriptPosition {
    x_bits: u64,
    y_bits: u64,
    z_bits: u64,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ScriptPositionCoordinates {
    x: f64,
    y: f64,
    z: f64,
}

impl TryFrom<ScriptPositionCoordinates> for ScriptPosition {
    type Error = ScriptDtoError;

    fn try_from(value: ScriptPositionCoordinates) -> Result<Self, Self::Error> {
        Self::try_new(value.x, value.y, value.z).ok_or(ScriptDtoError::InvalidBounds)
    }
}

impl From<ScriptPosition> for ScriptPositionCoordinates {
    fn from(value: ScriptPosition) -> Self {
        Self {
            x: value.x(),
            y: value.y(),
            z: value.z(),
        }
    }
}

impl ScriptPosition {
    #[must_use]
    pub fn try_new(x: f64, y: f64, z: f64) -> Option<Self> {
        (x.is_finite()
            && y.is_finite()
            && z.is_finite()
            && x.abs() <= SCRIPT_HORIZONTAL_COORDINATE_LIMIT
            && y.abs() <= SCRIPT_VERTICAL_COORDINATE_LIMIT
            && z.abs() <= SCRIPT_HORIZONTAL_COORDINATE_LIMIT)
            .then_some(Self {
                x_bits: x.to_bits(),
                y_bits: y.to_bits(),
                z_bits: z.to_bits(),
            })
    }

    #[must_use]
    pub fn x(self) -> f64 {
        f64::from_bits(self.x_bits)
    }

    #[must_use]
    pub fn y(self) -> f64 {
        f64::from_bits(self.y_bits)
    }

    #[must_use]
    pub fn z(self) -> f64 {
        f64::from_bits(self.z_bits)
    }
}

/// Bounded same-dimension player teleport requested by one plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptPlayerTeleportRequest {
    request_id: String,
    player_id: ScriptPlayerId,
    position: ScriptPosition,
    dimension: String,
}

impl ScriptPlayerTeleportRequest {
    pub fn try_new(
        request_id: impl AsRef<str>,
        player_id: ScriptPlayerId,
        dimension: impl AsRef<str>,
        position: ScriptPosition,
    ) -> Result<Self, ScriptDtoError> {
        Ok(Self {
            request_id: validate_script_id(request_id.as_ref())?,
            player_id,
            dimension: validate_contract_resource_id(dimension.as_ref())?,
            position,
        })
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub const fn player_id(&self) -> ScriptPlayerId {
        self.player_id
    }

    pub fn dimension(&self) -> &str {
        &self.dimension
    }

    pub const fn position(&self) -> ScriptPosition {
        self.position
    }
}

/// Exact reason why an admitted player teleport did not commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScriptPlayerTeleportFailure {
    PlayerUnavailable,
    DimensionMismatch,
    RuntimeUnavailable,
}

impl ScriptPlayerTeleportFailure {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PlayerUnavailable => "player_unavailable",
            Self::DimensionMismatch => "dimension_mismatch",
            Self::RuntimeUnavailable => "runtime_unavailable",
        }
    }
}

/// Bounded request for a point-in-time connected-player snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptOnlinePlayersRequest {
    request_id: String,
    limit: usize,
}

impl ScriptOnlinePlayersRequest {
    pub fn try_new(request_id: impl AsRef<str>, limit: usize) -> Result<Self, ScriptDtoError> {
        if limit == 0 || limit > MAX_ONLINE_PLAYER_QUERY_LIMIT {
            return Err(ScriptDtoError::InvalidBounds);
        }
        Ok(Self {
            request_id: validate_script_id(request_id.as_ref())?,
            limit,
        })
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub const fn limit(&self) -> usize {
        self.limit
    }
}

impl ScriptPlayerContext {
    pub fn try_new(
        uuid: impl AsRef<str>,
        username: impl AsRef<str>,
        operator: bool,
        dimension: impl AsRef<str>,
        x: f64,
        y: f64,
        z: f64,
    ) -> Result<Self, ScriptDtoError> {
        let uuid = uuid.as_ref();
        let username = username.as_ref();
        validate_bounded_nonempty("player uuid", uuid, MAX_SCRIPT_PLAYER_UUID_BYTES)?;
        validate_bounded_nonempty("player username", username, MAX_SCRIPT_PLAYER_NAME_BYTES)?;
        validate_player_identity(uuid, username)?;
        let position = ScriptPosition::try_new(x, y, z).ok_or(ScriptDtoError::InvalidBounds)?;
        Ok(Self {
            snapshot: Box::new(ScriptPlayerContextSnapshot {
                uuid: uuid.to_owned(),
                username: username.to_owned(),
                dimension: validate_contract_resource_id(dimension.as_ref())?,
                operator,
                x_bits: position.x_bits,
                y_bits: position.y_bits,
                z_bits: position.z_bits,
            }),
        })
    }

    #[must_use]
    pub fn new(
        uuid: impl AsRef<str>,
        username: impl AsRef<str>,
        operator: bool,
        dimension: impl AsRef<str>,
        x: f64,
        y: f64,
        z: f64,
    ) -> Self {
        Self::try_new(uuid, username, operator, dimension, x, y, z)
            .expect("server-authored script player context must be bounded")
    }

    #[must_use]
    pub fn uuid(&self) -> &str {
        &self.snapshot.uuid
    }

    #[must_use]
    pub fn username(&self) -> &str {
        &self.snapshot.username
    }

    #[must_use]
    pub fn operator(&self) -> bool {
        self.snapshot.operator
    }

    #[must_use]
    pub fn dimension(&self) -> &str {
        &self.snapshot.dimension
    }

    #[must_use]
    pub fn x(&self) -> f64 {
        f64::from_bits(self.snapshot.x_bits)
    }

    #[must_use]
    pub fn y(&self) -> f64 {
        f64::from_bits(self.snapshot.y_bits)
    }

    #[must_use]
    pub fn z(&self) -> f64 {
        f64::from_bits(self.snapshot.z_bits)
    }

    pub fn validate(&self) -> Result<(), ScriptDtoError> {
        validate_bounded_nonempty("player uuid", self.uuid(), MAX_SCRIPT_PLAYER_UUID_BYTES)?;
        validate_bounded_nonempty(
            "player username",
            self.username(),
            MAX_SCRIPT_PLAYER_NAME_BYTES,
        )?;
        validate_player_identity(self.uuid(), self.username())?;
        check_contract_resource_id(self.dimension())?;
        ScriptPosition::try_new(self.x(), self.y(), self.z())
            .ok_or(ScriptDtoError::InvalidBounds)
            .map(drop)
    }
}

/// Immutable identity, pose, and dimension for one connected player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptOnlinePlayerSnapshot {
    player_id: ScriptPlayerId,
    context: ScriptPlayerContext,
    dimension: String,
}

impl ScriptOnlinePlayerSnapshot {
    pub fn try_new(
        player_id: ScriptPlayerId,
        context: ScriptPlayerContext,
        dimension: impl AsRef<str>,
    ) -> Result<Self, ScriptDtoError> {
        context.validate()?;
        let dimension = dimension.as_ref();
        validate_contract_resource_id(dimension)?;
        Ok(Self {
            player_id,
            context,
            dimension: dimension.to_owned(),
        })
    }

    pub const fn player_id(&self) -> ScriptPlayerId {
        self.player_id
    }

    pub fn context(&self) -> &ScriptPlayerContext {
        &self.context
    }

    pub fn dimension(&self) -> &str {
        &self.dimension
    }

    pub(crate) fn validate(&self) -> Result<(), ScriptDtoError> {
        self.context.validate()?;
        validate_contract_resource_id(&self.dimension).map(drop)
    }
}
