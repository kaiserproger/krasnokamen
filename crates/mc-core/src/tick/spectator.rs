//! Native, session-only spectator camera authority. Edge owns delivery, not binding.
use std::sync::Arc;

use super::TickState;
use crate::boundary::ConnectionId;
use crate::command_args::CommandError;
use crate::command_execution::{self, CommandResult};
use crate::commands::{Command, Target};
use mc_domain::EntityId;

pub(crate) fn parse(args: &[&str]) -> Result<Command, CommandError> {
    let (target, player) = match args {
        [] => (None, Target::Executor),
        [target] => (Some(crate::commands::target_of(target)?), Target::Executor),
        [target, player] => (
            Some(crate::commands::target_of(target)?),
            crate::commands::target_of(player)?,
        ),
        _ => return Err(CommandError::Usage("spectate [<target> [<player>]]")),
    };
    Ok(Command::Spectate { target, player })
}

fn single(target: &Target) -> bool {
    match target {
        Target::Player(_) | Target::Entity(_) | Target::Executor | Target::Nearest => true,
        Target::AllPlayers => false,
        Target::Selector(selector) => selector.limit.map_or_else(
            || {
                matches!(
                    selector.kind,
                    crate::commands::SelectorKind::Executor
                        | crate::commands::SelectorKind::Nearest
                        | crate::commands::SelectorKind::NearestEntity
                        | crate::commands::SelectorKind::Random
                )
            },
            |limit| limit == 1,
        ),
    }
}

fn validate_selector(target: &Target, player: bool) -> Result<(), &'static str> {
    if !single(target) {
        return Err(if player {
            "argument.player.toomany"
        } else {
            "argument.entity.toomany"
        });
    }
    if player
        && let Target::Selector(selector) = target
        && !matches!(
            selector.kind,
            crate::commands::SelectorKind::Nearest
                | crate::commands::SelectorKind::AllPlayers
                | crate::commands::SelectorKind::Random
                | crate::commands::SelectorKind::Executor
        )
    {
        return Err("argument.player.entities");
    }
    Ok(())
}

fn resolve_one(
    state: &TickState,
    executor: EntityId,
    target: &Target,
    player: bool,
) -> Result<EntityId, &'static str> {
    validate_selector(target, player)?;
    let missing = if player {
        "argument.entity.notfound.player"
    } else {
        "argument.entity.notfound.entity"
    };
    let ids = command_execution::resolve_target(state, executor, target).map_err(|_| missing)?;
    match ids.as_slice() {
        [id] if !player || state.players.contains_key(id) => Ok(*id),
        _ => Err(missing),
    }
}

pub(crate) fn execute(
    state: &mut TickState,
    connection: ConnectionId,
    executor: EntityId,
    target: Option<&Target>,
    player: &Target,
) -> CommandResult {
    let selected = (|| {
        let target = target
            .map(|target| resolve_one(state, executor, target, false))
            .transpose()?;
        let viewer = resolve_one(state, executor, player, true)?;
        bind(state, viewer, target)?;
        crate::battle::live_players::stop_camera(state, viewer);
        Ok::<_, &'static str>((viewer, target))
    })();
    match selected {
        Ok((_, target)) => {
            let (key, arguments) = match target {
                Some(target) => (
                    "commands.spectate.success.started",
                    vec![command_execution::entity_name(state, target)],
                ),
                None => ("commands.spectate.success.stopped", Vec::new()),
            };
            command_execution::success_feedback(state, connection, key, arguments);
            CommandResult::Success(1)
        }
        Err(key) => {
            let arguments = if key == "commands.spectate.not_spectator" {
                resolve_one(state, executor, player, true)
                    .ok()
                    .map(|id| command_execution::entity_name(state, id))
                    .into_iter()
                    .collect()
            } else if key == "commands.spectate.cannot_spectate" {
                target
                    .and_then(|target| resolve_one(state, executor, target, false).ok())
                    .map(|id| command_execution::entity_name(state, id))
                    .into_iter()
                    .collect()
            } else {
                Vec::new()
            };
            command_execution::feedback(state, connection, key, arguments);
            CommandResult::Failed
        }
    }
}

pub(crate) fn alive(state: &TickState, id: EntityId) -> bool {
    state
        .entities
        .slot_of(id)
        .is_some_and(|slot| state.entities.body(slot).alive)
        && state
            .players
            .get(&id)
            .is_none_or(|player| !player.vitals.is_dead())
}

