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
    Mob(Box<Image>, crate::plugin_inventories::PreparedEntityCustody),
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
        return Err(crate::command_args::CommandError::Unknown.message());
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
    // These three carry a `VillagerState` row, whose own persistence image (`Brain`,
    // `Inventory`, `Offers`, `Gossips`, `Xp`, …) `data` reads and writes; every other
    // kind only has summon NBT.
    let villager = crate::command_summon::carries_villager_row(name);
    let mut image = crate::command_data::entity_image(world, id)?;
    let previous = crate::command_data::entity(world, id)?;
    let mut edited = previous.clone();
    crate::command_storage::mutate(state, context, action, &mut edited, &mut input, |source| {
        if let Tag::Compound(fields) = source {
            crate::command_tag::validate_fields(fields)?;
        }
        Ok(())
    })?;
    if edited == previous {
        return Err("Nothing changed. The specified properties already have these values".into());
    }
    let (Tag::Compound(old), Tag::Compound(new)) = (&previous, edited) else {
        return Err("Entity data must be a compound".into());
    };
    crate::command_tag::validate_fields(&new)?;
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
        if key == "Tags" {
            crate::command_summon::validate_tags(value).map_err(|error| error.message())?;
            image.retain(|(existing, _)| existing != key);
            image.push((key.clone(), value.clone()));
            touched.push(key.clone());
        } else if key != "UUID" {
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
        let owned = villager && crate::villager::persistence::owns_field(key);
        if crate::command_data::private_field(key) && !owned {
            return Err(format!("Editing {key} of {name} is not supported"));
        }
        let authored = if key == "Tags" {
            true
        } else if item {
            ITEM_FIELDS.contains(&key.as_str())
        } else {
            owned || crate::command_summon::supports(world, type_id, name, key) == Some(true)
        };
        if !authored {
            // Only what a summon authors, or the villager row's own persistence image,
            // is editable; everything else keeps its value.
            return Err(format!("Editing {key} of {name} is not supported"));
        }
    }
    let prepared = if item {
        // A removed field loads as its default, like a compound without it does.
        let fields = new
            .iter()
            .filter(|(key, _)| ITEM_FIELDS.contains(&key.as_str()))
            .cloned()
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
        let live_villager = world.entities.row(slot).villager.clone();
        let mut image = Box::new(
            crate::command_summon::prepare_edit(
                world,
                type_id,
                name,
                position,
                image,
                &changed,
                keep_villager,
                live_villager.as_deref(),
            )
            .map_err(|error| error.message())?,
        );
        let custody = crate::command_summon::data::prepare_edit(
            world,
            slot,
            &mut image,
            touched.iter().any(|key| key == "equipment"),
        )?;
        Prepared::Mob(image, custody)
    };
    let retained_tags = if touched.iter().any(|key| key == "Tags") {
        let image = match &prepared {
            Prepared::Mob(image, _) => Some(image.as_ref()),
            Prepared::Item(_) => None,
        };
        Some(crate::command_summon::data::prepare_tags(
            world,
            slot,
            image,
            new.iter()
                .find(|(key, _)| key == "Tags")
                .map(|(_, value)| value),
        )?)
    } else {
        None
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
    let lethal = match &prepared {
        Prepared::Mob(image, _) => image.lethal,
        Prepared::Item(_) => false,
    };
    match prepared {
        Prepared::Mob(image, custody) => {
            crate::command_summon::apply(world, id, *image);
            crate::plugin_inventories::apply_entity_custody(world, custody);
        }
        Prepared::Item(payload) => crate::command_summon::apply_item(world, id, payload),
    }
    if lethal {
        world.entities.row_mut(slot).health = 0.0;
        if world.entities.body(slot).alive {
            world.entities.body_mut(slot).alive = false;
            world.entities.begin_dying(slot);
        }
    }
    if let Some(tags) = retained_tags {
        world.entities.row_mut(slot).opaque_entity_fields = tags;
    }
    Ok(crate::command_execution::entity_name(world, id))
}
