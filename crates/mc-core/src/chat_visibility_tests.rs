//! The client's chat setting: `Full` gets everything, `System` ("commands
//! only") gets system text but no player chat, `Hidden` gets neither and
//! cannot send chat.
use crate::boundary::{ChatVisibility, ConnectionId, PlayerChange, PlayerInput};
use crate::tick::{TickState, run_tick};

const FULL: ConnectionId = ConnectionId(1);
const OTHER_FULL: ConnectionId = ConnectionId(2);
const SYSTEM: ConnectionId = ConnectionId(3);
const HIDDEN: ConnectionId = ConnectionId(4);

/// The text each of the four players was sent by one tick, in order:
/// `[FULL, OTHER_FULL, SYSTEM, HIDDEN]`.
type Heard = [Vec<String>; 4];

/// Run one tick and collect the text lines it delivered to each player.
fn tick(state: &mut TickState, inputs: &[PlayerInput]) -> Heard {
    let (_, drains, _) = run_tick(state, inputs);
    [FULL, OTHER_FULL, SYSTEM, HIDDEN].map(|connection| {
        drains
            .iter()
            .filter(|(id, _)| *id == connection)
            .flat_map(|(_, changes)| changes)
            .flat_map(|change| match change {
                PlayerChange::TextBurst { lines } => lines.iter().collect(),
                other => vec![other],
            })
            .filter(|change| change.is_text())
            .map(|change| format!("{change:?}"))
            .collect()
    })
}

fn join(state: &mut TickState, connection: ConnectionId, name: &str, visibility: ChatVisibility) {
    tick(
        state,
        &[
            PlayerInput::SetChatVisibility {
                connection,
                visibility,
            },
            PlayerInput::Join {
                connection,
                uuid: uuid::Uuid::from_u128(u128::from(connection.0)),
                name: name.into(),
                permission_level: 4,
            },
        ],
    );
}

/// Four players: connection 1 `Full` (from the shared fixture), then another
/// `Full`, a `System` and a `Hidden` one.
fn world() -> TickState {
    let (mut state, _, _) = crate::hostile_tests::setup(8.0);
    join(&mut state, OTHER_FULL, "full2", ChatVisibility::Full);
    join(&mut state, SYSTEM, "system", ChatVisibility::System);
    join(&mut state, HIDDEN, "hidden", ChatVisibility::Hidden);
    // Let the join announcements settle so each test sees only its own text.
    tick(&mut state, &[]);
    state
}

fn say(state: &mut TickState, connection: ConnectionId, text: &str) -> Heard {
    tick(
        state,
        &[PlayerInput::Chat {
            connection,
            message: text.into(),
        }],
    )
}

fn command(state: &mut TickState, connection: ConnectionId, line: &str) -> Heard {
    tick(
        state,
        &[PlayerInput::Command {
            connection,
            command: line.into(),
            permission_level: 4,
        }],
    )
}

fn contains(lines: &[String], needle: &str) -> bool {
    lines.iter().any(|line| line.contains(needle))
}

#[test]
fn public_chat_reaches_only_full_clients() {
    let mut state = world();
    let [full, other_full, system, hidden] = say(&mut state, FULL, "hello there");
    assert!(contains(&full, "hello there"), "{full:?}");
    assert!(contains(&other_full, "hello there"), "{other_full:?}");
    assert!(
        system.is_empty(),
        "commands-only clients get no chat: {system:?}"
    );
    assert!(hidden.is_empty(), "hidden clients get no chat: {hidden:?}");
}

#[test]
fn a_hidden_client_cannot_chat_and_is_told_so_in_red() {
    let mut state = world();
    let [full, other_full, system, hidden] = say(&mut state, HIDDEN, "can anyone hear me");
    assert!(
        full.is_empty() && other_full.is_empty() && system.is_empty(),
        "{full:?} {other_full:?} {system:?}"
    );
    assert_eq!(hidden.len(), 1, "{hidden:?}");
    assert!(
        hidden[0].contains("chat.disabled.options") && hidden[0].contains("red"),
        "{hidden:?}"
    );

    // A commands-only client may still chat: that is the setting's name.
    let [full, ..] = say(&mut state, SYSTEM, "psst");
    assert!(contains(&full, "psst"), "{full:?}");
}

#[test]
fn system_text_reaches_commands_only_clients_but_not_hidden_ones() {
    let mut state = world();
    let [_, _, system, hidden] = command(&mut state, SYSTEM, "time query daytime");
    assert!(
        contains(&system, "commands.time.query"),
        "command feedback is a system message: {system:?}"
    );
    assert!(hidden.is_empty());
    let [.., hidden] = command(&mut state, HIDDEN, "time query daytime");
    assert!(
        hidden.is_empty(),
        "a hidden client sees no feedback either: {hidden:?}"
    );
}

#[test]
fn say_and_private_messages_are_chat_not_system_text() {
    let mut state = world();
    let [full, other_full, system, hidden] = command(&mut state, FULL, "say attention");
    assert!(contains(&full, "attention"), "{full:?}");
    assert!(contains(&other_full, "attention"), "{other_full:?}");
    assert!(
        system.is_empty() && hidden.is_empty(),
        "{system:?} {hidden:?}"
    );

    let [full, _, system, _] = command(&mut state, FULL, "msg system secret");
    assert!(
        system.is_empty(),
        "a commands-only client gets no private chat: {system:?}"
    );
    assert!(
        contains(&full, "commands.message.display.outgoing"),
        "the sender still sees the confirmation: {full:?}"
    );
    let [_, _, system, _] = command(&mut state, SYSTEM, "msg full2 secret");
    assert!(
        !contains(&system, "commands.message.display.outgoing"),
        "a commands-only sender is not sent its own chat copy: {system:?}"
    );
    let [_, other_full, ..] = command(&mut state, FULL, "msg full2 hello");
    assert!(
        contains(&other_full, "commands.message.display.incoming"),
        "{other_full:?}"
    );
}

#[test]
fn the_setting_changes_in_place_and_resets_when_the_player_leaves() {
    let mut state = world();
    tick(
        &mut state,
        &[PlayerInput::SetChatVisibility {
            connection: SYSTEM,
            visibility: ChatVisibility::Full,
        }],
    );
    let [_, _, system, _] = say(&mut state, FULL, "now you hear");
    assert!(contains(&system, "now you hear"), "{system:?}");

    tick(&mut state, &[PlayerInput::Leave { connection: HIDDEN }]);
    assert_eq!(
        state.connections.chat_visibility_of(HIDDEN),
        ChatVisibility::Full,
        "a connection that is gone has no setting left"
    );
}
