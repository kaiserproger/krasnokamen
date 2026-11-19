//! Villager and zombie-villager data, trades, gossip and memory.

use super::*;

use super::ids::set_id;
use super::uuids::long;

/// v502 EntityZombieVillagerTypeFix: IsVillager + missing ZombieType ->
/// ZombieType from VillagerProfession (0..5 valid; invalid/missing falls back
/// to a deterministic stand-in since vanilla picks nextInt(6) at random and
/// any of the 6 is legitimate), then IsVillager removed. Deterministic port:
/// UUIDMost rem_euclid 6, 0 when no UUID.
pub(super) fn zombie_villager_type(tag: &mut Tag) -> bool {
    if nbt::get(tag, "IsVillager")
        .and_then(nbt::number)
        .is_none_or(|v| v as i8 == 0)
    {
        return false;
    }
    if nbt::get(tag, "ZombieType").is_some() {
        nbt::remove(tag, "IsVillager");
        return true;
    }
    let profession = nbt::get(tag, "VillagerProfession")
        .and_then(nbt::int)
        .unwrap_or(-1);
    if !(0..6).contains(&profession) {
        // Vanilla picks RandomSource.nextInt(6) at random; any of the 6 is a
        // legitimate vanilla outcome. Deterministic stand-in: stable
        // entity-UUID-derived index (UUIDMost rem_euclid 6; 0 when no UUID).
        let most = nbt::get(tag, "UUIDMost").map_or(0, long);
        nbt::set(tag, "ZombieType", Tag::Int(most.rem_euclid(6) as i32));
        nbt::remove(tag, "IsVillager");
        return true;
    }
    nbt::set(tag, "ZombieType", Tag::Int(profession));
    nbt::remove(tag, "IsVillager");
    true
}

/// v702 EntityZombieSplitFix: Zombie + ZombieType 1..5 -> ZombieVillager with
/// Profession=type-1; 6 -> Husk; else stays Zombie. ZombieType is stripped in
/// ALL arms (vanilla lambda$fix$0 runs on every path).
pub(super) fn zombie_split(tag: &mut Tag) -> bool {
    if nbt::get(tag, "ZombieType").is_none() {
        return false;
    }
    let zombie_type = nbt::get(tag, "ZombieType").and_then(nbt::int).unwrap_or(0);
    nbt::remove(tag, "ZombieType");
    match zombie_type {
        1..=5 => {
            set_id(tag, "ZombieVillager");
            nbt::set(tag, "Profession", Tag::Int(zombie_type - 1));
        }
        6 => {
            set_id(tag, "Husk");
        }
        _ => {}
    }
    true
}

/// v1451 VillagerTradeFix: carved_pumpkin -> pumpkin in trade item ids.
pub(super) fn villager_trade(tag: &mut Tag) -> bool {
    let mut changed = false;
    if let Some(offers) = nbt::get_mut(tag, "Offers") {
        let recipes = nbt::get_mut(offers, "Recipes").and_then(nbt::list_mut);
        if let Some(recipes) = recipes {
            for recipe in recipes.iter_mut() {
                for key in ["buy", "buyB", "sell"] {
                    if let Some(item) = nbt::get_mut(recipe, key)
                        && nbt::get(item, "id").and_then(nbt::string)
                            == Some("minecraft:carved_pumpkin")
                    {
                        nbt::set(item, "id", text("minecraft:pumpkin"));
                        changed = true;
                    }
                }
            }
        }
    }
    changed
}

/// v1904 EntityCatSplitFix (SimpleEntityRenameFix on minecraft:ocelot):
/// - CatType==0 with Owner/OwnerUUID -> stays ocelot, Trusting=true;
/// - 1..=3 -> minecraft:cat (CatType kept, OwnerUUID string round-trip);
/// - else stays ocelot unchanged.
pub(super) fn cat_split(tag: &mut Tag) -> bool {
    let cat_type = nbt::get(tag, "CatType").and_then(nbt::int).unwrap_or(0);
    if cat_type == 0 {
        let owned = nbt::get(tag, "Owner")
            .and_then(nbt::string)
            .is_some_and(|o| !o.is_empty())
            || nbt::get(tag, "OwnerUUID")
                .and_then(nbt::string)
                .is_some_and(|o| !o.is_empty());
        if owned {
            nbt::set(tag, "Trusting", Tag::Byte(1));
            return true;
        }
        return false;
    }
    if (1..4).contains(&cat_type) {
        set_id(tag, "minecraft:cat");
        return true;
    }
    false
}

/// v1918 VillagerDataFix: Profession/Career/CareerLevel ->
/// VillagerData{type: minecraft:plains, profession: upgradeData, level:
/// CareerLevel tag when present else Int(1)}; only the three legacy keys are
/// removed (root `type` remainder, if any, is kept).
pub(super) fn villager_data(tag: &mut Tag) -> bool {
    let profession = nbt::get(tag, "Profession").and_then(nbt::int).unwrap_or(0);
    let career = nbt::get(tag, "Career").and_then(nbt::int).unwrap_or(0);
    let level = nbt::get(tag, "CareerLevel").cloned().unwrap_or(Tag::Int(1));
    nbt::remove(tag, "Profession");
    nbt::remove(tag, "Career");
    nbt::remove(tag, "CareerLevel");
    nbt::set(
        tag,
        "VillagerData",
        nbt::compound(vec![
            ("type", text("minecraft:plains")),
            ("profession", text(upgrade_profession(profession, career))),
            ("level", level),
        ]),
    );
    true
}

