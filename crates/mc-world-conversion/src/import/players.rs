//! Legacy player records: UUID, dimension and hotbar-slot admission.

use super::*;

pub(super) fn player_uuid(tag: &Tag, path: &str) -> Result<Option<uuid::Uuid>, CoreError> {
    match field(tag, "UUID")? {
        None => Ok(None),
        Some(Tag::IntArray(words)) if words.len() == 4 => {
            let bits = words
                .iter()
                .fold(0_u128, |bits, word| (bits << 32) | u128::from(*word as u32));
            let uuid = uuid::Uuid::from_u128(bits);
            if uuid.is_nil() {
                return Err(fail(format!("{path}: nil player UUID")));
            }
            Ok(Some(uuid))
        }
        _ => Err(fail(format!("{path}: expected four-int player UUID"))),
    }
}

fn missing_player_dimension(tag: Option<&Tag>) -> bool {
    matches!(tag, Some(Tag::String(name))
        if Dimension::parse(name).is_none() && mc_data::Identifier::parse(name).is_ok())
}

pub(crate) fn player_missing_dimension_source(tag: &Tag) -> Result<Option<String>, CoreError> {
    let respawn = field(tag, "respawn")?;
    if !missing_player_dimension(field(tag, "Dimension")?)
        && !missing_player_dimension(field(tag, "SpawnDimension")?)
        && !missing_player_dimension(
            respawn
                .map(|respawn| field(respawn, "dimension"))
                .transpose()?
                .flatten(),
        )
    {
        return Ok(None);
    }
    Ok(Some(format!(
        "Dimension={:?}, Pos={:?}, SpawnDimension={:?}, SpawnX={:?}, SpawnY={:?}, SpawnZ={:?}, respawn={respawn:?}",
        field(tag, "Dimension")?,
        field(tag, "Pos")?,
        field(tag, "SpawnDimension")?,
        field(tag, "SpawnX")?,
        field(tag, "SpawnY")?,
        field(tag, "SpawnZ")?,
    )))
}

pub(crate) fn normalize_missing_player_dimensions(
    tag: &mut Tag,
    source_location: Option<String>,
    manifest: &CheckpointManifest,
    path: &str,
    losses: &mut Vec<String>,
) -> Result<(), CoreError> {
    let Some(source_location) = source_location else {
        return Ok(());
    };
    let relocate = missing_player_dimension(field(tag, "Dimension")?);
    let clear_respawn = missing_player_dimension(
        field(tag, "respawn")?
            .map(|respawn| field(respawn, "dimension"))
            .transpose()?
            .flatten(),
    );
    if !relocate && !clear_respawn {
        return Ok(());
    }
    let spawn = manifest
        .world_spawn
        .ok_or_else(|| fail(format!("{path}: missing overworld world spawn")))?;
    let Tag::Compound(fields) = tag else {
        return Err(fail(format!("{path}: expected player compound")));
    };
    if relocate {
        fields.retain(|(key, _)| !matches!(key.as_str(), "Dimension" | "Pos"));
        fields.push((
            "Dimension".into(),
            Tag::String("minecraft:overworld".into()),
        ));
        fields.push((
            "Pos".into(),
            Tag::List(mc_nbt::ListTag {
                element_type: mc_nbt::tag_type::DOUBLE,
                elements: spawn
                    .into_iter()
                    .map(|value| Tag::Double(f64::from(value)))
                    .collect(),
            }),
        ));
    }
    if clear_respawn {
        fields.retain(|(key, _)| key != "respawn");
    }
    losses.push(format!(
        "PRESERVATION LOSS: {path}: owner-approved absent-dimension login fallback; relocated_to_overworld_world_spawn={relocate}, cleared_missing_dimension_respawn={clear_respawn}, world_spawn={spawn:?}; original source {source_location}; all other player fields retained"
    ));
    Ok(())
}

// PlayerEquipmentFix (4312) folds old armor/offhand entries into a map.
// Validate before that fold, not just before the native array projection.
pub(super) fn validate_legacy_player_slots(
    tag: &Tag,
    from: i32,
    path: &str,
) -> Result<(), CoreError> {
    if from >= 4312 {
        return Ok(());
    }
    if let Some(equipment) =
        field(tag, "equipment").map_err(|error| fail(format!("{path}: {error}")))?
        && !matches!(equipment, Tag::Compound(fields) if fields.is_empty())
    {
        return Err(fail(format!(
            "{path}: equipment slot: unsupported before PlayerEquipmentFix 4312"
        )));
    }
    let Some(value) = field(tag, "Inventory").map_err(|error| fail(format!("{path}: {error}")))?
    else {
        return Ok(());
    };
    let Tag::List(list) = value else {
        return Err(fail(format!("{path}: Inventory: expected list")));
    };
    let mut occupied = 0_u64;
    for (index, item) in list.elements.iter().enumerate() {
        let bad_slot =
            |reason: &str| fail(format!("{path}: Inventory entry {index} Slot: {reason}"));
        let Some(Tag::Byte(slot)) =
            field(item, "Slot").map_err(|error| bad_slot(&error.to_string()))?
        else {
            return Err(bad_slot("missing byte Slot"));
        };
        let bit = match *slot {
            0..=35 => *slot as u8,
            100..=103 => (*slot - 100) as u8 + 36,
            -106 => 40,
            _ => return Err(bad_slot(&format!("{slot} outside supported legacy slots"))),
        };
        if occupied & (1_u64 << bit) != 0 {
            return Err(bad_slot(&format!("duplicate {slot}")));
        }
        occupied |= 1_u64 << bit;
    }
    Ok(())
}

pub(super) fn admit_player(
    mut tag: Tag,
    uuid: uuid::Uuid,
    path: &str,
    tick: u64,
    items: &mc_data::items::ItemRegistry,
    manifest: &mut CheckpointManifest,
    players: &mut Vec<NativePlayerSnapshot>,
) -> Result<(), CoreError> {
    if player_uuid(&tag, path)?.is_some_and(|actual| actual != uuid) {
        return Err(fail(format!(
            "{path}: player UUID disagrees with source identity"
        )));
    }
    let player = mc_core::journal::player_from_tag(&tag, items)
        .map_err(|error| fail(format!("{path}: player {uuid}: {error}")))?;
    if player.pos.iter().any(|value| !value.is_finite())
        || !player.yaw.is_finite()
        || !player.pitch.is_finite()
        || !player.health.is_finite()
    {
        return Err(fail(format!("{path}: non-finite player")));
    }
    let Tag::Compound(fields) = &mut tag else {
        return Err(fail(format!("{path}: player is not compound")));
    };
    fields.retain(|(key, _)| {
        !matches!(
            key.as_str(),
            "krasnokamen:Tick" | "KrasnokamenUnlockedRecipes"
        )
    });
    if let Some((_, Tag::Compound(book))) = fields.iter_mut().find(|(key, _)| key == "recipeBook") {
        book.retain(|(key, _)| key != "recipes");
    }
    fields.push((
        "KrasnokamenUnlockedRecipes".into(),
        Tag::List(mc_nbt::ListTag {
            element_type: mc_nbt::tag_type::STRING,
            elements: player
                .unlocked_recipes
                .iter()
                .cloned()
                .map(Tag::String)
                .collect(),
        }),
    ));
    fields.push((
        "krasnokamen:Tick".into(),
        Tag::Long(i64::try_from(tick).map_err(fail)?),
    ));
    manifest.players.insert(uuid.to_string(), tick);
    players.push(NativePlayerSnapshot {
        uuid: *uuid.as_bytes(),
        record: bridge::tag_record(tick, &tag)?,
    });
    Ok(())
}
