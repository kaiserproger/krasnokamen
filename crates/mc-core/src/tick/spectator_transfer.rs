//! Camera commands and transfers use the existing shared dimension commit boundary.
use super::*;
use crate::boundary::ChangeSet;
use crate::dimensions::{Dimension, commands, travel};
use std::collections::BTreeMap;

fn resolve(
    state: &TickState,
    request: &travel::Request,
    target: &Target,
    player: bool,
    origin: [f64; 3],
) -> Result<EntityId, String> {
    validate_selector(target, player).map_err(str::to_owned)?;
    let ids = commands::resolve(state, request, target, origin)?;
    if let [id] = ids.as_slice()
        && let Some((dimension, _)) = travel::locate(state, *id)
        && (!player
            || travel::world(state, dimension)
                .unwrap()
                .players
                .contains_key(id))
    {
        return Ok(*id);
    }
    Err(if player {
        "argument.entity.notfound.player"
    } else {
        "argument.entity.notfound.entity"
    }
    .into())
}

fn world_mut(state: &mut TickState, dimension: Dimension) -> &mut TickState {
    if state.dimension == dimension {
        state
    } else {
        state
            .other_dimensions
            .get_mut(&dimension)
            .expect("located world")
    }
}

fn label(state: &TickState, id: EntityId) -> mc_nbt::Tag {
    let (dimension, _) = travel::locate(state, id).expect("resolved entity");
    command_execution::entity_name(travel::world(state, dimension).unwrap(), id)
}

pub(crate) fn execute(
    state: &mut TickState,
    request: travel::Request,
    changes: &mut BTreeMap<Dimension, ChangeSet>,
) -> Result<CommandResult, String> {
    let Command::Spectate { target, player } = &request.command else {
        unreachable!("spectate request")
    };
    let source = travel::world(state, request.dimension).ok_or("Unknown dimension")?;
    let origin = state
        .command_context
        .current
        .map(|context| context.position)
        .or_else(|| crate::command_console::pose(source, request.executor).map(|pose| pose.0))
        .ok_or("argument.entity.notfound.entity")?;
    let target = target
        .as_ref()
        .map(|target| resolve(state, &request, target, false, origin))
        .transpose()?;
    let viewer = resolve(state, &request, player, true, origin)?;
    let (from, viewer_slot) = travel::locate(state, viewer).expect("resolved viewer");
    let destination = target
        .map(|id| travel::locate(state, id).expect("resolved target").0)
        .unwrap_or(from);
    let viewer_world = travel::world(state, from).unwrap();
    let target_world = travel::world(state, destination).unwrap();
    if let Err(key) = validate_binding(viewer_world, target_world, viewer, target) {
        let arguments = match key {
            "commands.spectate.not_spectator" => vec![label(state, viewer)],
            "commands.spectate.cannot_spectate" => {
                vec![label(state, target.expect("target refusal"))]
            }
            _ => Vec::new(),
        };
        command_execution::feedback(state, request.connection, key, arguments);
        return Ok(CommandResult::Failed);
    }
    if from != destination {
        let slot = target_world
            .entities
            .slot_of(target.expect("cross-world target"))
            .unwrap();
        let position = target_world.entities.pos(slot);
        let [yaw, pitch, _] = viewer_world.entities.rot(viewer_slot);
        travel::change_dimension(state, viewer, destination, position, [yaw, pitch], changes)?;
    }
    let world = world_mut(state, destination);
    // DimensionChanged already carries the cross-world position synchronization.
    apply_binding(world, viewer, target, from == destination).map_err(str::to_owned)?;
    crate::battle::live_players::stop_camera(world, viewer);
    changes
        .entry(destination)
        .or_default()
        .entities_appearance_changed
        .push(viewer);
    let (key, arguments) = match target {
        Some(id) => ("commands.spectate.success.started", vec![label(state, id)]),
        None => ("commands.spectate.success.stopped", Vec::new()),
    };
    command_execution::success_feedback(state, request.connection, key, arguments);
    Ok(CommandResult::Success(1))
}

/// Vanilla target travel moves attached viewers through the same transition and
/// releases their cameras. Detach before moving any body so cycles cannot recur.
pub(crate) fn detach_for_transfer(state: &mut TickState, target: EntityId) -> Vec<EntityId> {
    let followers: Vec<_> = state
        .cameras
        .iter()
        .filter_map(|(&viewer, &subject)| (subject == target).then_some(viewer))
        .collect();
    for &viewer in &followers {
        stop(state, viewer);
    }
    stop(state, target);
    followers
}
