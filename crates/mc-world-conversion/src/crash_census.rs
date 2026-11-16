//! Durable crash-census projection for a disposable copy of a stopped world.
use std::collections::BTreeMap;

use mc_core::native_checkpoint as native;
use mc_nbt::Tag;
use serde::Serialize;

use crate::{CoreConfig, CoreError};

#[derive(Serialize)]
pub struct CrashCensus {
    pub checkpoint_tick: u64,
    pub durable_tick: u64,
    pub entities: BTreeMap<String, CrashEntity>,
}

#[derive(Serialize)]
pub struct CrashEntity {
    pub kind: String,
    pub holders: BTreeMap<String, String>,
}

fn holders(fields: &[(String, Tag)]) -> BTreeMap<String, String> {
    fields
        .iter()
        .filter(|(name, _)| {
            matches!(
                name.as_str(),
                "Inventory"
                    | "EnderItems"
                    | "Items"
                    | "HandItems"
                    | "ArmorItems"
                    | "Item"
                    | "equipment"
            )
        })
        .map(|(name, tag)| (name.clone(), native::format_census_tag(tag)))
        .collect()
}

fn stack_tag(
    items: &mc_data::items::ItemRegistry,
    stack: &mc_data::ItemStack,
) -> Result<Tag, CoreError> {
    mc_data::item_nbt::stack_to_fields(items, stack)
        .map(Tag::Compound)
        .map_err(|error| CoreError::Storage(error.to_string()))
}

fn stack_list<'a>(
    items: &mc_data::items::ItemRegistry,
    stacks: impl Iterator<Item = (Option<usize>, &'a mc_data::ItemStack)>,
) -> Result<Tag, CoreError> {
    let mut elements = Vec::new();
    for (slot, stack) in stacks {
        if stack.is_empty() {
            continue;
        }
        let Tag::Compound(mut fields) = stack_tag(items, stack)? else {
            unreachable!()
        };
        if let Some(slot) = slot {
            fields.push(("Slot".into(), Tag::Byte(slot as i8)));
        }
        elements.push(Tag::Compound(fields));
    }
    Ok(Tag::List(mc_nbt::ListTag {
        element_type: mc_nbt::tag_type::COMPOUND,
        elements,
    }))
}

fn entity_fields(
    entity: &mc_core::journal::ReplayEntity,
    kind: &str,
    items: &mc_data::items::ItemRegistry,
) -> Result<Vec<(String, Tag)>, CoreError> {
    let mut fields = if entity.state.opaque_entity_fields.is_empty() {
        Vec::new()
    } else {
        match mc_nbt::read_named(&mut entity.state.opaque_entity_fields.as_slice())
            .map_err(|error| CoreError::Storage(error.to_string()))?
            .1
        {
            Tag::Compound(fields) => fields,
            _ => {
                return Err(CoreError::Storage(
                    "crash census: invalid opaque entity fields".into(),
                ));
            }
        }
    };
    let mut equipment = Vec::new();
    for (name, stack) in ["mainhand", "offhand", "feet", "legs", "chest", "head"]
        .into_iter()
        .zip(entity.appearance.equipment.iter().map(Option::as_ref))
        .chain([
            ("body", entity.state.loot.body_armor.as_deref()),
            ("saddle", entity.state.loot.saddle.as_deref()),
        ])
    {
        if let Some(stack) = stack.filter(|stack| !stack.is_empty()) {
            equipment.push((name.into(), stack_tag(items, stack)?));
        }
    }
    fields.retain(|(name, _)| name != "equipment" && name != "Item");
    fields.push(("equipment".into(), Tag::Compound(equipment)));
    if let Some(villager) = &entity.villager {
        fields.retain(|(name, _)| name != "Inventory");
        fields.push((
            "Inventory".into(),
            stack_list(
                items,
                villager
                    .inventory
                    .iter()
                    .flatten()
                    .map(|stack| (None, stack)),
            )?,
        ));
    } else if let Some(allay) = &entity.state.allay {
        fields.retain(|(name, _)| name != "Inventory");
        fields.push((
            "Inventory".into(),
            stack_list(items, std::iter::once((None, &allay.inventory)))?,
        ));
    }
    let mount = &entity.state.entity_life.mount;
    if mount.chested
        && matches!(
            kind,
            "minecraft:donkey" | "minecraft:mule" | "minecraft:llama" | "minecraft:trader_llama"
        )
    {
        fields.retain(|(name, _)| name != "Items");
        fields.push((
            "Items".into(),
            stack_list(
                items,
                mount
                    .cargo
                    .iter()
                    .enumerate()
                    .map(|(slot, stack)| (Some(slot), stack)),
            )?,
        ));
    }
    if let Some(vehicle) = &entity.state.entity_life.vehicle {
        if vehicle.loot_table.is_none() && !vehicle.slots.is_empty() {
            fields.retain(|(name, _)| name != "Items");
            fields.push((
                "Items".into(),
                stack_list(
                    items,
                    vehicle
                        .slots
                        .iter()
                        .enumerate()
                        .map(|(slot, stack)| (Some(slot), stack)),
                )?,
            ));
        }
    }
    let mut item = entity.item.as_ref();
    if matches!(kind, "minecraft:item_frame" | "minecraft:glow_item_frame") {
        for (index, value) in &entity.appearance.extra {
            if let (9, mc_core::journal::EntityDataValueImage::Item(frame_item)) = (*index, value) {
                item = frame_item.as_ref();
            }
        }
    }
    if let Some(projectile) = &entity.projectile {
        let key = match kind {
            "minecraft:arrow" | "minecraft:spectral_arrow" | "minecraft:trident" => Some("item"),
            "minecraft:snowball"
            | "minecraft:egg"
            | "minecraft:ender_pearl"
            | "minecraft:eye_of_ender"
            | "minecraft:potion"
            | "minecraft:lingering_potion"
            | "minecraft:experience_bottle" => Some("Item"),
            "minecraft:firework_rocket" => Some("FireworksItem"),
            _ => None,
        };
        if let Some(key) = key {
            fields.retain(|(name, _)| name != key);
            if !projectile.stack.is_empty() {
                fields.push((key.into(), stack_tag(items, &projectile.stack)?));
            }
            item = None;
        }
    }
    if let Some(item) = item {
        fields.push(("Item".into(), stack_tag(items, item)?));
    }
    Ok(fields)
}

