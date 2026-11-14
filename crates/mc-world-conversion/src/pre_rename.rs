//! Normalize only server-owned fields of a pre-Krasnokamen Anvil export.
//!
//! Owner contexts follow Anvil envelopes and native NBT readers, including
//! vanilla bee entity data inside typed item stacks. Opaque compounds and
//! foreign components are never field namespaces. String values stay unchanged.

use super::fail;
use mc_core::CoreError;
use mc_nbt::{ListTag, Tag, tag_type};
use serde::{Deserialize, Serialize};

/// The owner of this compound (or each compound in this list).
#[derive(Clone, Copy)]
pub(crate) enum Owner {
    Level,
    LevelData,
    Chunk,
    EntityRegion,
    Entity,
    Player,
    RootVehicle,
    BlockEntity,
    Bees,
    VillagerRuntime,
    ItemStack,
    ItemComponents,
    ItemContainer,
    Equipment,
    Structures,
    Starts,
    StructureStart,
    Raids,
    RaidsData,
    Raid,
    DragonFight,
    FightData,
    Opaque,
}

impl Owner {
    pub(crate) fn metadata(path: &std::path::Path) -> Self {
        match path.file_name().and_then(|name| name.to_str()) {
            Some("level.dat") => Self::Level,
            Some("raids.dat") => Self::Raids,
            Some("ender_dragon_fight.dat") => Self::DragonFight,
            _ => Self::Opaque,
        }
    }

    fn child(self, key: &str) -> Self {
        use Owner::*;
        match (self, key) {
            (Level, "Data") => LevelData,
            (LevelData, "Player") => Player,
            (LevelData, "DragonFight") | (DragonFight, "data") => FightData,
            (Chunk, "Level") => Chunk,
            (Chunk | EntityRegion, "Entities" | "entities") | (Entity, "Passengers") => Entity,
            (Chunk, "block_entities" | "TileEntities") => BlockEntity,
            (Chunk, "structures" | "Structures") => Structures,
            (Structures, "starts" | "Starts") => Starts,
            (Starts, _) => StructureStart,
            (Player, "ShoulderEntityLeft" | "ShoulderEntityRight") | (RootVehicle, "Entity") => {
                Entity
            }
            (Player, "RootVehicle") => RootVehicle,
            (Entity, "krasnokamen:VillagerRuntime") => VillagerRuntime,
            (Player, "Inventory" | "EnderItems" | "SelectedItem")
            | (Entity, "Inventory" | "Items" | "HandItems" | "ArmorItems")
            | (Entity, "Item" | "item" | "SaddleItem" | "ArmorItem" | "DecorItem")
            | (BlockEntity, "Items" | "RecordItem" | "Book" | "item") => ItemStack,
            (Entity, "equipment") => Equipment,
            (
                Equipment,
                "head" | "chest" | "legs" | "feet" | "mainhand" | "offhand" | "body" | "saddle",
            ) => ItemStack,
            (ItemStack, "components") => ItemComponents,
            (ItemComponents, "minecraft:bees") => Bees,
            (ItemComponents, "minecraft:container") => ItemContainer,
            (ItemContainer, "item")
            | (ItemComponents, "minecraft:bundle_contents" | "minecraft:charged_projectiles") => {
                ItemStack
            }
            (BlockEntity, "Bees" | "bees") => Bees,
            (Bees, "EntityData" | "entity_data") => Entity,
            (Raids, "data") => RaidsData,
            (RaidsData, "Raids" | "raids") => Raid,
            _ => Opaque,
        }
    }
}

/// What the renaming did across one import, for the receipt.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub(crate) struct PreRename {
    pub keys: u64,
    pub values: u64,
    pub first_value: Option<String>,
}

impl PreRename {
    /// `record` lazily supplies source context for errors and the receipt.
    pub(crate) fn rename(
        &mut self,
        tag: &mut Tag,
        owner: Owner,
        record: &dyn Fn() -> String,
    ) -> Result<(), CoreError> {
        self.walk(tag, owner, record, "")
    }

