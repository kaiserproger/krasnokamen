//! UUID forms, owner references and the entity-class predicates they need.

use super::*;

/// v2511 EntityProjectileOwnerFix, three disjoint shapes:
/// - arrows (arrow/spectral_arrow/trident): top-level OwnerUUIDMost/Least ->
///   OwnerUUID int-array (zero-guard), both keys removed;
/// - llama_spit only: nested Owner{OwnerUUIDMost,OwnerUUIDLeast} -> parent
///   OwnerUUID int-array when both non-zero; Owner ALWAYS removed;
/// - throwables (egg/ender_pearl/xp_bottle/snowball/potion): lowercase
///   owner{M,L} -> OwnerUUID when both non-zero; owner ALWAYS removed.
pub(super) fn projectile_owner(tag: &mut Tag) -> bool {
    let id = nbt::get(tag, "id").and_then(nbt::string).unwrap_or("");
    if id == "minecraft:arrow" || id == "minecraft:spectral_arrow" || id == "minecraft:trident" {
        return old_player_map::replace_uuid(tag, "OwnerUUID", "OwnerUUID");
    }
    if id == "minecraft:llama_spit" {
        if let Some(owner) = nbt::take(tag, "Owner") {
            return llama_owner(tag, owner);
        }
        return false;
    }
    if let Some(owner) = nbt::take(tag, "owner") {
        return throwable_owner(tag, owner);
    }
    false
}

/// Llama-spit nested owner conversion (called with the taken Owner compound).
fn llama_owner(parent: &mut Tag, owner: Tag) -> bool {
    let most = nbt::get(&owner, "OwnerUUIDMost").map_or(0, long);
    let least = nbt::get(&owner, "OwnerUUIDLeast").map_or(0, long);
    if most != 0 && least != 0 {
        set_uuid_array(parent, "OwnerUUID", most, least);
    }
    true
}

/// Throwable lowercase owner conversion (called with the taken owner compound).
fn throwable_owner(parent: &mut Tag, owner: Tag) -> bool {
    let (most, least) = match &owner {
        Tag::Compound(fields) => {
            let most = fields
                .iter()
                .find(|(k, _)| k == "M")
                .map_or(0, |(_, value)| long(value));
            let least = fields
                .iter()
                .find(|(k, _)| k == "L")
                .map_or(0, |(_, value)| long(value));
            (most, least)
        }
        _ => (0, 0),
    };
    if most != 0 && least != 0 {
        set_uuid_array(parent, "OwnerUUID", most, least);
    }
    true
}

fn set_uuid_array(tag: &mut Tag, key: &str, most: i64, least: i64) {
    nbt::set(
        tag,
        key,
        Tag::IntArray(vec![
            (most >> 32) as i32,
            most as i32,
            (least >> 32) as i32,
            least as i32,
        ]),
    );
}

/// Read Number.longValue without rounding the low bits of an NBT long via f64.
pub(super) fn long(value: &Tag) -> i64 {
    match value {
        Tag::Long(value) => *value,
        value => nbt::number(value).unwrap_or(0.0) as i64,
    }
}

/// v2514 EntityUUIDFix: each family inherits its living/mob/animal rewrites;
/// only horses and tameables have the OwnerUUID string reference.
pub(super) fn entity_uuid(tag: &mut Tag) -> bool {
    let id = nbt::get(tag, "id").and_then(nbt::string).unwrap_or("");
    let (horse, tameable, animal, mob, projectile) = (
        is_horse(id),
        is_tameable(id),
        is_animal(id),
        is_mob(id),
        is_projectile(id),
    );
    let living = id == "minecraft:armor_stand" || mob;
    let hurt = matches!(id, "minecraft:bee" | "minecraft:zombified_piglin");
    let special: u8 = match id {
        "minecraft:fox" => 0,
        "minecraft:item" => 1,
        "minecraft:shulker_bullet" => 2,
        "minecraft:area_effect_cloud" => 3,
        "minecraft:zombie_villager" => 4,
        "minecraft:evoker_fangs" => 5,
        "minecraft:piglin" => 6,
        _ => 7,
    };
    let mut changed = old_player_map::replace_uuid(tag, "UUID", "UUID");
    if living {
        changed |= old_player_map::living_uuid(tag);
    }
    if horse || tameable {
        changed |= replace_uuid_string(tag, "OwnerUUID", "Owner");
    }
    if animal {
        changed |= old_player_map::replace_uuid(tag, "LoveCause", "LoveCause");
    }
    if mob {
        changed |= replace_leash_uuid(tag);
    }
    if projectile {
        changed |= move_uuid_array(tag, "OwnerUUID", "Owner");
    }
    if special == 0 {
        changed |= fox_trusted(tag);
    }
    if special == 1 {
        changed |= replace_uuid_ml(tag, "Owner", "Owner");
        changed |= replace_uuid_ml(tag, "Thrower", "Thrower");
    }
    if special == 2 {
        changed |= replace_uuid_ml(tag, "Owner", "Owner");
        changed |= replace_uuid_ml(tag, "Target", "Target");
    }
    if special == 3 {
        changed |= old_player_map::replace_uuid(tag, "OwnerUUID", "Owner");
    }
    if special == 4 {
        changed |= old_player_map::replace_uuid(tag, "ConversionPlayer", "ConversionPlayer");
    }
    if special == 5 {
        changed |= old_player_map::replace_uuid(tag, "OwnerUUID", "Owner");
    }
    if special == 6 {
        changed |= piglin_angry_at(tag);
    }
    if hurt {
        changed |= replace_uuid_string(tag, "HurtBy", "HurtBy");
    }
    changed
}

