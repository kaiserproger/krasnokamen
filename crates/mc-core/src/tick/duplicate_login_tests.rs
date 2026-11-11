//! Duplicate-login arbitration follows vanilla's order: the newer *login* kicks a session
//! already in Play (`disconnectAllPlayersWithProfile`), while a connection that finishes
//! *configuration* after the profile is in Play is refused (`handleConfigurationFinished`)
//! and never replaces the player in Play.
use super::*;

const KICK: &str = "You logged in from another location";
const PROFILE: u128 = 9;

fn fixture() -> TickState {
    let registry = Arc::new(crate::registry::CoreRegistry::load(None).unwrap());
    let world = crate::horde_bench::flat_world(&registry);
    TickState::new(registry, world, 2, 512)
}

fn login(connection: u64) -> PlayerInput {
    PlayerInput::ProfileLogin {
        connection: ConnectionId(connection),
        uuid: uuid::Uuid::from_u128(PROFILE),
    }
}

fn join(connection: u64) -> PlayerInput {
    PlayerInput::Join {
        connection: ConnectionId(connection),
        uuid: uuid::Uuid::from_u128(PROFILE),
        name: "Dup".into(),
        permission_level: 0,
    }
}

fn tick(state: &mut TickState, inputs: &[PlayerInput]) -> Vec<(ConnectionId, Vec<PlayerChange>)> {
    crate::tick::run_tick(state, inputs).1
}

fn kicked(drains: &[(ConnectionId, Vec<PlayerChange>)], connection: u64) -> bool {
    drains
        .iter()
        .filter(|(id, _)| *id == ConnectionId(connection))
        .flat_map(|(_, queue)| queue)
        .any(|change| matches!(change, PlayerChange::Disconnect { reason, .. } if reason == KICK))
}

/// Put a recognisable stack in the player's first slot so custody moves are visible.
fn hold_diamond(state: &mut TickState, connection: u64) {
    let diamond = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:diamond").unwrap())
        .unwrap();
    let entity = state.connections.entity_of(ConnectionId(connection)).unwrap();
    state.players.get_mut(&entity).unwrap().inventory.slots[0] =
        mc_data::ItemStack::new(diamond, 3);
}

fn first_slot_count(state: &TickState, connection: u64) -> u8 {
    let entity = state.connections.entity_of(ConnectionId(connection)).unwrap();
    state.players[&entity].inventory.slots[0].count as u8
}

fn in_play(state: &TickState, connection: u64) -> bool {
    state.connections.entity_of(ConnectionId(connection)).is_some()
}

#[test]
fn late_configuration_finisher_is_refused_and_the_player_in_play_keeps_custody() {
    let mut state = fixture();
    // Connection 7 logged in first and is still in configuration; connection 8 logs in
    // later, finishes configuration and joins.
    tick(
        &mut state,
        &[
            login(7),
            login(8),
            // Pre-Join metadata the edge records for the held connection.
            PlayerInput::SetViewDistance {
                connection: ConnectionId(7),
                view_distance: 6,
            },
            PlayerInput::SetLocale {
                connection: ConnectionId(7),
                locale: "ru_ru".into(),
            },
            join(8),
        ],
    );
    hold_diamond(&mut state, 8);
    let entity = state.connections.entity_of(ConnectionId(8)).unwrap();
    // The first connection now finishes configuration.
    let drains = tick(&mut state, &[join(7)]);
    assert!(kicked(&drains, 7), "the late finisher gets the duplicate-login refusal");
    assert!(!in_play(&state, 7));
    assert!(state.connections.view_distance_of(ConnectionId(7)).is_none());
    assert!(state.connections.locale_of(ConnectionId(7)).is_none());
    assert!(!kicked(&drains, 8), "the player in Play is not kicked");
    assert_eq!(state.connections.entity_of(ConnectionId(8)), Some(entity));
    assert_eq!(first_slot_count(&state, 8), 3, "custody stays with the player in Play");
    assert!(state.entities.slot_of_uuid(uuid::Uuid::from_u128(PROFILE)).is_some());
    assert!(!state.offline_players.contains_key(&uuid::Uuid::from_u128(PROFILE)));
}

#[test]
fn second_finisher_is_refused_even_when_it_logged_in_later() {
    let mut state = fixture();
    // Both logged in before either finished configuration; connection 7 finishes first.
    tick(&mut state, &[login(7), login(8), join(7)]);
    hold_diamond(&mut state, 7);
    let drains = tick(&mut state, &[join(8)]);
    assert!(kicked(&drains, 8));
    assert!(!kicked(&drains, 7));
    assert!(in_play(&state, 7) && !in_play(&state, 8));
    assert_eq!(first_slot_count(&state, 7), 3);
}

#[test]
fn newer_login_still_kicks_a_session_already_in_play_and_takes_over_custody() {
    let mut state = fixture();
    tick(&mut state, &[login(7), join(7)]);
    hold_diamond(&mut state, 7);
    // The newer login kicks the incumbent when it finishes logging in ...
    let drains = tick(&mut state, &[login(8)]);
    assert!(kicked(&drains, 7));
    assert!(!in_play(&state, 7));
    // ... and its own Join then takes the saved player over exactly once.
    let drains = tick(&mut state, &[join(8)]);
    assert!(!kicked(&drains, 8));
    assert!(in_play(&state, 8));
    assert_eq!(first_slot_count(&state, 8), 3);
    assert!(!state.offline_players.contains_key(&uuid::Uuid::from_u128(PROFILE)));
}
