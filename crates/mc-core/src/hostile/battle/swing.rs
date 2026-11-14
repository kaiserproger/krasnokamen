//! Vanilla operator `/swing`, emitted as a transient entity event.
use crate::boundary::{ChangeSet, EntityEvent, EntityId, Hand, SwingKind};
use crate::commands::{CommandError, Target, target_of};
use crate::entities::EntityKind;
use crate::tick::TickState;

const USAGE: &str = "swing [<targets> [mainhand|offhand [<animation> [<duration>]]]]";

#[derive(Debug, Clone, PartialEq)]
pub struct SwingCommand {
    target: Target,
    hand: Hand,
    animation: SwingKind,
    duration_ticks: i32,
}

pub(crate) fn parse(args: &[&str]) -> Result<SwingCommand, CommandError> {
    if args.len() > 4 {
        return Err(CommandError::Usage(USAGE));
    }
    let target = match args.first() {
        Some(target) => target_of(target)?,
        None => Target::Executor,
    };
    let hand = match args.get(1).copied() {
        None | Some("mainhand") => Hand::Main,
        Some("offhand") => Hand::Off,
        _ => return Err(CommandError::Usage(USAGE)),
    };
    let animation = match args.get(2).copied() {
        None | Some("whack") => SwingKind::Whack,
        Some("stab") => SwingKind::Stab,
        Some("none") => SwingKind::None,
        _ => return Err(CommandError::Usage(USAGE)),
    };
    let duration_ticks = if let Some(word) = args.get(3) {
        let (number, scale) = match word.as_bytes().last() {
            Some(b'd') => (&word[..word.len() - 1], 24_000.0_f32),
            Some(b's') => (&word[..word.len() - 1], 20.0),
            Some(b't') => (&word[..word.len() - 1], 1.0),
            _ => (*word, 1.0),
        };
        if number.is_empty()
            || !number
                .bytes()
                .all(|b| b.is_ascii_digit() || b == b'.' || b == b'-')
        {
            return Err(CommandError::Usage(USAGE));
        }
        let value: f32 = number.parse().map_err(|_| CommandError::Usage(USAGE))?;
        let ticks = (value * scale + 0.5).floor() as i32;
        if ticks < 1 {
            return Err(CommandError::Usage(USAGE));
        }
        ticks
    } else {
        6
    };
    Ok(SwingCommand {
        target,
        hand,
        animation,
        duration_ticks,
    })
}

pub(crate) fn execute(
    state: &TickState,
    executor: EntityId,
    command: &SwingCommand,
    changes: &mut ChangeSet,
) -> Result<i32, CommandError> {
    let mut count = 0;
    for entity in crate::command_execution::resolve_target(state, executor, &command.target) {
        if !state.entities.slot_of_any(entity).is_some_and(|slot| {
            let body = state.entities.body(slot);
            body.alive
                && matches!(
                    body.kind,
                    EntityKind::Player { .. } | EntityKind::Mob { .. }
                )
        }) {
            continue;
        }
        changes.entity_events.push(EntityEvent::Swing {
            entity,
            hand: command.hand,
            animation: command.animation,
            duration_ticks: command.duration_ticks,
        });
        count += 1;
    }
    if count == 0 {
        Err(CommandError::Message("No living entity was found".into()))
    } else {
        Ok(count)
    }
}

#[cfg(test)]
#[path = "swing_tests.rs"]
mod tests;
