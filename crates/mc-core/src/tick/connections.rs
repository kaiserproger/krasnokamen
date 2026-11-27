//! Connection registry: connection -> player entity plus the identity and
//! negotiated client state that travel and chat need. Byte-exact move out of
//! `tick.rs`; no behaviour change.

use super::*;

/// Connections registry: connection -> player entity + identity.
#[derive(Debug, Default)]
pub struct Connections {
    pub(in crate::tick) by_connection: HashMap<ConnectionId, EntityId>,
    pub(in crate::tick) by_entity: HashMap<EntityId, ConnectionId>,
    names: HashMap<ConnectionId, String>,
    pub(in crate::tick) uuids: HashMap<ConnectionId, uuid::Uuid>,
    /// Transient negotiated client view radii, keyed by live connection.
    /// Recorded on [`PlayerInput::SetViewDistance`](crate::boundary::PlayerInput::SetViewDistance)
    /// (which precedes Join), moved with the connection across dimensions,
    /// and cleared on leave/remove. Never persisted: unnegotiated callers
    /// fall back to the configured server view.
    view_distances: HashMap<ConnectionId, u8>,
    /// Transient client UI locales, keyed by live connection.
    /// Recorded on [`PlayerInput::SetLocale`](crate::boundary::PlayerInput::SetLocale)
    /// (which precedes Join), moved with the connection across dimensions,
    /// and cleared on leave/remove. Never persisted: unnegotiated callers read
    /// English.
    locales: HashMap<ConnectionId, String>,
    /// Transient client chat settings, keyed by live connection. Recorded on
    /// [`PlayerInput::SetChatVisibility`](crate::boundary::PlayerInput::SetChatVisibility)
    /// (which precedes Join), moved with the connection across dimensions,
    /// and cleared on leave/remove. Never persisted; absent means `Full`.
    chat_visibilities: HashMap<ConnectionId, mc_domain::ChatVisibility>,
    /// Advances on every join, leave and dimension move. The tick's
    /// edge-visible session mirror is re-projected only when this or
    /// [`SessionLocks::generation`] moved (see `core::session_mirror`).
    membership_generation: u64,
}

impl Connections {
    pub(crate) fn insert(
        &mut self,
        connection: ConnectionId,
        entity: EntityId,
        uuid: uuid::Uuid,
        name: String,
    ) {
        self.by_connection.insert(connection, entity);
        self.by_entity.insert(entity, connection);
        self.names.insert(connection, name);
        self.uuids.insert(connection, uuid);
        self.membership_generation += 1;
    }

    /// Connection for a player entity, if joined.
    #[must_use]
    pub fn entity_connection(&self, entity: EntityId) -> Option<ConnectionId> {
        self.by_entity.get(&entity).copied()
    }

    /// Identity uuid for a connection, if joined.
    #[must_use]
    pub fn uuid_of(&self, connection: ConnectionId) -> Option<uuid::Uuid> {
        self.uuids.get(&connection).copied()
    }

    /// Display name for a player entity, if joined.
    #[must_use]
    pub fn name_of(&self, entity: EntityId) -> Option<&str> {
        self.by_entity
            .get(&entity)
            .and_then(|connection| self.names.get(connection))
            .map(String::as_str)
    }

    /// Negotiated client view radius for a connection, if the edge admitted one.
    /// Unnegotiated callers get `None` and fall back to the configured view.
    #[must_use]
    pub fn view_distance_of(&self, connection: ConnectionId) -> Option<u8> {
        self.view_distances.get(&connection).copied()
    }

    pub(crate) fn set_view_distance(&mut self, connection: ConnectionId, view_distance: u8) {
        self.view_distances.insert(connection, view_distance);
    }

    /// Client UI locale (`ClientInformation.language`) for a connection, if the
    /// edge admitted one. Unnegotiated callers get `None` and read English.
    #[must_use]
    pub fn locale_of(&self, connection: ConnectionId) -> Option<&str> {
        self.locales.get(&connection).map(String::as_str)
    }

    pub(crate) fn set_locale(&mut self, connection: ConnectionId, locale: String) {
        self.locales.insert(connection, locale);
    }

    /// The connection's chat setting; `Full` until the edge admits one.
    #[must_use]
    pub fn chat_visibility_of(&self, connection: ConnectionId) -> mc_domain::ChatVisibility {
        self.chat_visibilities
            .get(&connection)
            .copied()
            .unwrap_or_default()
    }

    pub(crate) fn set_chat_visibility(
        &mut self,
        connection: ConnectionId,
        visibility: mc_domain::ChatVisibility,
    ) {
        self.chat_visibilities.insert(connection, visibility);
    }

    pub(crate) fn remove_by_connection(&mut self, connection: ConnectionId) -> Option<EntityId> {
        self.view_distances.remove(&connection);
        self.locales.remove(&connection);
        self.chat_visibilities.remove(&connection);
        let entity = self.by_connection.remove(&connection)?;
        self.by_entity.remove(&entity);
        self.names.remove(&connection);
        self.uuids.remove(&connection);
        self.membership_generation += 1;
        Some(entity)
    }

    pub(crate) fn transfer_to(&mut self, connection: ConnectionId, destination: &mut Self) {
        let entity = self
            .by_connection
            .remove(&connection)
            .expect("joined connection");
        self.by_entity.remove(&entity);
        let uuid = self.uuids.remove(&connection).expect("joined UUID");
        let name = self.names.remove(&connection).expect("joined name");
        // Destination `insert` advances its own generation; the source lost a
        // member here.
        self.membership_generation += 1;
        destination.insert(connection, entity, uuid, name);
        if let Some(view_distance) = self.view_distances.remove(&connection) {
            destination.view_distances.insert(connection, view_distance);
        }
        if let Some(locale) = self.locales.remove(&connection) {
            destination.locales.insert(connection, locale);
        }
        if let Some(visibility) = self.chat_visibilities.remove(&connection) {
            destination.chat_visibilities.insert(connection, visibility);
        }
    }

    /// Player entity for a connection, if joined.
    #[must_use]
    pub fn entity_of(&self, connection: ConnectionId) -> Option<EntityId> {
        self.by_connection.get(&connection).copied()
    }

    /// All joined connections (reliable fan-out targets).
    pub fn all_connections(&self) -> impl Iterator<Item = ConnectionId> + '_ {
        self.by_connection.keys().copied()
    }

    /// Advances on every join, leave and dimension move (see
    /// `core::session_mirror` for the reader).
    #[must_use]
    pub(crate) fn membership_generation(&self) -> u64 {
        self.membership_generation
    }
}
