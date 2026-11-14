use super::*;
use crate::boundary::ConnectionId;
use crate::commands::Command;
use mc_data::Identifier;

fn run(
    state: &mut TickState,
    actor: EntityId,
    text: &str,
) -> (crate::command_execution::CommandResult, ChangeSet) {
    let command = crate::commands::parse(text, true).unwrap();
    let mut changes = ChangeSet::default();
    let result =
        crate::command_execution::execute(state, ConnectionId(1), actor, &command, &mut changes);
    (result, changes)
}

#[test]
fn swing_command_parses_hand_type_and_duration_and_is_operator_only() {
    match crate::commands::parse("swing @e[type=zombie] offhand stab 23", true) {
        Ok(Command::Swing(command)) => {
            assert_eq!(
                (command.hand, command.animation, command.duration_ticks),
                (Hand::Off, SwingKind::Stab, 23)
            );
        }
        other => panic!("{other:?}"),
    }
    for text in [
        "swing @s",
        "swing @s mainhand whack",
        "swing @s leg whack 6",
        "swing @s mainhand slash 6",
        "swing @s mainhand whack 0",
        "swing @s mainhand whack 1201",
        "swing @s mainhand whack -3",
        "swing @s mainhand whack 6 extra",
    ] {
        assert!(crate::commands::parse(text, true).is_err(), "{text}");
    }
    assert!(crate::commands::parse("swing @s mainhand whack 6", false).is_err());
    assert!(crate::commands::root_permitted("swing", 2));
    assert!(!crate::commands::root_permitted("swing", 1));
}

#[test]
fn swing_command_emits_one_swing_event_per_living_target() {
    let (mut state, actor, centre) = crate::hostile_tests::setup(0.0);
    let zombie = state
        .registry
        .entity_types
        .id_of(&Identifier::parse("minecraft:zombie").unwrap())
        .unwrap();
    let first = state.spawn_mob(zombie, [centre[0] + 2., centre[1], centre[2]], 20.);
    let second = state.spawn_mob(zombie, [centre[0] - 2., centre[1], centre[2]], 20.);
    let (result, changes) = run(&mut state, actor, "swing @e[type=zombie] mainhand whack 21");
    assert_eq!(result, crate::command_execution::CommandResult::Success(2));
    let mut swung: Vec<_> = changes
        .entity_events
        .iter()
        .map(|event| match *event {
            EntityEvent::Swing {
                entity,
                hand,
                animation,
                duration_ticks,
            } => {
                assert_eq!(
                    (hand, animation, duration_ticks),
                    (Hand::Main, SwingKind::Whack, 21)
                );
                entity
            }
            other => panic!("unexpected event {other:?}"),
        })
        .collect();
    swung.sort();
    let mut expected = vec![first, second];
    expected.sort();
    assert_eq!(swung, expected);
    let (result, changes) = run(&mut state, actor, "swing @e[type=skeleton] offhand none 12");
    assert_eq!(result, crate::command_execution::CommandResult::Failed);
    assert!(
        changes.entity_events.is_empty(),
        "nothing to swing changes nothing"
    );
}
