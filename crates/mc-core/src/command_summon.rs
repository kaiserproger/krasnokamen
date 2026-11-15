//! Summon preflight uses the same native entity codecs as checkpoint recovery.
//! No entity is allocated until every authored field has been validated.
//! Experience orbs retain native `Value`/positive `Count` and spawn directly into
//! the orb index, rather than being allocated as mobs.
use crate::boundary::{EntityDataValue, EntityId};
use crate::command_args::CommandError;
use crate::journal::EntityStateImage;
use crate::tick::TickState;
use mc_data::entity_nbt::VanillaEntity;
use mc_nbt::{ListTag, Tag, tag_type};
use std::sync::Arc;

pub(crate) struct Image {
    native: VanillaEntity,
    state: EntityStateImage,
    villager: Option<crate::villager::VillagerState>,
    equipment: Option<[Option<mc_data::ItemStack>; 6]>,
    /// The command supplied `UUID`; otherwise the spawn's derived uuid stands.
    uuid_authored: bool,
    /// Painting direction (`add_entity` data 2..=5) and variant registry index.
    painting: Option<(u8, u32)>,
    /// An edit that touched no villager field keeps the entity's live villager state.
    keep_villager: bool,
    /// Validated once during preflight, then moved into the new appearance.
    variant: Option<(u8, EntityDataValue)>,
}

impl Image {
    /// Installs a registry-resolved kit before the normal attribute and
    /// appearance restoration. Like authored equipment, this replaces all slots.
    pub(crate) fn with_equipment(
        mut self,
        equipment: [Option<mc_data::ItemStack>; 6],
        body_armor: Option<mc_data::ItemStack>,
    ) -> Self {
        self.equipment = Some(equipment);
        self.state.loot.body_armor = body_armor.map(Box::new);
        self.state.loot.saddle = None;
        self
    }
}
fn error(text: impl Into<String>) -> CommandError {
    CommandError::Message(text.into())
}

/// Vanilla experience-orb summon: a signed `Value` (short, default zero), a
/// positive `Count` (default one) and the universal entity fields (`Motion`,
/// `Pos`, `id`, `UUID`, ...), which the orb spawn applies. Any other field is
/// refused rather than silently dropped.
pub(crate) fn experience_orb(nbt: &Tag) -> Result<(i16, i32, [f64; 3]), CommandError> {
    let Tag::Compound(fields) = nbt else {
        return Err(error("Summon NBT must be a compound"));
    };
    let (mut value, mut count, mut motion) = (0i16, 1i32, [0.0f64; 3]);
    let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for (key, tag) in fields {
        if !seen.insert(key.as_str()) {
            return Err(error(format!(
                "Unsupported or duplicate orb summon NBT field {key}"
            )));
        }
        match (key.as_str(), tag) {
            ("Value", Tag::Byte(n)) => value = i16::from(*n),
            ("Value", Tag::Short(n)) => value = *n,
            ("Value", Tag::Int(n)) => value = *n as i16,
            ("Value", Tag::Long(n)) => value = *n as i16,
            ("Value", _) => return Err(error("Invalid orb Value: expected integer")),
            ("Count", Tag::Byte(n)) => count = i32::from(*n),
            ("Count", Tag::Short(n)) => count = i32::from(*n),
            ("Count", Tag::Int(n)) => count = *n,
            ("Count", Tag::Long(n)) => count = *n as i32,
            ("Count", _) => return Err(error("Invalid orb Count: expected integer")),
            ("Motion", Tag::List(list)) if list.elements.len() == 3 => {
                for (axis, element) in list.elements.iter().enumerate() {
                    motion[axis] = number(element, "Motion")?;
                }
            }
            ("Motion", _) => return Err(error("Invalid Motion: expected three numbers")),
            ("Tags", _) => validate_tags(tag)?,
            (
                "Pos" | "id" | "UUID" | "CustomName" | "CustomNameVisible" | "Invulnerable"
                | "NoGravity" | "Rotation" | "OnGround" | "Fire" | "FallDistance",
                _,
            ) => {}
            _ => {
                return Err(error(format!(
                    "Unsupported or duplicate orb summon NBT field {key}"
                )));
            }
        }
    }
    if count <= 0 {
        return Err(error("Invalid orb Count: expected a positive integer"));
    }
    Ok((value, count, motion))
}

fn validate_tags(value: &Tag) -> Result<(), CommandError> {
    let Tag::List(list) = value else {
        return Err(error("Invalid Tags: expected a list of strings"));
    };
    if list
        .elements
        .iter()
        .any(|tag| !matches!(tag, Tag::String(_)))
    {
        return Err(error("Invalid Tags: every list element must be a string"));
    }
    if list.elements.len() > 1024 {
        return Err(error("Invalid Tags: expected at most 1024 strings"));
    }
    for (index, tag) in list.elements.iter().enumerate() {
        let Tag::String(value) = tag else {
            unreachable!()
        };
        // NBT uses Java Modified UTF-8 (UTF-16 code units, not UTF-8 bytes).
        let bytes: usize = value
            .encode_utf16()
            .map(|unit| match unit {
                1..=0x7f => 1,
                0..=0x7ff => 2,
                _ => 3,
            })
            .sum();
        if bytes > u16::MAX as usize {
            return Err(error(format!(
                "Invalid Tags: tag at index {index} is {bytes} Modified UTF-8 bytes; maximum is 65535"
            )));
        }
    }
    Ok(())
}

