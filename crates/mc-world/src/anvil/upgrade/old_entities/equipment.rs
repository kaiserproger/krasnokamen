//! Armor/hand equipment, spawner payloads, mount attachment and health.

use super::*;

/// v100 EntityEquipmentToArmorAndHandFix: Equipment[0..4] -> ArmorItems +
/// Equipment[1 spare slot] -> HandItems; DropChances[0] -> HandDropChances,
/// DropChances[1..4] -> ArmorDropChances. Only fills targets that are absent.
/// Elements coerce via asFloat(0) (non-numerics become 0, no slot shifting);
/// a non-list DropChances is stripped with NO default fill.
pub(super) fn equipment(tag: &mut Tag) -> bool {
    let mut changed = false;
    // Drop chances first (vanilla fixDropChances runs in the same rule).
    if let Some(chances) = nbt::take(tag, "DropChances") {
        changed = true;
        if let Tag::List(list) = chances {
            let mut rest = list
                .elements
                .into_iter()
                .map(|value| nbt::number(&value).unwrap_or(0.0) as f32);
            let first = rest.next().unwrap_or(0.0);
            if nbt::get(tag, "HandDropChances").is_none() {
                nbt::set(
                    tag,
                    "HandDropChances",
                    Tag::List(mc_nbt::ListTag {
                        element_type: mc_nbt::tag_type::FLOAT,
                        elements: vec![Tag::Float(first), Tag::Float(0.0)],
                    }),
                );
                changed = true;
            }
            if nbt::get(tag, "ArmorDropChances").is_none() {
                let armor: Vec<Tag> = rest.map(Tag::Float).collect();
                let mut armor = armor;
                while armor.len() < 4 {
                    armor.push(Tag::Float(0.0));
                }
                armor.truncate(4);
                nbt::set(
                    tag,
                    "ArmorDropChances",
                    Tag::List(mc_nbt::ListTag {
                        element_type: mc_nbt::tag_type::FLOAT,
                        elements: armor,
                    }),
                );
                changed = true;
            }
        }
    }
    let Some(equipment) = nbt::take(tag, "Equipment") else {
        return changed;
    };
    changed = true;
    let items = match equipment {
        Tag::List(list) => list.elements,
        _ => return changed,
    };
    let has_armor = items.len() > 1;
    let mut items = items.into_iter();
    let first = items.next();
    if let Some(first) = first.filter(|_| nbt::get(tag, "HandItems").is_none()) {
        let hand = vec![first, empty_item()];
        nbt::set(
            tag,
            "HandItems",
            Tag::List(mc_nbt::ListTag {
                element_type: mc_nbt::tag_type::COMPOUND,
                elements: hand,
            }),
        );
        changed = true;
    }
    if has_armor && nbt::get(tag, "ArmorItems").is_none() {
        let mut armor: Vec<Tag> = items.take(4).collect();
        while armor.len() < 4 {
            armor.push(empty_item());
        }
        nbt::set(
            tag,
            "ArmorItems",
            Tag::List(mc_nbt::ListTag {
                element_type: mc_nbt::tag_type::COMPOUND,
                elements: armor,
            }),
        );
        changed = true;
    }
    changed
}

fn empty_item() -> Tag {
    nbt::compound(vec![])
}

