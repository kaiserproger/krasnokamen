use crate::config::CoreError;
use crate::journal::ReplayEntity;
use crate::tick::TickState;

/// Recovery rejects invalid retained images before a checkpoint can rewrite them.
/// A skipped record would otherwise become permanent data loss.
pub(crate) fn validate_entity(
    state: &TickState,
    id: i32,
    record: &ReplayEntity,
) -> Result<uuid::Uuid, CoreError> {
    let invalid = |reason: String| {
        CoreError::Storage(format!(
            "{}: journal entity wire-id {id} UUID {} type-id {}: {reason}",
            state.dimension.name(),
            record.uuid,
            record.type_id,
        ))
    };
    if !record.state.opaque_entity_fields.is_empty() {
        let mut bytes = record.state.opaque_entity_fields.as_slice();
        let (_, fields) = mc_nbt::read_named(&mut bytes)
            .map_err(|err| invalid(format!("opaque entity fields: {err}")))?;
        if !matches!(fields, mc_nbt::Tag::Compound(_)) || !bytes.is_empty() {
            return Err(invalid(
                "opaque entity fields: expected one complete named compound".into(),
            ));
        }
    }
    let uuid = record
        .uuid
        .parse::<uuid::Uuid>()
        .map_err(|err| invalid(format!("invalid UUID: {err}")))?;
    let name = state
        .registry
        .entity_type_name(record.type_id)
        .ok_or_else(|| invalid("unknown entity type-id".into()))?;
    if matches!(name, "minecraft:item_frame" | "minecraft:glow_item_frame") {
        crate::item_frame::validate_image(record)?;
    }
    if name == "minecraft:painting" {
        crate::painting::validate_image(record, &state.registry)?;
    }
    if record.state.bee.is_some() && name != "minecraft:bee" {
        return Err(invalid(
            "Bee: retained state incompatible with entity type".into(),
        ));
    }
    if let Some(allay) = &record.state.allay {
        if name != "minecraft:allay" {
            return Err(invalid(
                "Allay: retained state incompatible with entity type".into(),
            ));
        }
        let rules = state.registry.item_rules();
        for stack in [&allay.held, &allay.inventory] {
            if !stack.is_empty()
                && (state.registry.items.name_of(stack.item_id).is_none()
                    || stack.count > rules.max_stack(stack))
            {
                return Err(invalid("Allay.Inventory: invalid registered stack".into()));
            }
        }
    }
    if record
        .pos
        .iter()
        .chain(record.vel.iter())
        .any(|value| !value.is_finite())
        || !record.yaw.is_finite()
        || !record.pitch.is_finite()
        || !record.health.is_finite()
        || !record.appearance.health.is_finite()
    {
        return Err(invalid(
            "non-finite position, motion, rotation or health".into(),
        ));
    }
    if let Some(projectile) = &record.projectile {
        crate::projectiles::persistence::validate_state(projectile)
            .map_err(|reason| invalid(reason.into()))?;
        if !projectile.stack.is_empty()
            && state
                .registry
                .items
                .name_of(projectile.stack.item_id)
                .is_none()
        {
            return Err(invalid(format!(
                "projectile has unknown item-id {}",
                projectile.stack.item_id
            )));
        }
        if name != projectile.kind.name() {
            return Err(invalid(format!(
                "projectile kind {} disagrees with entity type {}",
                projectile.kind.name(),
                name
            )));
        }
    }
    Ok(uuid)
}