pub(crate) fn retained_tags(nbt: &Tag) -> Result<Option<Vec<u8>>, CommandError> {
    let Tag::Compound(fields) = nbt else {
        return Err(error("Summon NBT must be a compound"));
    };
    let Some((_, tags)) = fields.iter().find(|(key, _)| key == "Tags") else {
        return Ok(None);
    };
    validate_tags(tags)?;
    let mut bytes = Vec::new();
    mc_nbt::write_named(
        &mut bytes,
        "",
        &Tag::Compound(vec![(
            "Tags".into(),
            crate::command_tag::normalized(tags).map_err(error)?,
        )]),
    )
    .map_err(|failure| error(format!("Invalid Tags: {failure}")))?;
    Ok(Some(bytes))
}
fn number(tag: &Tag, key: &str) -> Result<f64, CommandError> {
    let value = match tag {
        Tag::Byte(v) => f64::from(*v),
        Tag::Short(v) => f64::from(*v),
        Tag::Int(v) => f64::from(*v),
        Tag::Long(v) => *v as f64,
        Tag::Float(v) => f64::from(*v),
        Tag::Double(v) => *v,
        _ => return Err(error(format!("Invalid {key}: expected number"))),
    };
    if !value.is_finite() {
        return Err(error(format!("Invalid {key}: non-finite number")));
    }
    Ok(value)
}
fn uuid(tag: &Tag) -> Result<uuid::Uuid, CommandError> {
    match tag {
        Tag::String(value) => uuid::Uuid::parse_str(value).map_err(|_| error("Invalid UUID")),
        Tag::IntArray(parts) if parts.len() == 4 => Ok(uuid::Uuid::from_u128(
            (parts[0] as u32 as u128) << 96
                | (parts[1] as u32 as u128) << 64
                | (parts[2] as u32 as u128) << 32
                | parts[3] as u32 as u128,
        )),
        _ => Err(error("Invalid UUID")),
    }
}
/// Item stack authored as `{id, count, components}`, admitted like `/give`.
fn parse_stack(
    items: &mc_data::items::ItemRegistry,
    fields: &[(String, Tag)],
) -> Result<mc_data::ItemStack, String> {
    for (index, (key, _)) in fields.iter().enumerate() {
        if !matches!(key.as_str(), "id" | "count" | "components")
            || fields[..index].iter().any(|(seen, _)| seen == key)
        {
            return Err(format!("invalid or duplicate item field {key}"));
        }
    }
    let Some(Tag::String(id)) = mc_data::item_nbt::field(fields, "id") else {
        return Err("expected item id string".into());
    };
    let id = mc_data::Identifier::parse(id.clone()).map_err(|e| e.to_string())?;
    let count = match mc_data::item_nbt::field(fields, "count") {
        None => 1,
        Some(Tag::Byte(v)) => i32::from(*v),
        Some(Tag::Short(v)) => i32::from(*v),
        Some(Tag::Int(v)) => *v,
        _ => return Err("expected integer item count".into()),
    };
    if count <= 0 {
        return Err("item count must be positive".into());
    }
    let components = match mc_data::item_nbt::field(fields, "components") {
        None => &[][..],
        Some(Tag::Compound(components)) => components.as_slice(),
        _ => return Err("expected components compound".into()),
    };
    for (index, (key, _)) in components.iter().enumerate() {
        if components[..index].iter().any(|(seen, _)| seen == key) {
            return Err(format!("duplicate item component {key}"));
        }
    }
    // The same component admission as /give.
    let mut stack = mc_data::item_nbt::stack_from_component_patch(items, &id, components)
        .map_err(|e| e.to_string())?;
    stack.count = count;
    Ok(stack)
}
fn prepare_stack(
    items: &mc_data::items::ItemRegistry,
    fields: &[(String, Tag)],
) -> Result<Vec<(String, Tag)>, String> {
    if fields.is_empty() {
        return Ok(Vec::new());
    }
    let stack = parse_stack(items, fields)?;
    mc_data::item_nbt::stack_to_fields(items, &stack).map_err(|e| e.to_string())
}

