//! `data merge|modify|remove block`: edit one block entity's native NBT image in
//! the command's dimension and install it back through the same prepared-image
//! path `setblock` NBT uses, so every kind keeps its own validation and owners.
use crate::boundary::PlayerChange;
use crate::command_context::Context;
use crate::command_execution::CommandResult;
use crate::command_nbt_path::argument;
use crate::tick::TickState;
use mc_data::block_entity_kinds::BlockEntityKind;
use mc_nbt::Tag;

pub(crate) fn command(state: &mut TickState, context: Context, input: &str) -> CommandResult {
    match run(state, context, input) {
        Ok(position) => {
            crate::command_execution::success_feedback(
                state,
                context.connection,
                "commands.data.block.modified",
                position,
            );
            CommandResult::Success(1)
        }
        Err(message) => {
            state.push_player(context.connection, PlayerChange::Chat { message });
            CommandResult::Failed
        }
    }
}

/// Position of the edited block as the feedback arguments.
fn run(state: &mut TickState, context: Context, mut input: &str) -> Result<Vec<Tag>, String> {
    if context.permission < 2 {
        return Err(crate::commands::CommandError::Unknown.message());
    }
    argument(&mut input)?; // data
    let action = argument(&mut input)?;
    argument(&mut input)?; // block
    let pos = crate::command_data::block_target(context, &mut input)?;
    let world =
        crate::dimensions::travel::world(state, context.dimension).ok_or("Unknown dimension")?;
    let (kind, image) = crate::command_data::block_image(world, pos)?;
    let block = world
        .world
        .block_at(pos.x, pos.y, pos.z)
        .ok_or("That position is not loaded")?;
    let (private, public): (Vec<_>, Vec<_>) = image
        .into_iter()
        .partition(|(key, _)| crate::command_data::private_field(key));
    let mut edited = Tag::Compound(public.clone());
    crate::command_storage::mutate(state, context, action, &mut edited, &mut input)?;
    let Tag::Compound(edited) = edited else {
        return Err("Block entity data must be a compound".into());
    };
    if edited == public {
        return Err("Nothing changed. The specified properties already have these values".into());
    }
    let world = if context.dimension == state.dimension {
        state
    } else {
        state
            .other_dimensions
            .get_mut(&context.dimension)
            .map(Box::as_mut)
            .ok_or("Unknown dimension")?
    };
    if kind == BlockEntityKind::CommandBlock {
        crate::command_blocks::edit(world, pos, &public, &edited)?;
    } else {
        // Identity and position stay the block's own whatever the edit says.
        let mut fields: Vec<(String, Tag)> = edited
            .into_iter()
            .filter(|(key, _)| !matches!(key.as_str(), "id" | "x" | "y" | "z"))
            .collect();
        fields.extend(private);
        let mut bytes = Vec::new();
        mc_nbt::write_network(&mut bytes, &Tag::Compound(fields)).map_err(|e| e.to_string())?;
        let image = crate::command_block_entity::prepare(world, block, Some(&bytes))?
            .ok_or("The target block is not a block entity")?;
        crate::command_block_entity::install(world, pos, image);
    }
    Ok([pos.x, pos.y, pos.z]
        .map(crate::command_data::text)
        .to_vec())
}