    fn walk(
        &mut self,
        tag: &mut Tag,
        owner: Owner,
        record: &dyn Fn() -> String,
        field: &str,
    ) -> Result<(), CoreError> {
        match tag {
            Tag::Compound(fields) => {
                let mut renamed = false;
                for (key, value) in fields.iter_mut() {
                    if let Some(name) = current_name(owner, key) {
                        if key == "solaris:Rider" {
                            *value = riders(std::mem::replace(value, Tag::Byte(0)), record)?;
                        }
                        *key = name;
                        self.keys += 1;
                        renamed = true;
                    }
                    self.walk(value, owner.child(key), record, key)?;
                }
                if renamed {
                    let mut names: Vec<&str> = fields.iter().map(|(key, _)| key.as_str()).collect();
                    names.sort_unstable();
                    if let Some(pair) = names.windows(2).find(|pair| pair[0] == pair[1]) {
                        return Err(fail(format!(
                            "{}: field {} is present under both its pre-rename and its current name",
                            record(),
                            pair[0]
                        )));
                    }
                }
            }
            Tag::List(list) => {
                for element in &mut list.elements {
                    self.walk(element, owner, record, field)?;
                }
            }
            Tag::String(value) if value.starts_with("solaris:") => {
                self.values += 1;
                if self.first_value.is_none() {
                    self.first_value = Some(format!("{} field {field}", record()));
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// Exact legacy keys, scoped to the compound their native reader owns.
fn current_name(owner: Owner, key: &str) -> Option<String> {
    use Owner::*;
    let name = match (owner, key) {
        (LevelData, "SolarisTerrainEpoch") => "KrasnokamenTerrainEpoch",
        (LevelData, "SolarisEntitySalt") => "KrasnokamenEntitySalt",
        (Chunk, "SolarisJournalLsn") => "KrasnokamenJournalLsn",
        (Chunk, "SolarisGeneratedEntities") => "KrasnokamenGeneratedEntities",
        (Chunk, "SolarisSimLastTick") => "KrasnokamenSimLastTick",
        (Chunk, "SolarisSimDebt") => "KrasnokamenSimDebt",
        (StructureStart, "SolarisLocatePos") => "KrasnokamenLocatePos",
        (Player, "SolarisGameplaySeed") => "KrasnokamenGameplaySeed",
        (Player, "SolarisWardenWarning") => "KrasnokamenWardenWarning",
        (Player, "SolarisStatistics") => "KrasnokamenStatistics",
        (Player, "SolarisAdvancements") => "KrasnokamenAdvancements",
        (Player, "SolarisUnlockedRecipes") => "KrasnokamenUnlockedRecipes",
        (Player, "solaris:RespawnState") => "krasnokamen:RespawnState",
        (Player, "solaris:Respawn") => "krasnokamen:Respawn",
        (Player, "solaris:Tick") => "krasnokamen:Tick",
        (Player, "solaris:BurnTicks") => "krasnokamen:BurnTicks",
        (Player, "solaris:Carried") => "krasnokamen:Carried",
        (Player, "solaris:CraftingGrid") => "krasnokamen:CraftingGrid",
        (Player | Entity, "solaris:PortalTime") => "krasnokamen:PortalTime",
        (Entity, "solaris:WireId") => "krasnokamen:WireId",
        (Entity, "solaris:BreedCooldown") => "krasnokamen:BreedCooldown",
        (Entity, "solaris:Rider") => "krasnokamen:Riders",
        (Entity, "solaris:Riders") => "krasnokamen:Riders",
        (Entity, "solaris:TameRolls") => "krasnokamen:TameRolls",
        (Entity, "solaris:CamelDashCooldown") => "krasnokamen:CamelDashCooldown",
        (Entity, "solaris:AgeLockCooldownUntil") => "krasnokamen:AgeLockCooldownUntil",
        (Entity, "solaris:FrogPregnant") => "krasnokamen:FrogPregnant",
        (Entity, "solaris:AllayDancing") => "krasnokamen:AllayDancing",
        (Entity, "solaris:BeePollinationTicks") => "krasnokamen:BeePollinationTicks",
        (Entity, "solaris:BeeStingTicks") => "krasnokamen:BeeStingTicks",
        (Entity, "solaris:HiveEntity") => "krasnokamen:HiveEntity",
        (Entity, "solaris:entity_loot") => "krasnokamen:entity_loot",
        (Entity, "solaris:Faction") => "krasnokamen:Faction",
        (Entity, "solaris:LifetimeAge") => "krasnokamen:LifetimeAge",
        (Entity, "solaris:ProtectedUntil") => "krasnokamen:ProtectedUntil",
        (Entity, "solaris:NoActionTime") => "krasnokamen:NoActionTime",
        (Entity, "solaris:Presentation") => "krasnokamen:Presentation",
        (Entity, "solaris:MobBlueprint") => "krasnokamen:MobBlueprint",
        (Entity, "solaris:ProjectileRuntime") => "krasnokamen:ProjectileRuntime",
        (Entity, "solaris:SpecialMob") => "krasnokamen:SpecialMob",
        (Entity, "solaris:Sniffer") => "krasnokamen:Sniffer",
        (Entity, "solaris:VillagerRuntime") => "krasnokamen:VillagerRuntime",
        (Entity, "solaris:VehicleTransferCooldown") => "krasnokamen:VehicleTransferCooldown",
        (Entity, "SolarisTouchedWater") => "KrasnokamenTouchedWater",
        (VillagerRuntime, "SolarisMiningProgress") => "KrasnokamenMiningProgress",
        (VillagerRuntime, "SolarisLumberProgress") => "KrasnokamenLumberProgress",
        (VillagerRuntime, "SolarisSmelting") => "KrasnokamenSmelting",
        (VillagerRuntime, "SolarisSoldier") => "KrasnokamenSoldier",
        (BlockEntity, "solaris:CooldownUntil") => "krasnokamen:CooldownUntil",
        (BlockEntity, "solaris:TransferCooldownUntil") => "krasnokamen:TransferCooldownUntil",
        (BlockEntity, "solaris:brewing_ingredient") => "krasnokamen:brewing_ingredient",
        (BlockEntity, "solaris:spawn_deadline") => "krasnokamen:spawn_deadline",
        (BlockEntity, "solaris:ominous_item") => "krasnokamen:ominous_item",
        (BlockEntity, "solaris:phase_until") => "krasnokamen:phase_until",
        (BlockEntity, "solaris:bloom_until") => "krasnokamen:bloom_until",
        (BlockEntity, "solaris:shriek_until") => "krasnokamen:shriek_until",
        (BlockEntity, "solaris:event_arrival") => "krasnokamen:event_arrival",
        (RaidsData, "solaris:clock_tick") => "krasnokamen:clock_tick",
        (RaidsData, "solaris:pending_omens") => "krasnokamen:pending_omens",
        (Raid, "solaris:members") => "krasnokamen:members",
        (FightData, "SolarisFightState") => "KrasnokamenFightState",
        _ => return None,
    };
    Some(name.into())
}

/// RC5's single player rider as the current ordered rider list.
fn riders(rider: Tag, record: &dyn Fn() -> String) -> Result<Tag, CoreError> {
    if !matches!(&rider, Tag::IntArray(uuid) if uuid.len() == 4) {
        return Err(fail(format!(
            "{}: pre-rename solaris:Rider is not a UUID",
            record()
        )));
    }
    Ok(Tag::List(ListTag {
        element_type: tag_type::INT_ARRAY,
        elements: vec![rider],
    }))
}

#[cfg(test)]
#[path = "pre_rename_tests.rs"]
mod tests;