/// `summon minecraft:item` payload: the stack plus the authored item-entity fields.
pub(crate) struct ItemSummon {
    pub(crate) stack: mc_data::ItemStack,
    pub(crate) pickup_delay: Option<u32>,
    pub(crate) age: Option<u32>,
    pub(crate) motion: Option<[f64; 3]>,
    pub(crate) no_gravity: bool,
    pub(crate) invulnerable: bool,
    tags: Option<Vec<u8>>,
}
/// Validate item stack and supported item-entity summon fields before any spawn.
pub(crate) fn prepare_item(state: &TickState, nbt: &Tag) -> Result<ItemSummon, CommandError> {
    let Tag::Compound(authored) = nbt else {
        return Err(error("Summon NBT must be a compound"));
    };
    let mut stack = None;
    let (mut pickup_delay, mut age, mut motion) = (None, None, None);
    let (mut no_gravity, mut invulnerable) = (false, false);
    for (index, (key, value)) in authored.iter().enumerate() {
        if authored[..index].iter().any(|(seen, _)| seen == key) {
            return Err(error(format!("Duplicate summon NBT field {key}")));
        }
        match key.as_str() {
            "id" | "Pos" => {}
            "Tags" => validate_tags(value)?,
            "NoGravity" => no_gravity = number(value, key)? != 0.0,
            "Invulnerable" => invulnerable = number(value, key)? != 0.0,
            "Item" => {
                let Tag::Compound(fields) = value else {
                    return Err(error("Invalid Item: expected compound"));
                };
                stack = Some(
                    parse_stack(&state.registry.items, fields)
                        .map_err(|e| error(format!("Invalid Item: {e}")))?,
                );
            }
            "PickupDelay" => {
                // Vanilla stores a short; 32767 means "never".
                pickup_delay = Some(number(value, key)?.clamp(0.0, 32767.0) as u32);
            }
            "Age" => {
                let value = number(value, key)?;
                // Vanilla `INFINITE_LIFETIME`: the clock never advances.
                age = Some(if value == f64::from(i16::MIN) {
                    crate::entities::ITEM_AGE_NEVER
                } else {
                    value.clamp(0.0, 6000.0) as u32
                });
            }
            "Motion" => {
                let Tag::List(list) = value else {
                    return Err(error("Invalid Motion"));
                };
                if list.elements.len() != 3 {
                    return Err(error("Motion must contain 3 values"));
                }
                let mut values = [0.0; 3];
                for (slot, element) in values.iter_mut().zip(&list.elements) {
                    *slot = number(element, key)?;
                }
                motion = Some(values);
            }
            _ => {
                return Err(error(format!(
                    "Unknown or unsupported summon NBT field {key}"
                )));
            }
        }
    }
    let stack = stack.ok_or_else(|| error("Item entity summon requires an Item"))?;
    Ok(ItemSummon {
        stack,
        pickup_delay,
        age,
        motion,
        no_gravity,
        invulnerable,
        tags: retained_tags(nbt)?,
    })
}
/// Whether the summon NBT field `key` applies to the entity `name`; `None`
/// when no entity takes it.
pub(crate) fn supports(state: &TickState, type_id: u32, name: &str, key: &str) -> Option<bool> {
    let contract = mc_data::entity_types::entity_type_contract_26_1_2_by_protocol_id(type_id);
    let living = contract
        .is_some_and(|c| c.behavior.archetype == mc_data::entity_types::EntityArchetype::Living);
    let horse = crate::taming::horse(name);
    let is_painting = name == crate::painting::PAINTING_ID;
    let ageable = state
        .registry
        .blueprint(type_id)
        .is_some_and(|b| b.offspring.is_some() || b.capabilities.ageable);
    Some(match key {
        "Health" | "NoAI" | "PersistenceRequired" | "CanPickUpLoot" | "leash" | "drop_chances" => {
            living
        }
        "Owner" | "Tame" => {
            horse
                || matches!(
                    name,
                    "minecraft:wolf" | "minecraft:cat" | "minecraft:parrot"
                )
        }
        "Sitting" => matches!(
            name,
            "minecraft:wolf" | "minecraft:cat" | "minecraft:parrot"
        ),
        "Temper" => horse,
        "CollarColor" => matches!(name, "minecraft:wolf" | "minecraft:cat"),
        "equipment" => living || name == "minecraft:armor_stand",
        "Age" => {
            ageable
                || matches!(
                    name,
                    "minecraft:villager" | "minecraft:wandering_trader" | "minecraft:tadpole"
                )
        }
        "IsBaby" => crate::entity_life::uses_is_baby(name),
        "VillagerData" => matches!(name, "minecraft:villager" | "minecraft:zombie_villager"),
        "InLove" | "LoveCause" => ageable,
        "Color" | "Sheared" => name == "minecraft:sheep",
        "Pumpkin" => name == "minecraft:snow_golem",
        "Type" | "stew_effects" => name == "minecraft:mooshroom",
        "Trusting" => name == "minecraft:ocelot",
        "Trusted" => name == "minecraft:fox",
        "Small" | "ShowArms" | "NoBasePlate" | "Marker" | "Invisible" | "DisabledSlots"
        | "Pose" => name == "minecraft:armor_stand",
        "Size" => matches!(name, "minecraft:slime" | "minecraft:magma_cube"),
        "facing" => is_painting,
        "variant" => is_painting || crate::blueprint::variant_registry(name).is_some(),
        "Value" | "Count" => contract.is_some_and(|c| {
            c.behavior.physical_simulation
                == mc_data::entity_types::PhysicalSimulationClass::ExperienceOrb
        }),
        "krasnokamen:Faction" => true,
        "CustomName" | "CustomNameVisible" | "Invulnerable" | "NoGravity" | "Motion"
        | "Rotation" | "OnGround" | "Fire" | "FallDistance" | "UUID" | "id" | "Pos" | "Tags" => {
            true
        }
        "fuse" | "explosion_power" | "block_state" => name == "minecraft:tnt",
        "BlockState" | "Time" => name == "minecraft:falling_block",
        _ => return None,
    })
}
/// Normalised summon fields and the validated typed `variant` appearance value, if any.
type Authored = (Vec<(String, Tag)>, Option<(u8, EntityDataValue)>);
/// Validate `authored` against the summon field set and normalise each value.
fn authored_fields(
    state: &TickState,
    type_id: u32,
    name: &str,
    authored: &[(String, Tag)],
) -> Result<Authored, CommandError> {
    let horse = crate::taming::horse(name);
    let is_painting = name == crate::painting::PAINTING_ID;
    let mut fields = Vec::with_capacity(authored.len());
    let mut variant = None;
    for (key, value) in authored {
        if fields.iter().any(|(seen, _)| seen == key) {
            return Err(error(format!("Duplicate summon NBT field {key}")));
        }
        let Some(supported) = supports(state, type_id, name, key) else {
            return Err(error(format!(
                "Unknown or unsupported summon NBT field {key}"
            )));
        };
        if !supported {
            return Err(error(format!(
                "Entity does not support summon NBT field {key}"
            )));
        }
        let value = match key.as_str() {
            "id" | "Pos" => continue,
            "Tags" => {
                validate_tags(value)?;
                value.clone()
            }
            "NoAI"
            | "PersistenceRequired"
            | "CanPickUpLoot"
            | "IsBaby"
            | "Invulnerable"
            | "NoGravity"
            | "CustomNameVisible"
            | "OnGround"
            | "Tame"
            | "Sitting"
            | "Trusting"
            | "Small"
            | "ShowArms"
            | "NoBasePlate"
            | "Marker"
            | "Invisible"
            | "Sheared"
            | "Pumpkin" => Tag::Byte(i8::from(number(value, key)? != 0.0)),
            "Health" => {
                let v = number(value, key)?;
                if v <= 0.0 || v > f64::from(f32::MAX) {
                    return Err(error("Invalid Health"));
                }
                Tag::Float(v as f32)
            }
            "FallDistance" => {
                let v = number(value, key)?;
                if v.abs() > f64::from(f32::MAX) {
                    return Err(error("Invalid FallDistance"));
                }
                Tag::Float(v as f32)
            }
            "Age" => Tag::Int(number(value, key)?.clamp(i32::MIN as f64, i32::MAX as f64) as i32),
            "Value" => Tag::Short(number(value, key)? as i16),
            "Count" => {
                let count = number(value, key)?;
                if count.fract() != 0.0 || !(1.0..=f64::from(i32::MAX)).contains(&count) {
                    return Err(error("Invalid Count: expected positive integer"));
                }
                Tag::Int(count as i32)
            }
            // Vanilla Slime.setSize clamps the stored zero-based Size to 0..=126.
            "Size" => Tag::Int(number(value, key)?.clamp(0.0, 126.0) as i32),
            "facing" => {
                let v = number(value, key)?;
                if v.fract() != 0.0 || !(0.0..=3.0).contains(&v) {
                    return Err(error(
                        "Invalid painting facing: expected 0 (south), 1 (west), 2 (north) or 3 (east)",
                    ));
                }
                Tag::Byte(v as i8)
            }
            "variant" => {
                let Tag::String(raw) = value else {
                    return Err(error("Invalid variant: expected string"));
                };
                let species = name.strip_prefix("minecraft:").unwrap_or(name);
                let id = mc_data::Identifier::parse(raw.as_str())
                    .map_err(|_| error(format!("Invalid {species} variant {raw}")))?;
                let known = if is_painting {
                    state.registry.paintings.iter().any(|v| v.name == id)
                } else {
                    variant = crate::blueprint::variant_registry(name)
                        .and_then(|registry| mc_data::required_registry_entry_id(registry, &id))
                        .and_then(|value| crate::blueprint::variant_data(name, value));
                    variant.is_some()
                };
                if !known {
                    return Err(error(format!("Unknown {species} variant {raw}")));
                }
                Tag::String(id.as_str().into())
            }
            "DisabledSlots" => {
                Tag::Int(number(value, key)?.clamp(i32::MIN as f64, i32::MAX as f64) as i32)
            }
            "InLove" => Tag::Int(number(value, key)?.clamp(0.0, u16::MAX as f64) as i32),
            "Fire" => {
                Tag::Short(number(value, key)?.clamp(i16::MIN as f64, i16::MAX as f64) as i16)
            }
            "Temper" => Tag::Int(number(value, key)?.clamp(0.0, 100.0) as i32),
            "CollarColor" => Tag::Byte(number(value, key)?.clamp(0.0, 15.0) as i8),
            "Color" => {
                let v = number(value, key)?;
                if !(0.0..=15.0).contains(&v) || v.fract() != 0.0 {
                    return Err(error("Invalid sheep color"));
                }
                Tag::Byte(v as i8)
            }
            "Motion" | "Rotation" => {
                let Tag::List(list) = value else {
                    return Err(error(format!("Invalid {key}")));
                };
                let count = if key == "Motion" { 3 } else { 2 };
                if list.elements.len() != count {
                    return Err(error(format!("{key} must contain {count} values")));
                }
                let numbers = list
                    .elements
                    .iter()
                    .map(|v| number(v, key))
                    .collect::<Result<Vec<_>, _>>()?;
                if key == "Motion" {
                    if numbers.iter().any(|v| !(v * 20.0).is_finite()) {
                        return Err(error("Invalid Motion"));
                    }
                    Tag::List(ListTag {
                        element_type: tag_type::DOUBLE,
                        elements: numbers.into_iter().map(Tag::Double).collect(),
                    })
                } else {
                    if numbers.iter().any(|v| v.abs() > f64::from(f32::MAX)) {
                        return Err(error("Invalid Rotation"));
                    }
                    Tag::List(ListTag {
                        element_type: tag_type::FLOAT,
                        elements: numbers.into_iter().map(|v| Tag::Float(v as f32)).collect(),
                    })
                }
            }
            "UUID" => {
                let value = uuid(value)?;
                if state.dimension_globals.reserved_uuids.contains(&value)
                    || std::iter::once(state)
                        .chain(state.other_dimensions.values().map(|world| &**world))
                        .any(|world| {
                            world
                                .entities
                                .slots()
                                .any(|slot| world.entities.row(slot).uuid == value)
                                || world.offline_players.contains_key(&value)
                                || world.connections.all_connections().any(|connection| {
                                    world.connections.uuid_of(connection) == Some(value)
                                })
                        })
                {
                    return Err(error("Summon UUID already exists"));
                }
                Tag::IntArray(
                    value
                        .as_u128()
                        .to_be_bytes()
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|p| i32::from_be_bytes(*p))
                        .collect(),
                )
            }
            // Validated (profession/type/level) by `villager_from_fields` below.
            "VillagerData" => {
                let Tag::Compound(_) = value else {
                    return Err(error("Invalid VillagerData: expected compound"));
                };
                value.clone()
            }
            "equipment" => {
                let Tag::Compound(slots) = value else {
                    return Err(error("Invalid equipment: expected compound"));
                };
                let mut normalized = Vec::with_capacity(slots.len());
                for (slot, stack) in slots {
                    if !matches!(
                        slot.as_str(),
                        "mainhand"
                            | "offhand"
                            | "head"
                            | "chest"
                            | "legs"
                            | "feet"
                            | "body"
                            | "saddle"
                    ) || (slot == "saddle"
                        && !(horse
                            || matches!(
                                name,
                                "minecraft:pig" | "minecraft:strider" | "minecraft:armor_stand"
                            )))
                    {
                        return Err(error(format!("Unsupported summon equipment slot {slot}")));
                    }
                    if normalized.iter().any(|(seen, _)| seen == slot) {
                        return Err(error(format!("Duplicate equipment slot {slot}")));
                    }
                    let Tag::Compound(stack) = stack else {
                        return Err(error(format!(
                            "Invalid equipment {slot}: expected item compound"
                        )));
                    };
                    let parsed = prepare_stack(&state.registry.items, stack)
                        .map_err(|e| error(format!("Invalid equipment {slot}: {e}")))?;
                    normalized.push((slot.clone(), Tag::Compound(parsed)));
                }
                Tag::Compound(normalized)
            }
            "drop_chances" => {
                let Tag::Compound(slots) = value else {
                    return Err(error("Invalid drop_chances: expected compound"));
                };
                let mut normalized = Vec::with_capacity(slots.len());
                for (slot, value) in slots {
                    if !matches!(
                        slot.as_str(),
                        "mainhand"
                            | "offhand"
                            | "head"
                            | "chest"
                            | "legs"
                            | "feet"
                            | "body"
                            | "saddle"
                    ) || normalized.iter().any(|(seen, _)| seen == slot)
                    {
                        return Err(error(format!("Invalid drop_chances slot {slot}")));
                    }
                    let chance = number(value, "drop_chances")?;
                    if chance < 0.0 || chance > f64::from(f32::MAX) {
                        return Err(error(format!("Invalid drop_chances {slot}")));
                    }
                    normalized.push((slot.clone(), Tag::Float(chance as f32)));
                }
                Tag::Compound(normalized)
            }
            "CustomName" => match value {
                Tag::String(_) => value.clone(),
                Tag::Compound(component)
                    if component.len() == 1
                        && component[0].0 == "text"
                        && matches!(component[0].1, Tag::String(_)) =>
                {
                    component[0].1.clone()
                }
                _ => {
                    return Err(error(
                        "Unsupported CustomName text component: expected literal string or {text:string}",
                    ));
                }
            },
            "Type" => {
                if !matches!(value, Tag::String(_)) {
                    return Err(error(format!("Invalid {key}: expected string")));
                }
                value.clone()
            }
            "stew_effects" => {
                let Tag::List(list) = value else {
                    return Err(error("Invalid stew_effects"));
                };
                for effect in &list.elements {
                    let Tag::Compound(fields) = effect else {
                        return Err(error("Invalid stew_effects entry"));
                    };
                    let id = fields.iter().find(|(k, _)| k == "id").map(|(_, v)| v);
                    if !matches!(id, Some(Tag::String(raw)) if mc_data::Identifier::parse(raw).is_ok())
                    {
                        return Err(error("Invalid stew_effects.id"));
                    }
                    if let Some((_, duration)) = fields.iter().find(|(k, _)| k == "duration") {
                        number(duration, "stew_effects.duration")?;
                    }
                }
                value.clone()
            }
            "fuse" => Tag::Short(number(value, key)?.clamp(0.0, f64::from(i16::MAX)) as i16),
            "explosion_power" => Tag::Float(number(value, key)?.clamp(0.0, 128.0) as f32),
            "Time" => {
                let v = number(value, key)?;
                if v.fract() != 0.0 || !(0.0..=f64::from(i32::MAX)).contains(&v) {
                    return Err(error("Invalid Time: expected nonnegative integer"));
                }
                Tag::Int(v as i32)
            }
            "block_state" | "BlockState" => {
                if !matches!(value, Tag::Compound(_)) {
                    return Err(error(format!("Invalid {key}: expected compound")));
                }
                value.clone()
            }
            _ => value.clone(),
        };
        fields.push((key.clone(), value));
    }
    Ok((fields, variant))
}