/// Repair redb on the caller's disposable copy, then project the checkpoint and
/// contiguous published WAL without starting the tick owner. The supplied world
/// must be a copy: redb can write its crash-recovery metadata while opening.
pub fn crash_census(config: &CoreConfig) -> Result<CrashCensus, CoreError> {
    let path = config.world_dir.join("native/state.redb");
    let identity = native::native_identity(config);
    drop(
        mc_world::native::NativeStore::open(&path, identity)
            .map_err(|error| native::open_refusal(config, &path, error))?,
    );
    let world = native::recover_offline(config)?;
    let checkpoint_tick = world.manifest.tick;
    let mut entities = BTreeMap::new();
    for replay in world.dimensions.values() {
        for entity in replay.entities.values() {
            let kind = world
                .registry
                .entity_type_name(entity.type_id)
                .ok_or_else(|| {
                    CoreError::Storage(format!(
                        "crash census: unknown entity type {}",
                        entity.type_id
                    ))
                })?
                .to_owned();
            let fields = entity_fields(entity, &kind, &world.registry.items)?;
            let holders = holders(&fields);
            let uuid = entity
                .uuid
                .parse::<uuid::Uuid>()
                .map_err(|error| CoreError::Storage(error.to_string()))?;
            if entities
                .insert(uuid.to_string(), CrashEntity { kind, holders })
                .is_some()
            {
                return Err(CoreError::Storage(format!(
                    "crash census: duplicate entity {uuid}"
                )));
            }
        }
    }
    let root = &world.dimensions[&mc_core::dimensions::Dimension::Overworld];
    for (uuid, player) in &root.players {
        let Tag::Compound(fields) = mc_core::journal::player_to_tag(player, &world.registry.items)?
        else {
            return Err(CoreError::Storage(
                "crash census: player is not a compound".into(),
            ));
        };
        let uuid = uuid
            .parse::<uuid::Uuid>()
            .map_err(|error| CoreError::Storage(error.to_string()))?;
        // `/data get` exposes the absent durable ender chest as an empty list.
        let mut holders = holders(&fields);
        holders
            .entry("EnderItems".into())
            .or_insert_with(|| "[]".into());
        if entities
            .insert(
                uuid.to_string(),
                CrashEntity {
                    kind: "player".into(),
                    holders,
                },
            )
            .is_some()
        {
            return Err(CoreError::Storage(format!(
                "crash census: duplicate player {uuid}"
            )));
        }
    }
    Ok(CrashCensus {
        checkpoint_tick,
        durable_tick: root.tick,
        entities,
    })
}

#[cfg(test)]
#[path = "crash_census_tests.rs"]
mod tests;
