use super::*;
use crate::boundary::ConnectionId;
use crate::commands::Command;
use mc_data::Identifier;

#[test]
fn vanilla_grammar_defaults_units_and_permissions() {
    for (text, hand, animation, ticks) in [
        ("swing", Hand::Main, SwingKind::Whack, 6),
        ("swing @s", Hand::Main, SwingKind::Whack, 6),
        ("swing @s offhand", Hand::Off, SwingKind::Whack, 6),
        ("swing @s mainhand stab", Hand::Main, SwingKind::Stab, 6),
        ("swing @s offhand none 1.5s", Hand::Off, SwingKind::None, 30),
        (
            "swing @s mainhand whack 1d",
            Hand::Main,
            SwingKind::Whack,
            24000,
        ),
        (
            "swing @s mainhand stab 1201t",
            Hand::Main,
            SwingKind::Stab,
            1201,
        ),
    ] {
        let Command::Swing(command) = crate::commands::parse(text, true).unwrap() else {
            panic!("{text}")
        };
        assert_eq!(
            (command.hand, command.animation, command.duration_ticks),
            (hand, animation, ticks)
        );
    }
    for text in [
        "swing @s leg",
        "swing @s mainhand slash",
        "swing @s mainhand stab 0",
        "swing @s mainhand stab -1",
        "swing @s mainhand stab 1x",
        "swing @s mainhand stab 1 extra",
    ] {
        assert!(crate::commands::parse(text, true).is_err(), "{text}");
    }
    assert!(crate::commands::parse("swing", false).is_err());
    assert!(crate::commands::root_permitted("swing", 2));
    assert!(!crate::commands::root_permitted("swing", 1));
}

#[test]
fn swing_command_emits_one_event_per_living_target() {
    let (mut state, actor, centre) = crate::hostile_tests::setup(0.0);
    let zombie = state
        .registry
        .entity_types
        .id_of(&Identifier::parse("minecraft:zombie").unwrap())
        .unwrap();
    let first = state.spawn_mob(zombie, [centre[0] + 2., centre[1], centre[2]], 20.);
    let second = state.spawn_mob(zombie, [centre[0] - 2., centre[1], centre[2]], 20.);
    let command = crate::commands::parse("swing @e[type=zombie] offhand stab 1.5s", true).unwrap();
    let mut changes = ChangeSet::default();
    let result = crate::command_execution::execute(
        &mut state,
        ConnectionId(1),
        actor,
        &command,
        &mut changes,
    );
    assert_eq!(result, crate::command_execution::CommandResult::Success(2));
    let mut ids = Vec::new();
    for event in changes.entity_events {
        let EntityEvent::Swing {
            entity,
            hand,
            animation,
            duration_ticks,
        } = event
        else {
            panic!("unexpected event")
        };
        assert_eq!(
            (hand, animation, duration_ticks),
            (Hand::Off, SwingKind::Stab, 30)
        );
        ids.push(entity);
    }
    ids.sort();
    let mut expected = vec![first, second];
    expected.sort();
    assert_eq!(ids, expected);
}

#[test]
fn bare_swing_targets_the_executing_player() {
    let (mut state, actor, _) = crate::hostile_tests::setup(0.0);
    let command = crate::commands::parse("swing", true).unwrap();
    let mut changes = ChangeSet::default();
    assert_eq!(
        crate::command_execution::execute(
            &mut state,
            ConnectionId(1),
            actor,
            &command,
            &mut changes
        ),
        crate::command_execution::CommandResult::Success(1)
    );
    assert!(matches!(
        changes.entity_events.as_slice(),
        [EntityEvent::Swing { entity, hand: Hand::Main, animation: SwingKind::Whack, duration_ticks: 6 }] if *entity == actor
    ));
}