pub(crate) fn prepare(
    state: &TickState,
    type_id: u32,
    name: &str,
    pos: [f64; 3],
    nbt: &Tag,
) -> Result<Image, CommandError> {
    let Tag::Compound(authored) = nbt else {
        return Err(error("Summon NBT must be a compound"));
    };
    let is_painting = name == crate::painting::PAINTING_ID;
    let (mut fields, variant) = authored_fields(state, type_id, name, authored)?;
    if is_painting {
        // Vanilla Painting defaults: facing south, the default variant.
        if !fields.iter().any(|(key, _)| key == "facing") {
            fields.push(("facing".into(), Tag::Byte(0)));
        }
        if !fields.iter().any(|(key, _)| key == "variant") {
            fields.push((
                "variant".into(),
                Tag::String(crate::painting::DEFAULT_VARIANT.into()),
            ));
        }
    }
    if name == "minecraft:falling_block" && !fields.iter().any(|(key, _)| key == "BlockState") {
        // Vanilla FallingBlock defaults to sand when BlockState is absent.
        let sand = state
            .registry
            .blocks
            .by_id(mc_world::BlockStateId(state.registry.states.sand));
        let name = sand
            .expect("sand state registered")
            .block
            .id
            .as_str()
            .to_owned();
        fields.push((
            "BlockState".into(),
            Tag::Compound(vec![("Name".into(), Tag::String(name))]),
        ));
    }
    let equipment_authored = authored.iter().any(|(key, _)| key == "equipment");
    let mut image = decode(
        state,
        type_id,
        name,
        pos,
        fields,
        equipment_authored,
        Summoned::Fresh,
    )?;
    if image.uuid_authored && image.state.faction != 0 {
        crate::scoreboard::require_no_team(&state.scoreboard, &image.native.uuid.to_string())
            .map_err(error)?;
    }
    image.variant = variant;
    Ok(image)
}