/// AbstractUUIDFix.replaceUUIDMLTag: X{M,L} compound -> Target int-array
/// (both halves non-zero or no-op; source key removed on success).
fn replace_uuid_ml(tag: &mut Tag, source: &str, target: &str) -> bool {
    let (most, least) = match nbt::get(tag, source) {
        Some(Tag::Compound(fields)) => {
            let most = fields
                .iter()
                .find(|(k, _)| k == "M")
                .map_or(0, |(_, value)| long(value));
            let least = fields
                .iter()
                .find(|(k, _)| k == "L")
                .map_or(0, |(_, value)| long(value));
            (most, least)
        }
        _ => return false,
    };
    if most == 0 || least == 0 {
        return false;
    }
    nbt::remove(tag, source);
    set_uuid_array(tag, target, most, least);
    true
}

/// AbstractUUIDFix.replaceUUIDString uses Java UUID.fromString, including its
/// short-group fallback and nil UUID (unlike the nonzero-half ML helper).
fn replace_uuid_string(tag: &mut Tag, source: &str, target: &str) -> bool {
    let Some(uuid) = nbt::get(tag, source)
        .and_then(nbt::string)
        .and_then(old_items::parse_uuid)
    else {
        return false;
    };
    nbt::remove(tag, source);
    nbt::set(tag, target, Tag::IntArray(uuid.to_vec()));
    true
}

/// updateProjectile moves any existing OwnerUUID value, overwriting Owner.
fn move_uuid_array(tag: &mut Tag, source: &str, target: &str) -> bool {
    let Some(value) = nbt::take(tag, source) else {
        return false;
    };
    nbt::set(tag, target, value);
    true
}

fn replace_leash_uuid(tag: &mut Tag) -> bool {
    let Some(leash) = nbt::get_mut(tag, "Leash") else {
        return false;
    };
    old_player_map::replace_uuid(leash, "UUID", "UUID")
}

fn fox_trusted(tag: &mut Tag) -> bool {
    let Some(list) = nbt::take(tag, "TrustedUUIDs") else {
        return false;
    };
    let elements = match list {
        Tag::List(list) => list.elements,
        other => {
            nbt::set(tag, "TrustedUUIDs", other);
            return false;
        }
    };
    // Vanilla keeps unconvertible entries (orElseGet original), never drops.
    let mut trusted = Vec::with_capacity(elements.len());
    for entry in elements {
        let pair = match &entry {
            Tag::Compound(fields) => {
                let most = fields
                    .iter()
                    .find(|(k, _)| k == "M")
                    .map_or(0, |(_, value)| long(value));
                let least = fields
                    .iter()
                    .find(|(k, _)| k == "L")
                    .map_or(0, |(_, value)| long(value));
                (most != 0 && least != 0).then(|| {
                    Tag::IntArray(vec![
                        (most >> 32) as i32,
                        most as i32,
                        (least >> 32) as i32,
                        least as i32,
                    ])
                })
            }
            _ => None,
        };
        trusted.push(pair.unwrap_or(entry));
    }
    nbt::set(
        tag,
        "Trusted",
        Tag::List(mc_nbt::ListTag {
            element_type: mc_nbt::tag_type::INT_ARRAY,
            elements: trusted,
        }),
    );
    true
}

fn piglin_angry_at(tag: &mut Tag) -> bool {
    let Some(brain) = nbt::get_mut(tag, "Brain") else {
        return false;
    };
    let Some(memories) = nbt::get_mut(brain, "memories") else {
        return false;
    };
    let Some(angry) = nbt::get_mut(memories, "minecraft:angry_at") else {
        return false;
    };
    let Some(value) = nbt::get_mut(angry, "value") else {
        return false;
    };
    let Some(uuid) = nbt::string(value).and_then(old_items::parse_uuid) else {
        return false;
    };
    *value = Tag::IntArray(uuid.to_vec());
    true
}

