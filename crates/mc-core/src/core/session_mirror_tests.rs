//! The unlocked-session mirror the edge reads: its projection, and the
//! change stamp that lets the tick skip it on an unchanged tick.

use crate::boundary::{ConnectionId, EntityId};
use crate::core::session_mirror::{stamp, unlocked_sessions};
use crate::tick::TickState;
use std::sync::Arc;

fn fixture() -> TickState {
    let registry = Arc::new(crate::registry::CoreRegistry::load(None).unwrap());
    let world = crate::horde_bench::flat_world(&registry);
    let mut state = TickState::new(registry, world, 2, 512);
    state.require_plugin_login = true;
    state
}

fn join(state: &mut TickState, connection: ConnectionId) {
    state.connections.insert(
        connection,
        EntityId(connection.0 as i32),
        uuid::Uuid::from_u128(u128::from(connection.0)),
        format!("probe{}", connection.0),
    );
}

#[test]
fn projection_follows_join_unlock_relock_and_leave() {
    let mut state = fixture();
    let connection = ConnectionId(9);
    let empty = stamp(&state);
    assert!(unlocked_sessions(&state).is_empty());

    // A join under the login fence is projected locked, never unlocked.
    join(&mut state, connection);
    state.locked_sessions.insert(connection);
    let joined = stamp(&state);
    assert_ne!(joined, empty, "a join advances the mirror stamp");
    assert!(
        !unlocked_sessions(&state).contains(&connection),
        "a fenced join stays locked"
    );

    // The plugin login claim unlocks it: exactly what the edge may act on.
    state.locked_sessions.remove(&connection);
    let unlocked = stamp(&state);
    assert_ne!(unlocked, joined, "an unlock advances the mirror stamp");
    assert!(unlocked_sessions(&state).contains(&connection));

    // A rolled-back claim re-locks it.
    state.locked_sessions.insert(connection);
    assert!(!unlocked_sessions(&state).contains(&connection));
    state.locked_sessions.remove(&connection);

    // Leaving removes it from the projection even while unlocked.
    assert!(state.connections.remove_by_connection(connection).is_some());
    assert!(unlocked_sessions(&state).is_empty());
}

/// A join or a leave with no login-lock change must still move the stamp: the
/// two inputs are independent, and the edge mirror follows both.
#[test]
fn a_join_or_leave_alone_advances_the_stamp() {
    let mut state = fixture();
    let connection = ConnectionId(31);
    let empty = stamp(&state);
    join(&mut state, connection);
    let joined = stamp(&state);
    assert_ne!(joined, empty, "a join alone advances the mirror stamp");
    assert!(state.connections.remove_by_connection(connection).is_some());
    assert_ne!(
        stamp(&state),
        joined,
        "a leave alone advances the mirror stamp"
    );
    assert!(unlocked_sessions(&state).is_empty());
}

#[test]
fn lock_set_generation_only_advances_on_a_real_change() {
    let mut state = fixture();
    let connection = ConnectionId(4);
    state.locked_sessions.insert(connection);
    let locked = stamp(&state);
    state.locked_sessions.insert(connection);
    assert_eq!(stamp(&state), locked, "a duplicate lock is not a change");
    state.locked_sessions.remove(&connection);
    assert_ne!(stamp(&state), locked);
    assert_eq!(state.locked_sessions.len(), 0);
}

#[test]
fn transfer_advances_both_worlds_and_keeps_one_membership() {
    let mut source = crate::tick::Connections::default();
    let mut destination = crate::tick::Connections::default();
    let connection = ConnectionId(3);
    source.insert(
        connection,
        EntityId(3),
        uuid::Uuid::from_u128(3),
        "probe".into(),
    );
    let source_before = source.membership_generation();
    let destination_before = destination.membership_generation();
    source.transfer_to(connection, &mut destination);
    assert!(
        source.membership_generation() > source_before,
        "the source lost a member"
    );
    assert!(
        destination.membership_generation() > destination_before,
        "the destination gained a member"
    );
    assert!(source.entity_of(connection).is_none());
    assert_eq!(destination.entity_of(connection), Some(EntityId(3)));
}
