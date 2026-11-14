//! Operator `/swing <targets> <mainhand|offhand> <none|whack|stab> <ticks>`: makes living
//! entities play an attack presentation through the ordinary `EntityEvent::Swing`, which
//! the edge sends as the 26.3 `swing_animation` packet. Solaris staging and QA aid (battle
//! video clips are chosen client-side from hand, type and duration); vanilla has no such
//! command. It changes no gameplay state.
use crate::boundary::{ChangeSet, EntityEvent, EntityId, Hand, SwingKind};
use crate::commands::{CommandError, Target, target_of};
use crate::tick::TickState;

const USAGE: &str = "swing <targets> <mainhand|offhand> <none|whack|stab> <ticks>";
/// Upper bound of one presentation: one minute.
const MAX_TICKS: u16 = 1200;

#[derive(Debug, Clone, PartialEq)]
pub struct SwingCommand {
    pub(crate) target: Target,
    hand: Hand,
    animation: SwingKind,
    duration_ticks: u16,
}

pub(crate) fn parse(args: &[&str]) -> Result<SwingCommand, CommandError> {
    let [target, hand, animation, ticks] = args else {
        return Err(CommandError::Usage(USAGE));
    };
    let hand = match *hand {
        "mainhand" => Hand::Main,
        "offhand" => Hand::Off,
        _ => return Err(CommandError::Usage(USAGE)),
    };
    let animation = match *animation {
        "none" => SwingKind::None,
        "whack" => SwingKind::Whack,
        "stab" => SwingKind::Stab,
        _ => return Err(CommandError::Usage(USAGE)),
    };
    let duration_ticks = ticks
        .parse::<u16>()
        .ok()
        .filter(|ticks| (1..=MAX_TICKS).contains(ticks))
        .ok_or(CommandError::Usage(USAGE))?;
    Ok(SwingCommand {
        target: target_of(target)?,
        hand,
        animation,
        duration_ticks,
    })
}

/// Emits one swing per living target; returns how many swung.
pub(crate) fn execute(
    state: &TickState,
    executor: EntityId,
    command: &SwingCommand,
    changes: &mut ChangeSet,
) -> Result<i32, CommandError> {
    let living: Vec<EntityId> =
        crate::command_execution::resolve_target(state, executor, &command.target)
            .into_iter()
            .filter(|&id| {
                state
                    .entities
                    .slot_of_any(id)
                    .is_some_and(|slot| state.entities.body(slot).alive)
            })
            .collect();
    if living.is_empty() {
        return Err(CommandError::Message("No entity was found".into()));
    }
    for &entity in &living {
        changes.entity_events.push(EntityEvent::Swing {
            entity,
            hand: command.hand,
            animation: command.animation,
            duration_ticks: command.duration_ticks,
        });
    }
    Ok(living.len() as i32)
}

#[cfg(test)]
#[path = "swing_tests.rs"]
mod tests;
