//! Preserve live-only entity state and preflight inventory custody for data edits.
use super::Image;
use crate::entities::EntitySlot;
use crate::tick::TickState;

pub(crate) fn prepare_edit(
    state: &TickState,
    slot: EntitySlot,
    image: &mut Image,
    equipment_touched: bool,
) -> Result<crate::plugin_inventories::PreparedEntityCustody, String> {
    let current = &state.entities.row(slot).entity_life;
    let next = &mut image.state.entity_life;
    next.mounted = current.mounted;
    next.frog_spawn_target = current.frog_spawn_target;
    next.cat_relax = current.cat_relax.clone();
    next.mount.input = current.mount.input;
    next.mount.vehicle = current.mount.vehicle;

    if equipment_touched {
        let equipment = image.equipment.get_or_insert_with(Default::default);
        crate::plugin_inventories::prepare_entity_custody(state, slot, equipment)
            .map_err(|_| "Entity inventory custody is unavailable".into())
    } else {
        Ok(Default::default())
    }
}

pub(crate) fn prepare_tags(
    state: &TickState,
    slot: EntitySlot,
    tags: Option<&mc_nbt::Tag>,
) -> Result<Vec<u8>, String> {
    let bytes = &state.entities.row(slot).opaque_entity_fields;
    let mut fields = if bytes.is_empty() {
        Vec::new()
    } else {
        let (_, mc_nbt::Tag::Compound(fields)) =
            mc_nbt::read_named(&mut bytes.as_slice()).map_err(|error| error.to_string())?
        else {
            return Err("Invalid retained entity fields".into());
        };
        fields
    };
    fields.retain(|(key, _)| key != "Tags");
    if let Some(tags) = tags {
        fields.push(("Tags".into(), tags.clone()));
    }
    let mut bytes = Vec::new();
    mc_nbt::write_named(&mut bytes, "", &mc_nbt::Tag::Compound(fields))
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}
