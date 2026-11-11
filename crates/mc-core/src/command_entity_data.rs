//! `data merge|modify|remove entity`: edit one non-player entity's NBT and load
//! it back through the summon NBT path, so only fields a summon can author are
//! editable.
use crate::boundary::PlayerChange;
use crate::command_context::Context;
use crate::command_execution::CommandResult;
use crate::command_nbt_path::argument;
use crate::entities::EntityKind;
use crate::tick::TickState;
use mc_nbt::Tag;

pub(crate) fn command(state: &mut TickState, context: Context, input: &str) -> CommandResult {
    match run(state, context, input) {
        Ok(name) => {
            crate::command_execution::success_feedback(
                state,
                context.connection,
                "commands.data.entity.modified",
                vec![name],
            );
            CommandResult::Success(1)
        }
        Err(message) => {
            state.push_player(context.connection, PlayerChange::Chat { message });
            CommandResult::Failed
        }
    }
}

fn run(state: &mut TickState, context: Context, mut input: &str) -> Result<Tag, String> {
    if context.permission < 2 {
        return Err(crate::commands::CommandError::Unknown.message());
    }
    argument(&mut input)?; // data
    let action = argument(&mut input)?;
    argument(&mut input)?; // entity
    let id = crate::command_data::single_entity(state, context, argument(&mut input)?)?;
    let (dimension, _) =
        crate::dimensions::travel::locate(state, id).ok_or("No entity was found")?;
    if dimension != state.dimension {
        return Err("Editing the data of an entity in another dimension is not supported".into());
    }
    let slot = state.entities.slot_of(id).ok_or("No entity was found")?;
    let type_id = match state.entities.body(slot).kind {
        EntityKind::Player { .. } => return Err("Unable to modify player data".into()),
        EntityKind::Item { .. } => {
            return Err("Editing the data of an item entity is not supported".into());
        }
        EntityKind::Mob { type_id } | EntityKind::ExperienceOrb { type_id } => type_id,
    };
    let name = crate::command_execution::entity_type_name(state, id)
        .ok_or("Entity has no vanilla data")?;
    let previous = crate::command_data::entity(state, id)?;
    let mut edited = previous.clone();
    crate::command_storage::mutate(state, context, action, &mut edited, &mut input)?;
    if edited == previous {
        return Err("Nothing changed. The specified properties already have these values".into());
    }
    let (Tag::Compound(old), Tag::Compound(new)) = (&previous, edited) else {
        return Err("Entity data must be a compound".into());
    };
    let mut fields = Vec::with_capacity(new.len());
    for (key, value) in new {
        let unchanged = old
            .iter()
            .any(|(old_key, old_value)| *old_key == key && *old_value == value);
        // The entity keeps its identity and place: those are not data to load back.
        if matches!(key.as_str(), "UUID" | "Pos" | "id") {
            if !unchanged && key != "UUID" {
                return Err(format!("Editing {key} is not supported; use /tp"));
            }
            continue;
        }
        match crate::command_summon::supports(state, type_id, name, &key) {
            Some(true) => fields.push((key, value)),
            // A field summon cannot author keeps the value it has.
            _ if unchanged => {}
            _ => return Err(format!("Editing {key} of {name} is not supported")),
        }
    }
    // A removed field loads as its default, like a compound without it does.
    let position = state.entities.pos(slot);
    let image =
        crate::command_summon::prepare(state, type_id, name, position, &Tag::Compound(fields))
            .map_err(|error| error.message())?;
    crate::command_summon::apply(state, id, image);
    Ok(crate::command_execution::entity_name(state, id))
}
