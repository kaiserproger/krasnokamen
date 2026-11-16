//! Durable crash-census projection for a disposable copy of a stopped world.
use std::collections::BTreeMap;

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
                "Inventory" | "EnderItems" | "Items" | "HandItems" | "ArmorItems" | "Item" | "equipment"
            )
        })
        .map(|(name, tag)| (name.clone(), crate::command_nbt_path::snbt(tag)))
        .collect()
}

/// Repair redb on the caller's disposable copy, then project the checkpoint and
/// contiguous published WAL without starting the tick owner. The supplied world
/// must be a copy: redb can write its crash-recovery metadata while opening.
pub fn crash_census(config: &CoreConfig) -> Result<CrashCensus, CoreError> {
    let path = config.world_dir.join("native/state.redb");
    let identity = super::native_identity(config);
    drop(
        mc_world::native::NativeStore::open(&path, identity)
            .map_err(|error| super::open_refusal(config, &path, error))?,
    );
    let world = super::recover_offline(config)?;
    let checkpoint_tick = world.manifest.tick;
    let image = super::encode_checkpoint(&world)?;
    let mut entities = BTreeMap::new();
    for entity in image.entities {
        let Some(Tag::Compound(fields)) = entity.payload else {
            continue;
        };
        let kind = fields
            .iter()
            .find_map(|(name, value)| match (name.as_str(), value) {
                ("id", Tag::String(value)) => Some(value.clone()),
                _ => None,
            })
            .ok_or_else(|| CoreError::Storage("crash census: entity missing id".into()))?;
        let holders = holders(&fields);
        let uuid = uuid::Uuid::from_bytes(entity.identity);
        if entities.insert(uuid.to_string(), CrashEntity { kind, holders }).is_some() {
            return Err(CoreError::Storage(format!(
                "crash census: duplicate entity {uuid}"
            )));
        }
    }
    for player in image.players {
        let Tag::Compound(fields) = super::decode_tag(&player.record, "player")? else {
            return Err(CoreError::Storage(
                "crash census: player is not a compound".into(),
            ));
        };
        let uuid = uuid::Uuid::from_bytes(player.uuid);
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
        durable_tick: image.tick,
        entities,
    })
}
