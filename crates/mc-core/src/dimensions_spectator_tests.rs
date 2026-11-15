use super::*;

fn cameras(
    output: &crate::boundary::TickOutput,
    dimension: Dimension,
) -> &rustc_hash::FxHashMap<EntityId, EntityId> {
    if dimension == Dimension::Overworld {
        &output.snapshot.cameras
    } else {
        &output.snapshot.dimensions[&dimension].snapshot.cameras
    }
}

#[test]
fn spectator_global_target_lookup_transfers_before_publishing_camera() {
    let (mut state, alice, bob) = worlds();
    command(&mut state, 1, "gamemode spectator");
    for target in [
        "Bob".to_owned(),
        uuid::Uuid::from_u128(2).to_string(),
        "@a[name=Bob,limit=1]".to_owned(),
    ] {
        let (output, drains) = command(&mut state, 1, &format!("spectate {target}"));
        assert_eq!(
            travel::locate(&state, alice).unwrap().0,
            Dimension::Nether,
            "{target}"
        );
        assert_eq!(cameras(&output, Dimension::Nether).get(&alice), Some(&bob));
        assert!(cameras(&output, Dimension::Overworld).is_empty());
        assert!(output.snapshot.entities.get(alice).is_none());
        assert!(
            drains
                .iter()
                .filter(|(id, _)| *id == ConnectionId(1))
                .any(|(_, changes)| changes.iter().any(|c| matches!(
                    c,
                    PlayerChange::DimensionChanged {
                        dimension: Dimension::Nether,
                        ..
                    }
                )))
        );
        assert!(
            !drains
                .iter()
                .filter(|(id, _)| *id == ConnectionId(1))
                .any(|(_, changes)| changes
                    .iter()
                    .any(|c| matches!(c, PlayerChange::PositionCorrected { .. }))),
            "dimension reset already carries the initial binding correction"
        );
        command(&mut state, 1, "execute in overworld run tp Alice 8 150 8");
    }
}

#[test]
fn spectator_target_dimension_transfer_moves_viewer_then_releases_in_same_publication() {
    let (mut state, alice, bob) = worlds();
    command(&mut state, 1, "gamemode spectator");
    command(&mut state, 1, "spectate Bob");
    let (output, drains) = command(&mut state, 1, "execute in the_end run tp Bob 9 150 9");
    for id in [alice, bob] {
        let (dimension, slot) = travel::locate(&state, id).unwrap();
        assert_eq!(dimension, Dimension::End);
        assert_eq!(
            travel::world(&state, dimension).unwrap().entities.pos(slot),
            [9.0, 150.0, 9.0]
        );
    }
    for dimension in [Dimension::Overworld, Dimension::Nether, Dimension::End] {
        assert!(cameras(&output, dimension).is_empty());
    }
    assert!(
        drains
            .iter()
            .filter(|(id, _)| *id == ConnectionId(1))
            .any(|(_, changes)| changes.iter().any(|c| matches!(
                c,
                PlayerChange::DimensionChanged {
                    dimension: Dimension::End,
                    ..
                }
            )))
    );
    command(&mut state, 2, "tp 12 150 12");
    let world = travel::world(&state, Dimension::End).unwrap();
    assert_ne!(
        world.entities.pos(world.entities.slot_of(alice).unwrap()),
        world.entities.pos(world.entities.slot_of(bob).unwrap())
    );
    assert!(
        world.cameras.is_empty(),
        "ordinary camera never rebinds after target transfer"
    );
    let (output, _) = command(&mut state, 1, "spectate Bob");
    assert_eq!(cameras(&output, Dimension::End).get(&alice), Some(&bob));
}

#[test]
fn spectator_viewer_dimension_transfer_clears_source_binding_immediately() {
    let (mut state, alice, bob) = worlds();
    command(&mut state, 1, "gamemode spectator");
    command(&mut state, 1, "spectate Bob");
    let (output, _) = command(&mut state, 1, "execute in overworld run tp Alice 10 150 10");
    assert_eq!(
        travel::locate(&state, alice).unwrap().0,
        Dimension::Overworld
    );
    assert_eq!(travel::locate(&state, bob).unwrap().0, Dimension::Nether);
    assert!(cameras(&output, Dimension::Overworld).is_empty());
    assert!(cameras(&output, Dimension::Nether).is_empty());
    crate::tick::run_tick(&mut state, &[]);
    assert!(state.cameras.is_empty());
    assert!(
        state.other_dimensions[&Dimension::Nether]
            .cameras
            .is_empty()
    );
}

#[test]
fn spectator_global_refusals_do_not_transfer_and_spatial_selectors_stay_local() {
    let (mut state, alice, _) = worlds();
    command(&mut state, 1, "spectate Bob");
    assert_eq!(
        travel::locate(&state, alice).unwrap().0,
        Dimension::Overworld
    );
    command(&mut state, 1, "gamemode spectator");
    for text in [
        "spectate Nobody",
        "spectate Alice",
        "spectate @a",
        "spectate @e[type=player,name=Bob,distance=..5,limit=1]",
    ] {
        let (output, _) = command(&mut state, 1, text);
        assert_eq!(
            travel::locate(&state, alice).unwrap().0,
            Dimension::Overworld,
            "{text}"
        );
        assert!(cameras(&output, Dimension::Overworld).is_empty());
        assert!(cameras(&output, Dimension::Nether).is_empty());
    }
}

#[test]
fn spectator_explicit_same_world_teleport_releases_only_after_valid_destination() {
    let (mut state, alice, bob) = worlds();
    command(&mut state, 1, "gamemode spectator");
    command(&mut state, 1, "spectate Bob");
    let (output, _) = command(&mut state, 1, "tp 30000000 150 0");
    assert_eq!(cameras(&output, Dimension::Nether).get(&alice), Some(&bob));
    let (output, _) = command(&mut state, 1, "tp 11 150 11");
    assert!(cameras(&output, Dimension::Nether).is_empty());
    crate::tick::run_tick(&mut state, &[]);
    let world = travel::world(&state, Dimension::Nether).unwrap();
    assert!(world.cameras.is_empty());
    assert_ne!(
        world.entities.pos(world.entities.slot_of(alice).unwrap()),
        world.entities.pos(world.entities.slot_of(bob).unwrap())
    );
}