/// The image of an existing entity after a `data` edit. `image` is the whole
/// edited native NBT, private fields included, so everything a checkpoint keeps
/// (a villager's trades and gossip, mount cargo, leash, loot) decodes exactly as
/// recovery decodes it; only the `changed` fields are admitted through the
/// summon field set and normalised like a summon's.
pub(crate) fn prepare_edit(
    state: &TickState,
    type_id: u32,
    name: &str,
    pos: [f64; 3],
    mut image: Vec<(String, Tag)>,
    changed: &[(String, Tag)],
    keep_villager: bool,
) -> Result<Image, CommandError> {
    let (fields, variant) = authored_fields(state, type_id, name, changed)?;
    for (key, value) in fields {
        match image.iter_mut().find(|(seen, _)| *seen == key) {
            Some((_, old)) => *old = value,
            None => image.push((key, value)),
        }
    }
    // Identity and place are the entity's own, not data to load back.
    image.retain(|(key, _)| !matches!(key.as_str(), "id" | "Pos" | "UUID"));
    let equipment_authored = changed.iter().any(|(key, _)| key == "equipment");
    let mut edited = decode(
        state,
        type_id,
        name,
        pos,
        image,
        equipment_authored,
        Summoned::Existing,
    )?;
    edited.variant = variant;
    // An edit that leaves every villager field alone leaves the live villager
    // (and the transient brain state a reload would drop) alone too.
    edited.keep_villager = keep_villager;
    Ok(edited)
}

