//! Live data edits share summon admission/codecs, never spawn initialization.
use super::*;
use crate::entities::{EntityKind, EntitySlot};
use mc_data::item_nbt::field;

const ARMOR_STAND: &[&str] = &["Small", "ShowArms", "NoBasePlate", "Marker", "Invisible", "DisabledSlots", "Pose"];

struct Prepared {
    image: Image,
    item: Option<ItemSummon>,
    health: Option<f32>,
    air: Option<i32>,
    position: Option<[f64; 3]>,
    retained: Option<Vec<u8>>,
    custody: crate::plugin_inventories::PreparedEntityCustody,
}

pub(crate) fn update(
    state: &mut TickState,
    id: EntityId,
    before: &Tag,
    candidate: &Tag,
) -> Result<(), String> {
    let slot = state.entities.slot_of(id).ok_or("No entity was found")?;
    let type_id = match state.entities.body(slot).kind {
        EntityKind::Player { .. } => return Err("Unable to modify player data".into()),
        EntityKind::Mob { type_id } | EntityKind::Item { type_id }
        | EntityKind::ExperienceOrb { type_id } => type_id,
    };
    let name = state.registry.entity_type_name(type_id).ok_or("Unknown entity type")?.to_owned();
    let (Tag::Compound(before), Tag::Compound(candidate)) = (before, candidate) else {
        return Err("Entity data must be a compound".into());
    };
    let mut keys: Vec<_> = before.iter().chain(candidate).map(|(key, _)| key.as_str()).collect();
    keys.sort_unstable();
    keys.dedup();
    keys.retain(|key| field(before, key) != field(candidate, key));
    let changed = |key: &str| keys.contains(&key);
    if keys.iter().any(|key| key.starts_with("solaris") || key.starts_with("Solaris")) {
        return Err("Internal entity fields cannot be edited".into());
    }
    let item = name == "minecraft:item";
    let item_changed = item && ["Item", "PickupDelay", "Age"].iter().any(|key| changed(key));
    let item_image = if item_changed {
        let fields = candidate.iter().filter(|(key, _)| matches!(key.as_str(), "Item" | "PickupDelay" | "Age"))
            .cloned().collect();
        Some(prepare_item(state, &Tag::Compound(fields)).map_err(|error| error.message())?)
    } else { None };
    let special = |key: &str| matches!(key, "id" | "UUID" | "Pos" | "Health" | "Air" | "Tags")
        || (item && matches!(key, "Item" | "PickupDelay" | "Age"));
    // Coupled vanilla values are decoded together, while unrelated native state
    // is never part of the commit (brain, passenger graph, hit timers, custody).
    let coupled = |key: &str| {
        (["Color", "Sheared"].contains(&key) && (changed("Color") || changed("Sheared")))
            || (["facing", "variant"].contains(&key) && (changed("facing") || changed("variant")))
            || (ARMOR_STAND.contains(&key) && ARMOR_STAND.iter().any(|key| changed(key)))
    };
    let authored = candidate.iter().filter(|(key, _)| !special(key) && (changed(key) || coupled(key)))
        .cloned().collect();
    let removed: Vec<_> = before.iter().filter(|(key, _)| changed(key) && !special(key)
        && field(candidate, key).is_none()).cloned().collect();
    let pos = state.entities.pos(slot);
    if !removed.is_empty() {
        prepare(state, type_id, &name, pos, &Tag::Compound(removed)).map_err(|error| error.message())?;
    }
    let image = prepare(state, type_id, &name, pos, &Tag::Compound(authored))
        .map_err(|error| error.message())?;
    let health = if changed("Health") {
        let value = field(candidate, "Health").map(|tag| number(tag, "Health")).transpose()
            .map_err(|error| error.message())?.unwrap_or_else(|| state.registry.blueprint(type_id)
                .map_or(20.0, |blueprint| f64::from(blueprint.params.max_health)));
        if !(0.0..=f64::from(f32::MAX)).contains(&value) { return Err("Invalid Health".into()); }
        Some(value as f32)
    } else { None };
    let air = if changed("Air") {
        let value = field(candidate, "Air").map(|tag| number(tag, "Air")).transpose()
            .map_err(|error| error.message())?.unwrap_or(300.0);
        if value.fract() != 0.0 || !(i32::MIN as f64..=i32::MAX as f64).contains(&value) {
            return Err("Invalid Air".into());
        }
        Some(value as i32)
    } else { None };
    let position = if changed("Pos") {
        let Some(Tag::List(list)) = field(candidate, "Pos") else { return Err("Invalid Pos".into()); };
        if list.elements.len() != 3 { return Err("Pos must contain three values".into()); }
        let mut position = [0.0; 3];
        for (axis, tag) in position.iter_mut().zip(&list.elements) {
            *axis = number(tag, "Pos").map_err(|error| error.message())?;
            if *axis < i32::MIN as f64 || *axis > i32::MAX as f64 {
                return Err("Entity position is outside supported world coordinates".into());
            }
        }
        Some(position)
    } else { None };
    let retained = prepare_retained(state, slot, candidate, &image.native.extra_fields, &keys)?;
    let custody = if changed("equipment") {
        crate::plugin_inventories::prepare_entity_custody(
            state, slot, image.equipment.as_ref().unwrap_or(&Default::default()),
        ).map_err(|_| "Entity inventory custody is unavailable")?
    } else {
        Default::default()
    };
    // All fallible work is above this point. Identity and the dense entity slot
    // remain untouched, including an authored UUID (vanilla restores its UUID).
    commit(state, slot, id, &name, &keys, Prepared {
        image, item: item_image, health, air, position, retained, custody,
    });
    Ok(())
}

