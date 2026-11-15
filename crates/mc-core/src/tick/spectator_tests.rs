use super::*;
use crate::boundary::{PlayerChange, PlayerInput};
use crate::tick::run_tick;

fn join(connection: u64, name: &str) -> PlayerInput {
    PlayerInput::Join {
        connection: ConnectionId(connection),
        uuid: uuid::Uuid::from_u128(connection.into()),
        name: name.into(),
        permission_level: 2,
    }
}

fn fixture() -> (TickState, EntityId, EntityId) {
    let registry = Arc::new(crate::registry::CoreRegistry::load(None).unwrap());
    let world = crate::horde_bench::flat_world(&registry);
    let mut state = TickState::new(registry, world, 2, 512);
    run_tick(&mut state, &[join(1, "PovRed"), join(2, "CamSpec")]);
    let target = state.connections.entity_of(ConnectionId(1)).unwrap();
    let viewer = state.connections.entity_of(ConnectionId(2)).unwrap();
    command(&mut state, 2, "gamemode spectator");
    (state, viewer, target)
}

fn command(state: &mut TickState, connection: u64, text: &str) -> Vec<PlayerChange> {
    let (_, drains, _) = run_tick(
        state,
        &[PlayerInput::Command {
            connection: ConnectionId(connection),
            command: text.into(),
            permission_level: 2,
        }],
    );
    drains
        .into_iter()
        .filter(|(id, _)| *id == ConnectionId(connection))
        .flat_map(|(_, changes)| changes)
        .collect()
}

fn position(state: &TickState, id: EntityId) -> [f64; 3] {
    state.entities.pos(state.entities.slot_of(id).unwrap())
}

#[test]
fn spectator_command_permissions_and_rejected_targets_do_not_bind() {
    assert!(crate::commands::parse("spectate PovRed", false).is_err());
    assert!(!crate::commands::root_permitted("spectate", 1));
    assert!(crate::commands::root_permitted("spectate", 2));
    let (mut state, viewer, _) = fixture();
    for command_text in [
        "spectate Nobody",
        "spectate CamSpec",
        "spectate @a",
        "spectate PovRed Nobody",
    ] {
        command(&mut state, 2, command_text);
        assert!(!state.cameras.contains_key(&viewer), "{command_text}");
    }
    command(&mut state, 2, "gamemode survival");
    command(&mut state, 2, "spectate PovRed");
    assert!(state.cameras.is_empty());
}

#[test]
fn spectator_tick_follows_target_and_publishes_tracking_anchor_without_teleport_spam() {
    let (mut state, viewer, target) = fixture();
    command(&mut state, 1, "spectate PovRed CamSpec");
    assert_eq!(state.cameras.get(&viewer), Some(&target));
    for x in [32.5, 64.5, 96.5] {
        command(&mut state, 1, &format!("tp {x} 70 8.5"));
        let (output, drains, _) = run_tick(&mut state, &[]);
        assert_eq!(position(&state, viewer), position(&state, target));
        assert_eq!(
            output.snapshot.entities.get(viewer).unwrap().position,
            position(&state, target)
        );
        assert_eq!(output.snapshot.cameras.get(&viewer), Some(&target));
        assert!(
            drains
                .iter()
                .filter(|(id, _)| *id == ConnectionId(2))
                .all(|(_, changes)| changes
                    .iter()
                    .all(|change| !matches!(change, PlayerChange::PositionCorrected { .. })))
        );
    }
    command(&mut state, 2, "spectate");
    assert!(state.cameras.is_empty());
    assert_eq!(position(&state, viewer), position(&state, target));
}

#[test]
fn spectator_target_death_and_disconnect_release_at_last_followed_position() {
    for disconnect in [false, true] {
        let (mut state, viewer, target) = fixture();
        command(&mut state, 2, "spectate PovRed");
        command(&mut state, 1, "tp 40.5 70 8.5");
        let last = position(&state, viewer);
        let input = if disconnect {
            PlayerInput::Leave {
                connection: ConnectionId(1),
            }
        } else {
            PlayerInput::Command {
                connection: ConnectionId(1),
                command: "kill".into(),
                permission_level: 2,
            }
        };
        let (output, drains, _) = run_tick(&mut state, &[input]);
        assert!(output.snapshot.cameras.is_empty());
        assert!(!state.cameras.contains_key(&viewer));
        assert_eq!(position(&state, viewer), last);
        assert!(drains.iter().filter(|(id, _)| *id == ConnectionId(2)).any(|(_, changes)| changes.iter().any(|change| matches!(change, PlayerChange::PositionCorrected { position, .. } if *position == last))));
        if disconnect {
            run_tick(&mut state, &[join(1, "PovRed")]);
            assert_ne!(
                state.connections.entity_of(ConnectionId(1)).unwrap(),
                target
            );
        } else {
            run_tick(
                &mut state,
                &[PlayerInput::Respawn {
                    connection: ConnectionId(1),
                }],
            );
        }
        assert!(
            state.cameras.is_empty(),
            "vanilla binding never reattaches itself"
        );
    }
}