/// v106 MobSpawnerEntityIdentifiersFix (UNTAGGED_SPAWNER scope): rewrites the
/// untyped spawner remainder in place. EntityId string -> SpawnData{id}
/// (empty EntityId defaults the spawned id to Pig; missing EntityId leaves
/// SpawnData untouched), EntityId removed; SpawnPotentials entries WITH a
/// Type string get {Entity: {id from Type plus preserved Properties fields}},
/// Type/Properties removed, all sibling keys (Weight, etc.) preserved;
/// Type-less entries are left UNCHANGED. Non-string EntityId is kept.
///
/// Scope routing: the block-entity MobSpawner arm stamps the wrapper id
/// MobSpawner (vanilla lambda$makeRule$0 sets id before fix/read); the
/// MinecartSpawner entity arm transforms only the nested spawner fields and
/// MUST NOT touch the outer entity id. Shared field logic lives in
/// spawner_fields so the entity path cannot clobber it.
pub(super) fn spawner_identifiers(tag: &mut Tag) -> bool {
    // BE path preserves vanilla lambda$makeRule$0: id stamped MobSpawner
    // unconditionally, field rewrite applied on top.
    spawner_fields(tag);
    nbt::set(tag, "id", text("MobSpawner"));
    true
}
/// Returns false when neither EntityId nor SpawnPotentials changed, so the
/// entity-nested MinecartSpawner arm stays a no-op (no report, id kept).
pub(super) fn spawner_fields(tag: &mut Tag) -> bool {
    let mut changed = false;
    if let Some(Tag::String(entity)) = nbt::get_mut(tag, "EntityId") {
        let mut id = std::mem::take(entity);
        if id.is_empty() {
            id.push_str("Pig");
        }
        let mut data = nbt::take(tag, "SpawnData").unwrap_or_else(|| nbt::compound(vec![]));
        if nbt::fields(&data).is_some() {
            nbt::set(&mut data, "id", Tag::String(id));
        }
        nbt::set(tag, "SpawnData", data);
        nbt::remove(tag, "EntityId");
        changed = true;
    }
    if let Some(potentials) = nbt::get_mut(tag, "SpawnPotentials").and_then(nbt::list_mut) {
        for entry in potentials.iter_mut() {
            let Some(Tag::String(kind)) = nbt::get_mut(entry, "Type") else {
                continue;
            };
            let kind = std::mem::take(kind);
            let properties =
                nbt::take(entry, "Properties").unwrap_or_else(|| nbt::compound(vec![]));
            let mut entity = properties;
            if nbt::fields(&entity).is_none() {
                entity = nbt::compound(vec![]);
            }
            nbt::set(&mut entity, "id", Tag::String(kind));
            nbt::remove(entry, "Type");
            nbt::remove(entry, "Properties");
            nbt::set(entry, "Entity", entity);
            changed = true;
        }
    }
    changed
}

/// v109 EntityHealthFix: HealF (else Health) -> Health float, HealF removed.
/// When neither is present the node is untouched.
pub(super) fn health(tag: &mut Tag) -> bool {
    let healf = nbt::get(tag, "HealF").and_then(nbt::number);
    let health = nbt::get(tag, "Health").and_then(nbt::number);
    let Some(value) = healf.or(health) else {
        return false;
    };
    nbt::remove(tag, "HealF");
    nbt::set(tag, "Health", Tag::Float(value as f32));
    true
}

/// v110 EntityHorseSaddleFix: Saddle=true without SaddleItem ->
/// SaddleItem{minecraft:saddle, Count 1, Damage 0}, Saddle removed.
pub(super) fn horse_saddle(tag: &mut Tag) -> bool {
    if nbt::get(tag, "SaddleItem").is_some() {
        return false;
    }
    if nbt::get(tag, "Saddle")
        .and_then(nbt::number)
        .is_none_or(|v| v as i8 == 0)
    {
        return false;
    }
    nbt::set(
        tag,
        "SaddleItem",
        nbt::compound(vec![
            ("id", text("minecraft:saddle")),
            ("Count", Tag::Byte(1)),
            ("Damage", Tag::Short(0)),
        ]),
    );
    nbt::remove(tag, "Saddle");
    true
}

/// v113 EntityRedundantChanceTagsFix: drop-chance lists that are all zero
/// (HandDropChances len 2, ArmorDropChances len 4) are removed.
pub(super) fn redundant_chances(tag: &mut Tag) -> bool {
    let mut changed = false;
    for (key, len) in [("HandDropChances", 2), ("ArmorDropChances", 4)] {
        if is_zero_float_list(tag, key, len) {
            nbt::remove(tag, key);
            changed = true;
        }
    }
    changed
}

/// v135 EntityRidingToPassengersFix: invert each Riding edge in order,
/// carrying the accumulated passenger subtree onto the next vehicle.
pub(super) fn riding(tag: &mut Tag) -> bool {
    if nbt::get(tag, "Riding").is_none() {
        return false;
    }
    let mut rider = std::mem::replace(tag, nbt::compound(vec![]));
    while let Some(mut vehicle) = nbt::take(&mut rider, "Riding") {
        if nbt::fields(&vehicle).is_none() {
            break;
        }
        nbt::set(
            &mut vehicle,
            "Passengers",
            Tag::List(mc_nbt::ListTag {
                element_type: mc_nbt::tag_type::COMPOUND,
                elements: vec![rider],
            }),
        );
        rider = vehicle;
    }
    *tag = rider;
    true
}