fn prepare_retained(
    state: &TickState,
    slot: EntitySlot,
    candidate: &[(String, Tag)],
    normalized: &[(String, Tag)],
    keys: &[&str],
) -> Result<Option<Vec<u8>>, String> {
    if !keys.iter().any(|key| *key == "Tags" || ARMOR_STAND.contains(key)) { return Ok(None); }
    if keys.contains(&"Tags") && let Some(tag) = field(candidate, "Tags") {
        let Tag::List(list) = tag else { return Err("Tags must be a string list".into()); };
        if list.elements.len() > 1024 || list.elements.iter().any(|tag| !matches!(tag, Tag::String(_))) {
            return Err("Tags must contain at most 1024 strings".into());
        }
    }
    let bytes = &state.entities.row(slot).opaque_entity_fields;
    let mut fields = if bytes.is_empty() { Vec::new() } else {
        let (_, Tag::Compound(fields)) = mc_nbt::read_named(&mut bytes.as_slice()).map_err(|error| error.to_string())?
        else { return Err("Invalid retained entity fields".into()); };
        fields
    };
    for key in keys.iter().filter(|key| **key == "Tags" || ARMOR_STAND.contains(key)) {
        fields.retain(|(name, _)| name != key);
        let source = if *key == "Tags" { candidate } else { normalized };
        if field(candidate, key).is_some() && let Some(value) = field(source, key) {
            fields.push(((*key).into(), value.clone()));
        }
    }
    let mut bytes = Vec::new();
    mc_nbt::write_named(&mut bytes, "", &Tag::Compound(fields)).map_err(|error| error.to_string())?;
    Ok(Some(bytes))
}