fn validate_binding(
    state: &TickState,
    target_world: &TickState,
    viewer: EntityId,
    target: Option<EntityId>,
) -> Result<(), &'static str> {
    if target == Some(viewer) {
        return Err("commands.spectate.self");
    }
    if !state
        .players
        .get(&viewer)
        .is_some_and(|player| player.spectator())
    {
        return Err("commands.spectate.not_spectator");
    }
    if let Some(target) = target {
        let slot = target_world
            .entities
            .slot_of(target)
            .ok_or("argument.entity.notfound.entity")?;
        if !alive(target_world, target) {
            return Err("argument.entity.notfound.entity");
        }
        let type_id = match target_world.entities.body(slot).kind {
            crate::entities::EntityKind::Player { .. } => crate::entities::player_type_id(),
            crate::entities::EntityKind::Mob { type_id } => crate::blueprint::wire_type_id(type_id),
            crate::entities::EntityKind::Item { type_id }
            | crate::entities::EntityKind::ExperienceOrb { type_id } => type_id,
        };
        if !mc_data::entity_types::entity_type_contract_26_1_2_by_protocol_id(type_id)
            .is_some_and(|contract| contract.tracking_range > 0)
        {
            return Err("commands.spectate.cannot_spectate");
        }
    }
    Ok(())
}

pub(crate) fn bind(
    state: &mut TickState,
    viewer: EntityId,
    target: Option<EntityId>,
) -> Result<(), &'static str> {
    validate_binding(state, state, viewer, target)?;
    apply_binding(state, viewer, target, true)
}

fn apply_binding(
    state: &mut TickState,
    viewer: EntityId,
    target: Option<EntityId>,
    correct_position: bool,
) -> Result<(), &'static str> {
    if let Some(target) = target {
        let slot = state
            .entities
            .slot_of(target)
            .ok_or("argument.entity.notfound.entity")?;
        let viewer_slot = state
            .entities
            .slot_of(viewer)
            .ok_or("argument.entity.notfound.player")?;
        let position = state.entities.pos(slot);
        state.entities.set_pos(viewer_slot, position);
        state.entities.set_vel(viewer_slot, [0.0; 3]);
        if state.cameras.get(&viewer) != Some(&target) {
            crate::mounts::dismount(state, viewer);
            Arc::make_mut(&mut state.cameras).insert(viewer, target);
            if correct_position {
                sync_position(state, viewer);
            }
            tracing::info!(
                viewer = viewer.0,
                target = target.0,
                "Spectator camera bound"
            );
        }
    } else {
        release(state, viewer);
    }
    Ok(())
}

fn release(state: &mut TickState, viewer: EntityId) {
    if state.cameras.contains_key(&viewer) {
        Arc::make_mut(&mut state.cameras).remove(&viewer);
        sync_position(state, viewer);
        tracing::info!(viewer = viewer.0, "Spectator camera released");
    }
}

fn sync_position(state: &mut TickState, viewer: EntityId) {
    if let Some(slot) = state.entities.slot_of(viewer)
        && let Some(connection) = state.connections.entity_connection(viewer)
    {
        let [yaw, pitch, _] = state.entities.rot(slot);
        state.correct_position(connection, state.entities.pos(slot), yaw, pitch);
    }
}

pub(crate) fn stop(state: &mut TickState, viewer: EntityId) {
    crate::battle::live_players::stop_camera(state, viewer);
    release(state, viewer);
}

pub(crate) fn player_change(
    state: &mut TickState,
    viewer: EntityId,
    change: &crate::boundary::PlayerChange,
) {
    if matches!(change, crate::boundary::PlayerChange::GameModeChanged { game_mode } if *game_mode != 3)
    {
        stop(state, viewer);
    }
}

pub(crate) fn tick(state: &mut TickState) {
    crate::battle::live_players::follow_cameras(state);
    // Reuse admission scratch; every implicit stop uses the same release path.
    let mut releases = std::mem::take(&mut state.camera_releases);
    releases.extend(
        state
            .cameras
            .iter()
            .filter_map(|(&viewer, &target)| invalid(state, viewer, target).then_some(viewer)),
    );
    for viewer in releases.drain(..) {
        release(state, viewer);
    }
    state.camera_releases = releases;
    for (&viewer, &target) in state.cameras.iter() {
        let target = state
            .entities
            .slot_of(target)
            .expect("validated camera target");
        let viewer = state
            .entities
            .slot_of(viewer)
            .expect("validated camera viewer");
        state.entities.set_pos(viewer, state.entities.pos(target));
        state.entities.set_vel(viewer, [0.0; 3]);
    }
}

fn invalid(state: &TickState, viewer: EntityId, target: EntityId) -> bool {
    !alive(state, viewer)
        || !alive(state, target)
        || !state
            .players
            .get(&viewer)
            .is_some_and(|player| player.spectator())
        || state.connections.entity_connection(viewer).is_none()
}

#[path = "spectator_transfer.rs"]
pub(crate) mod transfer;

#[cfg(test)]
#[path = "spectator_tests.rs"]
mod tests;