/// Whether an image is for a new entity or one already living.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Summoned {
    Fresh,
    Existing,
}

fn decode(
    state: &TickState,
    type_id: u32,
    name: &str,
    pos: [f64; 3],
    fields: Vec<(String, Tag)>,
    equipment_authored: bool,
    origin: Summoned,
) -> Result<Image, CommandError> {
    let is_painting = name == crate::painting::PAINTING_ID;
    let uuid_authored = fields.iter().any(|(key, _)| key == "UUID");
    let base = VanillaEntity {
        id: name.into(),
        uuid: uuid::Uuid::nil(),
        pos,
        health: state
            .registry
            .blueprint(type_id)
            .map_or(20.0, |b| b.params.max_health),
        ..Default::default()
    };
    let native = mc_data::entity_nbt::entity_from_fields(&state.registry.items, base, &fields)
        .map_err(|e| error(e.to_string()))?;
    let painting = if is_painting {
        Some(
            crate::painting::face_and_variant(&native, &state.registry)
                .map_err(|e| error(e.to_string()))?,
        )
    } else {
        None
    };
    let image = EntityStateImage::from_record(&state.registry, &native)
        .map_err(|e| error(e.to_string()))?;
    let equipment = if equipment_authored {
        Some(
            crate::entity_loot::equipment_from_fields(&state.registry.items, &native.extra_fields)
                .map_err(|e| error(e.to_string()))?,
        )
    } else {
        None
    };
    let villager = if matches!(
        name,
        "minecraft:villager" | "minecraft:zombie_villager" | "minecraft:wandering_trader"
    ) {
        let mut villager = crate::villager::persistence::villager_from_fields(
            &state.registry.items,
            &fields,
            state.tick,
        )
        .map_err(|e| error(e.to_string()))?;
        villager.wandering = name == "minecraft:wandering_trader";
        if origin == Summoned::Fresh && name == "minecraft:zombie_villager" {
            villager.conversion_time = Some(-1);
        }
        Some(villager)
    } else {
        None
    };
    Ok(Image {
        native,
        state: image,
        villager,
        equipment,
        uuid_authored,
        painting,
        keep_villager: false,
        variant: None,
    })
}
/// Allocate the canonical kind before indexing or publishing a summoned entity.
pub(crate) fn spawn(
    state: &mut TickState,
    type_id: u32,
    pos: [f64; 3],
    health: f32,
    image: Image,
) -> EntityId {
    let id = if let Some(orb) = image.native.experience_orb {
        crate::experience_orbs::spawn(state, type_id, pos, orb, [0.0; 3], 0.0)
    } else {
        state.spawn_mob(type_id, pos, health)
    };
    apply(state, id, image);
    id
}

