//! A player's entity removal is a membership change too.

use super::*;
use crate::boundary::{ConnectionId, PlayerInput};
use crate::core::session_mirror::{stamp, unlocked_sessions};
use std::sync::Arc;

fn fixture() -> TickState {
    let registry = Arc::new(crate::registry::CoreRegistry::load(None).unwrap());
    let world = crate::horde_bench::flat_world(&registry);
    let mut state = TickState::new(registry, world, 2, 512);
    state.require_plugin_login = true;
    state
}

/// Review P2: `remove_entity` used to delete the connection rows directly, so
/// the mirror's membership input never moved and the edge kept a stale unlock.
#[test]
fn removing_a_connected_player_advances_the_stamp() {
    let mut state = fixture();
    let connection = ConnectionId(41);
    let mut intents = Vec::new();
    let mut changes = ChangeSet::default();
    apply_inputs(
        &mut state,
        &[PlayerInput::Join {
            connection,
            uuid: uuid::Uuid::from_u128(41),
            name: "probe41".to_owned(),
            permission_level: 0,
        }],
        &mut intents,
        &mut changes,
    );
    let entity = state.connections.entity_of(connection).expect("joined");
    let slot = state.entities.slot_of_any(entity).expect("player row");
    // Unlock it so the projection holds the connection before the removal.
    state.locked_sessions.remove(&connection);
    assert!(unlocked_sessions(&state).contains(&connection));

    let before = stamp(&state);
    state.remove_entity(slot);
    assert_ne!(
        stamp(&state),
        before,
        "removing a connected player advances the mirror stamp"
    );
    assert!(
        !unlocked_sessions(&state).contains(&connection),
        "the removed player is no longer projected unlocked"
    );
    assert!(state.connections.entity_of(connection).is_none());
    assert_eq!(
        state.connections.uuid_of(connection),
        None,
        "the removal also clears the identity row"
    );
}
