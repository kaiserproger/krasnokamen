//! `data merge|modify|remove entity`: edit one non-player entity's NBT, in any
//! loaded dimension, and load it back through the summon NBT path, so only
//! fields a summon can author are editable.
use crate::boundary::PlayerChange;
use crate::command_context::Context;
use crate::command_execution::CommandResult;
use crate::command_nbt_path::argument;
use crate::command_summon::{Image, ItemSummon};
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

/// The loaded form of an edit, ready to install without further failure.
enum Prepared {
    Mob(Box<Image>),
    Item(ItemSummon),
}

/// The item-entity fields a `data` edit can author: the ones `summon` takes.
const ITEM_FIELDS: [&str; 6] = [
    "Item",
    "Age",
    "PickupDelay",
    "Motion",
    "NoGravity",
    "Invulnerable",
];

fn run(state: &mut TickState, context: Context, mut input: &str) -> Result<Tag, String> {
    if context.permission < 2 {
        return Err(crate::commands::CommandError::Unknown.message());
    }
    argument(&mut input)?; // data
    let action = argument(&mut input)?;
    argument(&mut input)?; // entity
    let id = crate::command_data::single_entity(state, context, argument(&mut input)?)?;
    let (dimension, slot) =
        crate::dimensions::travel::locate(state, id).ok_or("No entity was found")?;
    let world =
        crate::dimensions::travel::world(state, dimension).ok_or("Entity dimension unavailable")?;
    let type_id = match world.entities.body(slot).kind {
        EntityKind::Player { .. } => return Err("Unable to modify player data".into()),
        EntityKind::Item { type_id }
        | EntityKind::Mob { type_id }
        | EntityKind::ExperienceOrb { type_id } => type_id,
    };
    let item = matches!(world.entities.body(slot).kind, EntityKind::Item { .. });
    let name = crate::command_execution::entity_type_name(world, id)
        .ok_or("Entity has no vanilla data")?;
    let mut image = crate::command_data::entity_image(world, id)?;
    let previous = crate::command_data::entity(world, id)?;
    let mut edited = previous.clone();
    crate::command_storage::mutate(state, context, action, &mut edited, &mut input)?;
    if edited == previous {
        return Err("Nothing changed. The specified properties already have these values".into());
    }
    let (Tag::Compound(old), Tag::Compound(new)) = (&previous, edited) else {
        return Err("Entity data must be a compound".into());
    };
    // What the edit changed or removed. The entity keeps its identity and place:
    // those are not data to load back.
    let mut changed = Vec::new();
    let mut touched = Vec::new();
    for (key, value) in &new {
        if old.iter().any(|(k, v)| k == key && v == value) {
            continue;
        }
        if matches!(key.as_str(), "Pos" | "id") {
            return Err(format!("Editing {key} is not supported; use /tp"));
        }
        if key != "UUID" {
            changed.push((key.clone(), value.clone()));
            touched.push(key.clone());
        }
    }
    for (key, _) in old {
        if !new.iter().any(|(k, _)| k == key) {
            if matches!(key.as_str(), "Pos" | "id" | "UUID") {
                return Err(format!("Removing {key} is not supported"));
            }
            image.retain(|(k, _)| k != key);
            touched.push(key.clone());
        }
    }
    for key in &touched {
        let authored = if item {
            ITEM_FIELDS.contains(&key.as_str())
        } else {
            crate::command_summon::supports(world, type_id, name, key) == Some(true)
        };
        if !authored {
            // Only what a summon can author is editable; everything else keeps its value.
            return Err(format!("Editing {key} of {name} is not supported"));
        }
    }
    let prepared = if item {
        // A removed field loads as its default, like a compound without it does.
        let fields = new
            .into_iter()
            .filter(|(key, _)| ITEM_FIELDS.contains(&key.as_str()))
            .collect();
        let mut payload = crate::command_summon::prepare_item(world, &Tag::Compound(fields))
            .map_err(|error| error.message())?;
        payload.age.get_or_insert(0);
        payload.pickup_delay.get_or_insert(0);
        payload.motion.get_or_insert([0.0; 3]);
        Prepared::Item(payload)
    } else {
        let position = world.entities.pos(slot);
        let keep_villager = !touched
            .iter()
            .any(|key| crate::villager::persistence::owns_field(key));
        Prepared::Mob(Box::new(
            crate::command_summon::prepare_edit(
                world,
                type_id,
                name,
                position,
                image,
                &changed,
                keep_villager,
            )
            .map_err(|error| error.message())?,
        ))
    };
    let world = if dimension == state.dimension {
        state
    } else {
        state
            .other_dimensions
            .get_mut(&dimension)
            .map(Box::as_mut)
            .ok_or("Entity dimension unavailable")?
    };
    match prepared {
        Prepared::Mob(image) => crate::command_summon::apply(world, id, *image),
        Prepared::Item(payload) => crate::command_summon::apply_item(world, id, payload),
    }
    Ok(crate::command_execution::entity_name(world, id))
}