pub(crate) fn apply(state: &mut TickState, id: EntityId, mut image: Image) {
    let slot = state.entities.slot_of(id).expect("new summon");
    let was_baby = state.entities.row(slot).appearance.baby;
    let native = image.native;
    let baby =
        native.extra_fields.iter().any(
            |(key, value)| matches!((key.as_str(), value), ("IsBaby", Tag::Byte(v)) if *v != 0),
        ) || (image.state.entity_life.child_ticks != 0
            && crate::entity_life::age_has_baby_model(&native.id))
            || image
                .villager
                .as_ref()
                .is_some_and(|villager| villager.age < 0);
    if image.uuid_authored {
        state.entities.set_uuid(slot, native.uuid);
    }
    if state.dimension_globals.active {
        let uuid = state.entities.row(slot).uuid;
        state.dimension_globals.reserved_uuids.insert(uuid);
    }
    state
        .entities
        .set_vel(slot, native.motion.map(|v| v * 20.0));
    state
        .entities
        .set_rot(slot, native.yaw, (native.pitch % 360.0).clamp(-90.0, 90.0));
    state.entities.set_ground(slot, native.on_ground);
    let row = state.entities.row_mut(slot);
    // Spawn initialization owns defaults. Only authored equipment/chances replace them.
    if image.equipment.is_none() {
        image.state.loot.body_armor = row.loot.body_armor.clone();
        image.state.loot.saddle = row.loot.saddle.clone();
    }
    let authored_chances = native.extra_fields.iter().find_map(|(key, value)| {
        if key == "drop_chances"
            && let Tag::Compound(slots) = value
        {
            return Some(slots);
        }
        None
    });
    for (index, name) in [
        "mainhand", "offhand", "feet", "legs", "chest", "head", "body", "saddle",
    ]
    .into_iter()
    .enumerate()
    {
        if !authored_chances.is_some_and(|slots| slots.iter().any(|(key, _)| key == name)) {
            if index < 6 {
                image.state.loot.drop_chances[index] = row.loot.drop_chances[index];
            } else {
                image.state.loot.extra_drop_chances[index - 6] =
                    row.loot.extra_drop_chances[index - 6];
            }
        }
    }
    if !native
        .extra_fields
        .iter()
        .any(|(key, _)| key == "CanPickUpLoot")
    {
        image.state.loot.can_pick_up_loot = row.loot.can_pick_up_loot;
    }
    state
        .entities
        .set_entity_life(slot, image.state.entity_life);
    state.entities.set_faction(slot, image.state.faction);
    let (body, row) = state.entities.parts_mut(slot);
    row.health = native.health;
    row.age = image.state.age;
    if !image.keep_villager {
        row.villager = image.villager.map(Box::new);
    }
    row.controls = image.state.controls;
    body.burn_ticks = image.state.burn_ticks;
    row.love = image.state.love;
    row.breed_cooldown = image.state.breed_cooldown;
    row.loot = image.state.loot;
    row.persistent = image.state.persistent;
    row.opaque_entity_fields = image.state.opaque_entity_fields;
    // A size changes only when the baby state does: an already small hitbox is
    // not halved again, and growing up restores the full one.
    if baby != was_baby {
        body.height *= if baby { 0.5 } else { 2.0 };
    }
    let appearance = Arc::make_mut(&mut row.appearance);
    if let Some(equipment) = image.equipment {
        appearance.equipment = equipment;
    }
    appearance.health = native.health;
    if let Some(orb) = native.experience_orb {
        appearance.experience_orb = Some(orb);
    }
    appearance.baby = baby;
    crate::entity_controls::sync_no_gravity(appearance, row.controls.no_gravity);
    if let Some((index, value)) = image.variant {
        appearance.extra.retain(|(existing, _)| *existing != index);
        appearance.extra.push((index, value));
    }
    if body.burn_ticks > 0 {
        appearance.shared_flags |= 1;
    } else {
        appearance.shared_flags &= !1;
    }
    if let Some(wool) = native.sheep_wool {
        appearance
            .extra
            .retain(|(index, _)| *index != crate::blueprint::SHEEP_WOOL_DATA_INDEX);
        appearance.extra.push((
            crate::blueprint::SHEEP_WOOL_DATA_INDEX,
            EntityDataValue::Byte(wool as i8),
        ));
    }
    if native.id == "minecraft:snow_golem"
        && let Some((_, Tag::Byte(pumpkin))) =
            native.extra_fields.iter().find(|(key, _)| key == "Pumpkin")
    {
        crate::entity_life::set_snow_golem_pumpkin(appearance, *pumpkin != 0);
    }
    appearance.custom_name = native
        .extra_fields
        .iter()
        .find_map(|(key, value)| match value {
            Tag::String(name) if key == "CustomName" => Some(name.clone()),
            _ => None,
        });
    appearance.custom_name_visible = native.extra_fields.iter().any(
        |(key, value)| matches!((key.as_str(), value), ("CustomNameVisible", Tag::Byte(v)) if *v != 0),
    );
    crate::entity_life::restore_appearance(body, row);
    if baby != was_baby {
        let half_width = body.half_width * if baby { 0.5 } else { 2.0 };
        state.entities.set_half_width(slot, half_width);
    }
    // Authored primed-TNT and falling-block payloads replace the spawn defaults.
    if image.state.tnt.is_some() || image.state.falling.is_some() {
        let row = state.entities.row_mut(slot);
        if let Some(record) = image.state.tnt {
            row.tnt = Some(record);
        }
        if let Some(falling) = image.state.falling {
            row.falling = Some(falling);
        }
    }
    if image.state.tnt.is_some() {
        crate::tnt::restore(state, slot);
    }
    if image.state.falling.is_some() {
        crate::falling::configure(state, id);
    }
    crate::entity_loot::restore(state, slot);
    crate::armor_stand::restore(state, slot);
    if let Some((face, variant)) = image.painting {
        crate::painting::set(state, slot, face, variant);
    }
    crate::taming::sync(state, slot);
    crate::villager::sync_appearance(state, slot);
    state.appearance_changed.push(id);
}