fn commit(
    state: &mut TickState,
    slot: EntitySlot,
    id: EntityId,
    name: &str,
    keys: &[&str],
    prepared: Prepared,
) {
    let Prepared { mut image, item, health, air, position, retained, custody } = prepared;
    let changed = |key: &str| keys.contains(&key);
    if let Some(position) = position { state.entities.set_pos(slot, position); }
    if changed("Motion") { state.entities.set_vel(slot, image.native.motion.map(|value| value * 20.0)); }
    if changed("Rotation") { state.entities.set_rot(slot, image.native.yaw, (image.native.pitch % 360.0).clamp(-90.0, 90.0)); }
    if changed("OnGround") { state.entities.set_ground(slot, image.native.on_ground); }
    if changed("leash") { state.entities.set_leash(slot, image.state.entity_life.leash.take()); }
    let old_baby = state.entities.row(slot).appearance.baby;
    let old_width = state.entities.body(slot).half_width;
    let mut baby_changed = None;
    {
        let (body, row) = state.entities.parts_mut(slot);
        if let Some(health) = health { row.health = health; }
        if let Some(air) = air { body.air_supply = air; }
        if let Some(retained) = retained { row.opaque_entity_fields = retained; }
        for key in keys {
            match *key {
                "Fire" => body.burn_ticks = image.state.burn_ticks,
                "NoAI" => row.controls.no_ai = image.state.controls.no_ai,
                "Invulnerable" => row.controls.invulnerable = image.state.controls.invulnerable,
                "FallDistance" => row.controls.fall_distance = image.state.controls.fall_distance,
                "PersistenceRequired" => row.persistent = image.state.persistent,
                "CanPickUpLoot" => row.loot.can_pick_up_loot = image.state.loot.can_pick_up_loot,
                "InLove" => row.love = image.state.love,
                "LoveCause" => row.entity_life.taming.love_cause = image.state.entity_life.taming.love_cause,
                "Owner" => row.entity_life.taming.owner = image.state.entity_life.taming.owner,
                "Tame" => row.entity_life.tamed = image.state.entity_life.tamed,
                "Sitting" => row.entity_life.sitting = image.state.entity_life.sitting,
                "Temper" => row.entity_life.taming.temper = image.state.entity_life.taming.temper,
                "CollarColor" => row.entity_life.taming.collar = image.state.entity_life.taming.collar,
                "Trusting" => row.entity_life.taming.trusting = image.state.entity_life.taming.trusting,
                "Trusted" => row.entity_life.taming.trusted = std::mem::take(&mut image.state.entity_life.taming.trusted),
                "Type" => row.entity_life.brown_mooshroom = image.state.entity_life.brown_mooshroom,
                "stew_effects" => row.entity_life.stew_effects = std::mem::take(&mut image.state.entity_life.stew_effects),
                "Size" => row.loot.slime_size = image.state.loot.slime_size,
                "drop_chances" => {
                    row.loot.drop_chances = image.state.loot.drop_chances;
                    row.loot.extra_drop_chances = image.state.loot.extra_drop_chances;
                }
                "equipment" => {
                    row.loot.body_armor = image.state.loot.body_armor.take();
                    row.loot.saddle = image.state.loot.saddle.take();
                }
                "VillagerData" => {
                    if let (Some(current), Some(next)) = (row.villager.as_mut(), image.villager.as_ref()) {
                        current.data = next.data;
                    }
                }
                "Age" if name != "minecraft:item" => {
                    row.entity_life.child_ticks = image.state.entity_life.child_ticks;
                    row.entity_life.age_lock.tadpole_age = image.state.entity_life.age_lock.tadpole_age;
                    row.breed_cooldown = image.state.breed_cooldown;
                    if let (Some(current), Some(next)) = (row.villager.as_mut(), image.villager.as_ref()) { current.age = next.age; }
                }
                _ => {}
            }
        }
        let appearance = Arc::make_mut(&mut row.appearance);
        if let Some(health) = health { appearance.health = health; }
        if changed("Fire") {
            appearance.shared_flags = (appearance.shared_flags & !1) | u8::from(body.burn_ticks > 0);
        }
        if changed("CustomName") {
            appearance.custom_name = image.native.extra_fields.iter().find_map(|(key, value)| {
                if key == "CustomName" && let Tag::String(value) = value { Some(value.clone()) } else { None }
            });
        }
        if changed("CustomNameVisible") {
            appearance.custom_name_visible = matches!(field(&image.native.extra_fields, "CustomNameVisible"), Some(Tag::Byte(value)) if *value != 0);
        }
        if changed("equipment") { appearance.equipment = image.equipment.take().unwrap_or_default(); }
        if changed("Color") || changed("Sheared") {
            appearance.extra.retain(|(index, _)| *index != crate::blueprint::SHEEP_WOOL_DATA_INDEX);
            appearance.extra.push((crate::blueprint::SHEEP_WOOL_DATA_INDEX, EntityDataValue::Byte(image.native.sheep_wool.unwrap_or(0) as i8)));
        }
        if changed("Pumpkin") {
            crate::entity_life::set_snow_golem_pumpkin(appearance,
                !matches!(field(&image.native.extra_fields, "Pumpkin"), Some(Tag::Byte(0))));
        }
        if changed("Age") || changed("IsBaby") {
            let baby = matches!(field(&image.native.extra_fields, "IsBaby"), Some(Tag::Byte(value)) if *value != 0)
                || (row.entity_life.child_ticks != 0 && crate::entity_life::age_has_baby_model(name))
                || row.villager.as_ref().is_some_and(|villager| villager.age < 0);
            appearance.baby = baby;
            if baby != old_baby {
                let factor = if baby { 0.5 } else { 2.0 };
                body.height *= factor;
                baby_changed = Some(old_width * factor);
            }
        }
        if let Some(item) = item {
            if changed("Age") { row.age = item.age.unwrap_or(0); }
            if changed("PickupDelay") { body.pickup_delay = item.pickup_delay.unwrap_or(0); }
            if changed("Item") {
                let stack = item.stack;
                crate::entities::set_boxed(&mut row.item, crate::entities::StackRow {
                    item_id: stack.item_id, count: stack.count, stack: Some(stack.clone()),
                });
                appearance.item = Some(stack);
            }
        }
    }
    if let Some(width) = baby_changed { state.entities.set_half_width(slot, width); }
    if changed("equipment") || changed("Size") { crate::entity_loot::restore(state, slot); }
    if ARMOR_STAND.iter().any(|key| changed(key)) { crate::armor_stand::restore(state, slot); }
    if (changed("facing") || changed("variant")) && let Some((face, variant)) = image.painting {
        crate::painting::set(state, slot, face, variant);
    }
    if ["Type", "stew_effects"].iter().any(|key| changed(key)) {
        let (body, row) = state.entities.parts_mut(slot);
        crate::entity_life::restore_appearance(body, row);
    }
    if ["Owner", "Tame", "Sitting", "Temper", "CollarColor", "Trusting", "Trusted"].iter().any(|key| changed(key)) {
        crate::taming::sync(state, slot);
    }
    if changed("VillagerData") || changed("Age") { crate::villager::sync_appearance(state, slot); }
    crate::plugin_inventories::apply_entity_custody(state, custody);
    if health == Some(0.0) {
        state.merge_damage.push((slot, crate::tick::DamagePiece {
            physical: None, custom_armor: false, amount: f32::MAX,
            kind: crate::survival::DamageKind::GenericKill, source: None,
        }));
    }
    state.appearance_changed.push(id);
}