pub(super) fn is_horse(id: &str) -> bool {
    matches!(
        id,
        "minecraft:donkey"
            | "minecraft:horse"
            | "minecraft:llama"
            | "minecraft:mule"
            | "minecraft:skeleton_horse"
            | "minecraft:trader_llama"
            | "minecraft:zombie_horse"
    )
}

pub(super) fn is_tameable(id: &str) -> bool {
    matches!(id, "minecraft:cat" | "minecraft:parrot" | "minecraft:wolf")
}

pub(super) fn is_animal(id: &str) -> bool {
    is_horse(id)
        || is_tameable(id)
        || matches!(
            id,
            "minecraft:bee"
                | "minecraft:chicken"
                | "minecraft:cow"
                | "minecraft:fox"
                | "minecraft:mooshroom"
                | "minecraft:ocelot"
                | "minecraft:panda"
                | "minecraft:pig"
                | "minecraft:polar_bear"
                | "minecraft:rabbit"
                | "minecraft:sheep"
                | "minecraft:turtle"
                | "minecraft:hoglin"
        )
}

pub(super) fn is_mob(id: &str) -> bool {
    is_animal(id)
        || matches!(
            id,
            "minecraft:bat"
                | "minecraft:blaze"
                | "minecraft:cave_spider"
                | "minecraft:cod"
                | "minecraft:creeper"
                | "minecraft:dolphin"
                | "minecraft:drowned"
                | "minecraft:elder_guardian"
                | "minecraft:ender_dragon"
                | "minecraft:enderman"
                | "minecraft:endermite"
                | "minecraft:evoker"
                | "minecraft:ghast"
                | "minecraft:giant"
                | "minecraft:guardian"
                | "minecraft:husk"
                | "minecraft:illusioner"
                | "minecraft:magma_cube"
                | "minecraft:pufferfish"
                | "minecraft:zombified_piglin"
                | "minecraft:salmon"
                | "minecraft:shulker"
                | "minecraft:silverfish"
                | "minecraft:skeleton"
                | "minecraft:slime"
                | "minecraft:snow_golem"
                | "minecraft:spider"
                | "minecraft:squid"
                | "minecraft:stray"
                | "minecraft:tropical_fish"
                | "minecraft:vex"
                | "minecraft:villager"
                | "minecraft:iron_golem"
                | "minecraft:vindicator"
                | "minecraft:pillager"
                | "minecraft:wandering_trader"
                | "minecraft:witch"
                | "minecraft:wither"
                | "minecraft:wither_skeleton"
                | "minecraft:zombie"
                | "minecraft:zombie_villager"
                | "minecraft:phantom"
                | "minecraft:ravager"
                | "minecraft:piglin"
        )
}

fn is_projectile(id: &str) -> bool {
    matches!(
        id,
        "minecraft:arrow"
            | "minecraft:dragon_fireball"
            | "minecraft:firework_rocket"
            | "minecraft:fireball"
            | "minecraft:llama_spit"
            | "minecraft:small_fireball"
            | "minecraft:snowball"
            | "minecraft:spectral_arrow"
            | "minecraft:egg"
            | "minecraft:ender_pearl"
            | "minecraft:experience_bottle"
            | "minecraft:potion"
            | "minecraft:trident"
            | "minecraft:wither_skull"
    )
}

/// v2514 BlockEntityUUIDFix: conduit target_uuid{M,L} -> Target int-array
/// (ML-tag helper); skull Owner -> SkullOwner whenever Owner is present
/// (Id string converted when valid, Owner removed in all Owner-present cases).
pub(super) fn block_entity_uuid(tag: &mut Tag) -> bool {
    let id = nbt::get(tag, "id").and_then(nbt::string).unwrap_or("");
    if id == "minecraft:conduit" {
        return replace_uuid_ml(tag, "target_uuid", "Target");
    }
    if id == "minecraft:skull" {
        let Some(mut owner) = nbt::take(tag, "Owner") else {
            return false;
        };
        replace_uuid_string(&mut owner, "Id", "Id");
        nbt::set(tag, "SkullOwner", owner);
        return true;
    }
    false
}

/// v2516 GossipUUIDFix: Gossips[].Target Most/Least -> Target int-array.
pub(super) fn gossip_uuid(tag: &mut Tag) -> bool {
    let Some(gossips) = nbt::get_mut(tag, "Gossips").and_then(nbt::list_mut) else {
        return false;
    };
    let mut changed = false;
    for gossip in gossips.iter_mut() {
        changed |= old_player_map::replace_uuid(gossip, "Target", "Target");
    }
    changed
}