#[test]
fn spectator_named_battle_binding_reconnects_and_stops_on_sneak_mode_or_command() {
    for stop_kind in ["sneak", "mode", "command", "disconnect"] {
        let (mut state, viewer, target) = fixture();
        crate::battle::live_players::spectate(
            &mut state,
            &mut Default::default(),
            "CamSpec",
            "PovRed",
        )
        .unwrap();
        run_tick(
            &mut state,
            &[PlayerInput::Leave {
                connection: ConnectionId(1),
            }],
        );
        assert!(state.cameras.is_empty());
        run_tick(&mut state, &[join(1, "PovRed")]);
        let replacement = state.connections.entity_of(ConnectionId(1)).unwrap();
        assert_ne!(replacement, target);
        assert_eq!(state.cameras.get(&viewer), Some(&replacement));
        match stop_kind {
            "sneak" => {
                run_tick(
                    &mut state,
                    &[PlayerInput::Sneak {
                        connection: ConnectionId(2),
                        sneaking: true,
                    }],
                );
            }
            "mode" => {
                command(&mut state, 2, "gamemode creative");
            }
            "command" => {
                command(&mut state, 2, "spectate");
            }
            "disconnect" => {
                run_tick(
                    &mut state,
                    &[PlayerInput::Leave {
                        connection: ConnectionId(2),
                    }],
                );
            }
            _ => unreachable!(),
        }
        assert!(state.cameras.is_empty());
        run_tick(
            &mut state,
            &[PlayerInput::Leave {
                connection: ConnectionId(1),
            }],
        );
        run_tick(&mut state, &[join(1, "PovRed")]);
        if stop_kind == "disconnect" {
            run_tick(&mut state, &[join(2, "CamSpec")]);
        }
        command(&mut state, 2, "gamemode spectator");
        assert!(
            state.cameras.is_empty(),
            "{stop_kind} must end named intent"
        );
    }
}

#[test]
fn spectator_named_binding_releases_death_then_rebinds_respawn() {
    let (mut state, viewer, target) = fixture();
    crate::battle::live_players::spectate(&mut state, &mut Default::default(), "CamSpec", "PovRed")
        .unwrap();
    command(&mut state, 1, "kill");
    assert!(state.cameras.is_empty());
    run_tick(
        &mut state,
        &[PlayerInput::Respawn {
            connection: ConnectionId(1),
        }],
    );
    // Respawn admission requests its destination chunks before reviving the body.
    state.world.ensure_blocking(&state.desired_chunks());
    run_tick(&mut state, &[]);
    assert!(alive(&state, target));
    assert_eq!(state.cameras.get(&viewer), Some(&target));
    assert_eq!(position(&state, viewer), position(&state, target));
}

#[test]
fn spectator_mode_exit_cancels_named_intent_even_if_restored_in_same_tick() {
    let (mut state, viewer, _) = fixture();
    crate::battle::live_players::spectate(&mut state, &mut Default::default(), "CamSpec", "PovRed")
        .unwrap();
    let inputs = ["gamemode creative", "gamemode spectator"].map(|command| PlayerInput::Command {
        connection: ConnectionId(2),
        command: command.into(),
        permission_level: 2,
    });
    run_tick(&mut state, &inputs);
    assert!(!state.cameras.contains_key(&viewer));
    run_tick(&mut state, &[]);
    assert!(!state.cameras.contains_key(&viewer));
}

#[test]
fn spectator_mode_projects_invisibility_without_destroying_potion_state() {
    let (mut state, viewer, _) = fixture();
    let flags = |state: &TickState| {
        state
            .entities
            .row(state.entities.slot_of(viewer).unwrap())
            .appearance
            .shared_flags
    };
    assert_ne!(
        flags(&state) & 0x20,
        0,
        "mode entry publishes spectator invisibility"
    );
    command(&mut state, 2, "effect give @s minecraft:invisibility 30");
    let slot = state.entities.slot_of(viewer).unwrap();
    assert!(
        state
            .entities
            .row(slot)
            .appearance
            .effect_particles
            .is_empty()
    );
    assert!(!state.players[&viewer].active_effects.entries.is_empty());
    command(&mut state, 2, "gamemode survival");
    assert_ne!(
        flags(&state) & 0x20,
        0,
        "leaving spectator preserves potion invisibility"
    );
    let slot = state.entities.slot_of(viewer).unwrap();
    assert!(
        !state
            .entities
            .row(slot)
            .appearance
            .effect_particles
            .is_empty()
    );
    command(&mut state, 2, "effect clear @s");
    assert_eq!(flags(&state) & 0x20, 0);
    command(&mut state, 2, "gamemode spectator");
    command(&mut state, 2, "effect clear @s");
    assert_ne!(
        flags(&state) & 0x20,
        0,
        "effect clearing cannot make a spectator visible"
    );
    // Session restoration supplies the mode before the first appearance pass.
    let slot = state.entities.slot_of(viewer).unwrap();
    Arc::make_mut(&mut state.entities.row_mut(slot).appearance).shared_flags &= !0x20;
    state.world.ensure_blocking(&state.desired_chunks());
    let (output, _, _) = run_tick(&mut state, &[]);
    assert_ne!(
        output
            .snapshot
            .entities
            .get(viewer)
            .unwrap()
            .appearance
            .shared_flags
            & 0x20,
        0
    );
    command(&mut state, 2, "gamemode survival");
    assert_eq!(
        flags(&state) & 0x20,
        0,
        "no potion means mode exit clears invisibility"
    );
}
