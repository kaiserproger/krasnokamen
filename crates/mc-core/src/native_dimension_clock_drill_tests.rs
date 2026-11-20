//! Root game-clock authority must survive travel's post-dimension refresh.
use super::*;
use crate::dimensions::Dimension;

fn teleport(drill: &mut Drill, dimension: Dimension) {
    drill
        .run(&[PlayerInput::Command {
            connection: ConnectionId(1),
            permission_level: 4,
            command: format!(
                "execute in {} run tp durable 34.5 200 40.5",
                dimension.name()
            ),
        }])
        .unwrap();
    let world = if dimension == Dimension::Overworld {
        &drill.state
    } else {
        &drill.state.other_dimensions[&dimension]
    };
    assert!(
        world.connections.entity_of(ConnectionId(1)).is_some(),
        "command must actually arrive in {dimension:?}"
    );
}

#[test]
fn game_clock_hold_unfreeze_dimension_travel_uses_root_prefix_and_reopens_nether() {
    check_dimension(Dimension::Nether);
}

#[test]
fn game_clock_hold_unfreeze_dimension_travel_uses_root_prefix_and_reopens_end() {
    check_dimension(Dimension::End);
}

fn check_dimension(dimension: Dimension) {
    let directory = tempfile::tempdir().unwrap();
    let mut drill = fixture(directory.path());
    teleport(&mut drill, dimension);
    // A resident, once-active destination, then a real return to overworld.
    drill
        .state
        .other_dimensions
        .get_mut(&dimension)
        .unwrap()
        .world
        .ensure_blocking(&[(CHUNK, 0)].into_iter().collect());
    drill.run(&[]).unwrap();
    teleport(&mut drill, Dimension::Overworld);
    drill.run(&[]).unwrap();
    drill.state.tick_control.set_frozen(true);
    for _ in 0..7 {
        drill.run(&[]).unwrap();
    }
    let inactive = drill.state.other_dimensions[&dimension]
        .world
        .debt_checkpoint(drill.state.tick)
        .into_iter()
        .find(|image| image.chunk == [CHUNK.x, CHUNK.z])
        .unwrap();
    assert!(
        !inactive.active,
        "the destination must really be inactive before arrival: {dimension:?}, debt={inactive:?}, players={}",
        drill.state.other_dimensions[&dimension].players.len()
    );
    let held = drill.state.tick - drill.state.tick_control.game_ticks(drill.state.tick);
    let arrival = drill.state.tick + 1;
    let expected = inactive.remaining + (arrival - 1 - inactive.last_sim)
        - (held - inactive.held_anchor.unwrap());
    drill.state.tick_control.set_frozen(false);
    teleport(&mut drill, dimension);
    let active = drill.state.other_dimensions[&dimension]
        .world
        .debt_checkpoint(drill.state.tick)
        .into_iter()
        .find(|image| image.chunk == [CHUNK.x, CHUNK.z])
        .unwrap();
    assert!(active.active, "travel must activate the destination");
    assert_eq!(
        active.remaining, expected,
        "only dormant running time is owed in {dimension:?}"
    );
    assert_eq!(
        active.held_anchor,
        Some(held),
        "post-travel checkpoint uses the root H"
    );
    drill.checkpoint();
    let config = drill.config.clone();
    drop(drill);
    let recovered = crate::native_checkpoint::recover_offline(&config).unwrap();
    let debt = recovered.dimensions[&dimension].sim_debt[&[CHUNK.x, CHUNK.z]];
    assert_eq!(debt.held_anchor, Some(held));
    assert_eq!(debt.remaining, expected);
}