/// Install a prepared item payload on the existing item entity `id`: its stack,
/// gravity, invulnerability, pickup delay, age and motion. Summon and
/// `data merge entity` share this, so both author the same fields.
pub(crate) fn apply_item(state: &mut TickState, id: EntityId, item: ItemSummon) {
    let Some(slot) = state.entities.slot_of(id) else {
        return;
    };
    crate::entity_controls::set_no_gravity(state, slot, item.no_gravity);
    let row = state.entities.row_mut(slot);
    row.controls.invulnerable = item.invulnerable;
    if let Some(tags) = item.tags {
        row.opaque_entity_fields = tags;
    }
    row.item = Some(Box::new(crate::entities::StackRow {
        item_id: item.stack.item_id,
        count: item.stack.count,
        stack: Some(item.stack.clone()),
    }));
    Arc::make_mut(&mut row.appearance).item = Some(item.stack);
    if let Some(age) = item.age {
        row.age = age;
    }
    if let Some(delay) = item.pickup_delay {
        state.entities.body_mut(slot).pickup_delay = delay;
    }
    if let Some(motion) = item.motion {
        state.entities.set_vel(slot, motion.map(|v| v * 20.0));
    }
    state.appearance_changed.push(id);
}

#[cfg(test)]
#[path = "command_summon_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "command_summon_age_tests.rs"]
mod age_tests;

#[cfg(test)]
#[path = "command_summon_hanging_tests.rs"]
mod hanging_tests;

#[path = "command_summon_data.rs"]
pub(crate) mod data;