fn upgrade_profession(profession: i32, career: i32) -> &'static str {
    match (profession, career) {
        (0, 2) => "minecraft:fisherman",
        (0, 3) => "minecraft:shepherd",
        (0, 4) => "minecraft:fletcher",
        (0, _) => "minecraft:farmer",
        (1, 2) => "minecraft:cartographer",
        (1, _) => "minecraft:librarian",
        (2, _) => "minecraft:cleric",
        (3, 2) => "minecraft:weaponsmith",
        (3, 3) => "minecraft:toolsmith",
        (3, _) => "minecraft:armorer",
        (4, 2) => "minecraft:leatherworker",
        (4, _) => "minecraft:butcher",
        (5, _) => "minecraft:nitwit",
        _ => "minecraft:none",
    }
}

/// v1955 VillagerRebuildLevelAndXpFix: rebuild novice levels from trades,
/// then supply missing/non-numeric Xp for every level.
pub(super) fn villager_rebuild(tag: &mut Tag) -> bool {
    let mut level = nbt::get(tag, "VillagerData")
        .and_then(|data| nbt::get(data, "level"))
        .and_then(nbt::int)
        .unwrap_or(0);
    let mut changed = false;
    if level == 0 || level == 1 {
        let count = nbt::get(tag, "Offers")
            .and_then(|offers| nbt::get(offers, "Recipes"))
            .and_then(nbt::list)
            .map_or(0, |recipes| recipes.len());
        level = (count as i32 / 2).clamp(1, 5);
        if level > 1
            && let Some(data) = nbt::get_mut(tag, "VillagerData")
        {
            nbt::set(data, "level", Tag::Int(level));
            changed = true;
        }
    }
    if nbt::get(tag, "Xp").and_then(nbt::number).is_none() {
        const THRESHOLDS: [i32; 5] = [0, 10, 50, 100, 150];
        let xp = THRESHOLDS[(level - 1).clamp(0, 4) as usize];
        nbt::set(tag, "Xp", Tag::Int(xp));
        changed = true;
    }
    changed
}

/// v1955 ZombieVillagerRebuildXpFix: missing Xp -> thresholds[VillagerData.level].
pub(super) fn zombie_villager_rebuild(tag: &mut Tag) -> bool {
    if nbt::get(tag, "Xp").and_then(nbt::number).is_some() {
        return false;
    }
    let level = nbt::get(tag, "VillagerData")
        .and_then(|data| nbt::get(data, "level"))
        .and_then(nbt::int)
        .unwrap_or(1);
    const THRESHOLDS: [i32; 5] = [0, 10, 50, 100, 150];
    nbt::set(
        tag,
        "Xp",
        Tag::Int(THRESHOLDS[(level - 1).clamp(0, 4) as usize]),
    );
    true
}

/// v1963 RemoveGolemGossipFix: drop Gossips entries with Type=="golem".
pub(super) fn remove_golem_gossip(tag: &mut Tag) -> bool {
    let Some(gossips) = nbt::get_mut(tag, "Gossips").and_then(nbt::list_mut) else {
        return false;
    };
    let before = gossips.len();
    gossips.retain(|entry| nbt::get(entry, "Type").and_then(nbt::string) != Some("golem"));
    before != gossips.len()
}

/// v2505 MemoryExpiryDataFix (minecraft:villager): wrap EVERY
/// Brain.memories value as {value: previous}, unconditionally.
pub(super) fn memory_expiry(tag: &mut Tag) -> bool {
    let Some(brain) = nbt::get_mut(tag, "Brain") else {
        return false;
    };
    let Some(memories) = nbt::get_mut(brain, "memories") else {
        return false;
    };
    let Some(fields) = nbt::fields_mut(memories) else {
        return false;
    };
    for (_, value) in fields.iter_mut() {
        let old = std::mem::replace(value, nbt::compound(vec![]));
        *value = nbt::compound(vec![("value", old)]);
    }
    true
}

/// v2533 VillagerFollowRangeFix: Attributes[] generic.follow_range Base
/// 16.0 -> 48.0.
pub(super) fn villager_follow_range(tag: &mut Tag) -> bool {
    let Some(attributes) = nbt::get_mut(tag, "Attributes").and_then(nbt::list_mut) else {
        return false;
    };
    let mut changed = false;
    for attribute in attributes.iter_mut() {
        if nbt::get(attribute, "Name").and_then(nbt::string) != Some("generic.follow_range") {
            continue;
        }
        if nbt::get(attribute, "Base").and_then(nbt::number) == Some(16.0) {
            nbt::set(attribute, "Base", Tag::Double(48.0));
            changed = true;
        }
    }
    changed
}
