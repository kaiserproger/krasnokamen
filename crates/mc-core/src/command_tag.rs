//! Scoreboard tags use the retained vanilla entity and player NBT owners.
use crate::command_context::Context;
use crate::command_execution::CommandResult;
use crate::tick::TickState;
use mc_nbt::{ListTag, Tag, tag_type};

/// Vanilla entity tags are a set; retain first-occurrence order in the NBT list.
pub(crate) fn normalized(value: &Tag) -> Result<Tag, String> {
    let Tag::List(list) = value else {
        return Err("Invalid entity tags: expected a list of strings".into());
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut elements = Vec::with_capacity(list.elements.len());
    for value in &list.elements {
        let Tag::String(name) = value else {
            return Err("Invalid entity tags: expected string elements".into());
        };
        if seen.insert(name.as_str()) {
            elements.push(value.clone());
        }
    }
    Ok(Tag::List(ListTag {
        element_type: tag_type::STRING,
        elements,
    }))
}

pub(crate) fn normalize_fields(fields: &mut [(String, Tag)]) -> Result<(), String> {
    for (key, value) in fields {
        if key == "Tags" {
            *value = normalized(value)?;
        }
    }
    Ok(())
}

/// Also canonicalize existing native/WAL records, without changing the format.
pub(crate) fn normalize_opaque(mut bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    if bytes.is_empty() {
        return Ok(bytes);
    }
    let (_, Tag::Compound(mut fields)) =
        mc_nbt::read_named(&mut bytes.as_slice()).map_err(|error| error.to_string())?
    else {
        return Err("Invalid retained entity data".into());
    };
    normalize_fields(&mut fields)?;
    bytes.clear();
    mc_nbt::write_named(&mut bytes, "", &Tag::Compound(fields))
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

pub(crate) fn execute(state: &mut TickState, context: Context, input: &str) -> CommandResult {
    let result = run(state, context, input);
    match result {
        Ok((count, message)) => {
            if crate::command_execution::accepts_success(state, context.connection) {
                state.push_player(
                    context.connection,
                    crate::boundary::PlayerChange::Chat { message },
                );
            }
            CommandResult::Success(count)
        }
        Err(message) => {
            state.push_player(
                context.connection,
                crate::boundary::PlayerChange::Chat { message },
            );
            CommandResult::Failed
        }
    }
}

fn run(state: &mut TickState, context: Context, input: &str) -> Result<(i32, String), String> {
    let mut input = input;
    let word = crate::command_nbt_path::argument;
    if word(&mut input)? != "tag" {
        return Err("Expected tag command".into());
    }
    let target = crate::commands::target_of(word(&mut input)?).map_err(|e| e.message())?;
    let action = word(&mut input)?;
    let name = if action == "list" {
        None
    } else {
        Some(word(&mut input)?)
    };
    if !input.trim().is_empty() || !matches!(action, "add" | "remove" | "list") {
        return Err("Expected tag <targets> add|remove <name> or list".into());
    }
    let targets = crate::dimensions::travel::context_targets(state, context, &target)?;
    if targets.is_empty() {
        return Err("No entities were found".into());
    }
    let mut count = 0;
    let mut listed = std::collections::BTreeSet::new();
    for entity in targets {
        let (dimension, slot) =
            crate::dimensions::travel::locate(state, entity).ok_or("No entity was found")?;
        let world = if dimension == state.dimension {
            &mut *state
        } else {
            state
                .other_dimensions
                .get_mut(&dimension)
                .ok_or("Unknown dimension")?
        };
        let bytes = if let Some(player) = world.players.get_mut(&entity) {
            &mut player.opaque_player_fields
        } else {
            &mut world.entities.row_mut(slot).opaque_entity_fields
        };
        let mut fields = if bytes.is_empty() {
            Vec::new()
        } else {
            match mc_nbt::read_named(&mut bytes.as_slice())
                .map_err(|e| e.to_string())?
                .1
            {
                Tag::Compound(fields) => fields,
                _ => return Err("Invalid retained entity data".into()),
            }
        };
        let index = fields.iter().position(|(key, _)| key == "Tags");
        let mut tags = match index.map(|i| &fields[i].1) {
            None => Vec::new(),
            Some(Tag::List(list))
                if list.element_type == tag_type::STRING || list.elements.is_empty() =>
            {
                let Tag::List(tags) = normalized(&fields[index.unwrap()].1)? else {
                    unreachable!("normalized tags are a list");
                };
                tags.elements
            }
            _ => return Err("Invalid entity tags".into()),
        };
        if action == "list" {
            for tag in tags {
                if let Tag::String(tag) = tag {
                    listed.insert(tag);
                }
            }
            continue;
        }
        let name = name.expect("name for add/remove");
        let present = tags
            .iter()
            .position(|tag| matches!(tag, Tag::String(value) if value == name));
        match (action, present) {
            ("add", None) if tags.len() < 1024 => tags.push(Tag::String(name.into())),
            ("remove", Some(index)) => {
                tags.remove(index);
            }
            _ => continue,
        }
        let value = Tag::List(ListTag {
            element_type: tag_type::STRING,
            elements: tags,
        });
        if let Some(index) = index {
            fields[index].1 = value;
        } else {
            fields.push(("Tags".into(), value));
        }
        let mut encoded = Vec::new();
        mc_nbt::write_named(&mut encoded, "", &Tag::Compound(fields)).map_err(|e| e.to_string())?;
        *bytes = encoded;
        count += 1;
    }
    if action == "list" {
        return Ok((
            listed.len() as i32,
            format!(
                "Tags: {}",
                listed.into_iter().collect::<Vec<_>>().join(", ")
            ),
        ));
    }
    if count == 0 {
        return Err(format!("No entity tags were {action}ed"));
    }
    Ok((count, format!("Updated tags on {count} entities")))
}
