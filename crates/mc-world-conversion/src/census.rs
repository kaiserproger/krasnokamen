//! Read-only census of a stopped native world: stored entities, villagers,
//! chunk coverage and settlements, without starting Core or any simulation.
//!
//! Scope (stated in every receipt): the pinned native checkpoint only. The
//! journal is never replayed, so a world with complete journal batches after
//! its checkpoint tick is refused rather than reported with stale counts.
//! Entities stream one record at a time through the native checkpoint reader
//! and pass the same per-entity validation as recovery; settlements come from
//! the persisted `settlement/<id>` rows, never inferred from beds or POIs.
//! Villager membership is the persisted `krasnokamen:VillagerRuntime` column.
use super::{fail, hashes, write_receipt};
use mc_core::dimensions::Dimension;
use mc_core::journal::{CheckpointManifest, Replay};
use mc_core::native_checkpoint::{
    dimension_id, generation_records, hydrate_visited_entity, native_identity, validate_manifest,
};
use mc_core::settlements::households::HouseholdOrigin;
use mc_core::settlements::persistence::{
    checkpoint_records, checkpoint_sequence, transit_census_rows,
};
use mc_core::{CoreConfig, CoreError};
use mc_nbt::Tag;
use mc_world::native::NativeReadOnlyStore;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const SCOPE: &str = "pinned native checkpoint only: entities are the stored table incl. nested \
Passengers, chunks are persisted native chunk positions, settlements and migration transit \
records are persisted core rows; every entity and transit member UUID is checked to occur once; \
no journal replay, no Core load, no tick; refused when journal batches follow the checkpoint";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DimensionCensus {
    pub entity_roots: u64,
    pub entities: u64,
    pub villagers: u64,
    pub chunks: u64,
    pub settlements: u64,
    pub tombstones: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SettlementCensus {
    pub id: u64,
    pub name: String,
    pub dimension: String,
    pub anchor: [i32; 3],
    pub radius: u32,
    /// Households recorded in the persisted settlement row.
    pub households: u64,
    /// Stored entities whose villager column names this settlement.
    pub members: u64,
    /// Members that also carry a household identity.
    pub household_members: u64,
    /// Each persisted household of the row, keyed by id.
    pub household_details: BTreeMap<String, HouseholdCensus>,
}

/// One persisted household with its stored members; adults have `Age >= 0`,
/// read as villager persistence reads it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HouseholdCensus {
    pub origin: HouseholdOrigin,
    pub members: u64,
    pub adults: u64,
}

/// Durable operator evidence; only a fully completed census reports PASS.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CensusReceipt {
    pub status: String,
    pub operation: String,
    pub read_only: bool,
    pub source: PathBuf,
    pub scope: String,
    pub native_schema: u32,
    pub checkpoint_tick: Option<u64>,
    pub counts: BTreeMap<String, u64>,
    pub dimensions: BTreeMap<String, DimensionCensus>,
    pub settlement_details: Vec<SettlementCensus>,
    pub source_unchanged: Option<bool>,
    pub hash_algorithm: String,
    pub input_hashes: BTreeMap<String, String>,
    pub error: Option<String>,
}

/// Count a stopped native world. Writes only the receipt, which must lie
/// outside the source; the source is locked, hashed before and after.
pub fn census_native(
    config: &CoreConfig,
    source: &Path,
    receipt_path: &Path,
) -> Result<CensusReceipt, CoreError> {
    let receipt_parent = receipt_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let receipt_absolute = fs::canonicalize(receipt_parent).map_err(fail)?.join(
        receipt_path
            .file_name()
            .ok_or_else(|| fail("receipt has no filename"))?,
    );
    // The receipt is an output: validate before any refusal can emit one.
    if let Ok(root) = fs::canonicalize(source)
        && receipt_absolute.starts_with(root)
    {
        return Err(fail("receipt must be outside source"));
    }
    let mut receipt = CensusReceipt {
        status: "FAIL".into(),
        operation: "census-native".into(),
        read_only: true,
        source: source.into(),
        scope: SCOPE.into(),
        native_schema: mc_world::native::NATIVE_PROTOTYPE_SCHEMA,
        hash_algorithm: "CRC32".into(),
        ..Default::default()
    };
    let result = census(config, source, &mut receipt);
    if let Err(error) = &result {
        receipt.status = "FAIL".into();
        receipt.error = Some(error.to_string());
    }
    write_receipt(receipt_path, &receipt)?;
    result?;
    Ok(receipt)
}

fn census(
    config: &CoreConfig,
    source: &Path,
    receipt: &mut CensusReceipt,
) -> Result<(), CoreError> {
    let source = fs::canonicalize(source).map_err(fail)?;
    if !source.is_dir() {
        return Err(fail("source must be a directory"));
    }
    let lease = mc_world::storage::open_for_lease(&source).map_err(fail)?;
    lease.try_lock().map_err(fail)?;
    receipt.input_hashes = hashes(&source)?;
    let mut config = config.clone();
    config.world_dir = source.clone();
    let path = source.join("native/state.redb");
    if !path.is_file() {
        return Err(mc_core::native_checkpoint::import_required(&config));
    }
    let registry = mc_core::registry::CoreRegistry::load(config.vanilla_data_dir.as_deref())?;
    let storage = |error: mc_world::native::NativeError| CoreError::Storage(error.to_string());
    let store =
        NativeReadOnlyStore::open_with_cache(&path, native_identity(&config), 16 * 1024 * 1024)
            .map_err(storage)?;
    // Books R4-P1: a census of a store that lost a committed checkpoint would
    // report the older world as truth; refuse before reading it.
    store.verify_marker(&config.world_dir).map_err(storage)?;
    let reader = store.checkpoint_reader().map_err(storage)?;
    let image = reader.read_checkpoint_metadata().map_err(storage)?;
    let manifest: CheckpointManifest = serde_json::from_slice(&image.globals["checkpoint"].payload)
        .map_err(|e| CoreError::Storage(format!("native checkpoint: {e}")))?;
    validate_manifest(&manifest)?;
    let expected = generation_records(&config)?;
    for key in ["generation", "dimensions"] {
        if image.globals[key].payload != expected[key].payload {
            return Err(CoreError::Worldgen(format!(
                "native {key}: configured generation identity mismatch"
            )));
        }
    }
    receipt.checkpoint_tick = Some(manifest.tick);

    let mut dimensions: BTreeMap<Dimension, DimensionCensus> =
        [Dimension::Overworld, Dimension::Nether, Dimension::End]
            .into_iter()
            .map(|dimension| (dimension, DimensionCensus::default()))
            .collect();

    // Authoritative settlement rows, validated as hydration validates them.
    let invalid = |reason: String| CoreError::Storage(format!("native settlement: {reason}"));
    let mut settlements = BTreeMap::new();
    let mut households = BTreeSet::new();
    for row in checkpoint_records(&image.globals) {
        let row = row?;
        let id = row.value.id;
        dimensions
            .get_mut(&row.dimension)
            .expect("closed set")
            .settlements += 1;
        households.extend(row.value.households.keys().cloned());
        settlements.insert(
            id.0,
            SettlementCensus {
                id: id.0,
                name: row.value.name.clone(),
                dimension: row.dimension.name().into(),
                anchor: row.value.anchor,
                radius: row.value.radius,
                households: row.value.households.len() as u64,
                members: 0,
                household_members: 0,
                household_details: row
                    .value
                    .households
                    .iter()
                    .map(|(id, household)| {
                        let detail = HouseholdCensus {
                            origin: household.origin,
                            members: 0,
                            adults: 0,
                        };
                        (id.clone(), detail)
                    })
                    .collect(),
            },
        );
    }
    let sequence = match checkpoint_sequence(&image.globals, manifest.tick) {
        Ok(sequence) => sequence,
        Err(_) if settlements.is_empty() && !image.globals.contains_key("settlement_sequence") => 0,
        Err(error) => return Err(error),
    };
    if let Some(&highest) = settlements.keys().next_back()
        && highest > sequence
    {
        return Err(invalid(format!(
            "settlement {highest} exceeds persisted sequence {sequence}"
        )));
    }
    // Migration transit custody (ADR 0023), validated as recovery validates it.
    let transits = transit_census_rows(&image.globals)?;
    for transit in &transits {
        if let Some(household) = &transit.household
            && households.contains(household)
        {
            return Err(invalid(format!(
                "household {household} is both in transit record {} and a settlement's",
                transit.id
            )));
        }
    }

    // Stream every entity once; the scratch replay holds at most one record.
    let mut scratch = Replay::default();
    let mut wire_ids = BTreeSet::new();
    let mut entity_uuids = BTreeSet::new();
    let mut orphan_members = 0u64;
    let mut members_total = 0u64;
    let mut household_members_total = 0u64;
    reader
        .visit_entities(|entity| {
            let dimension = match entity.dimension {
                0 => Dimension::Overworld,
                1 => Dimension::Nether,
                2 => Dimension::End,
                _ => unreachable!("native reader validates dimension"),
            };
            let census = dimensions.get_mut(&dimension).expect("closed set");
            let Some(payload) = &entity.payload else {
                census.tombstones += 1;
                return Ok(());
            };
            hydrate_visited_entity(
                &registry,
                &config,
                &mut scratch,
                dimension,
                &entity,
                payload,
                manifest.tick,
            )?;
            // hydrate_entity inserts exactly this record; wire ids stay unique.
            let (wire_id, _) = scratch.entities.pop_first().expect("hydrated one entity");
            scratch.entities.clear();
            if !wire_ids.insert(wire_id) {
                return Err(mc_world::native::NativeError::Source(format!(
                    "{}: {}: chunk ({}, {}): duplicate entity identity WireId {wire_id}",
                    dimension.name(),
                    path.display(),
                    entity.position.x,
                    entity.position.z,
                )));
            }
            census.entity_roots += 1;
            let root = uuid::Uuid::from_bytes(entity.identity);
            if !entity_uuids.insert(root) {
                return Err(mc_world::native::NativeError::Source(format!(
                    "{}: duplicate entity UUID {root}",
                    dimension.name()
                )));
            }
            let mut pending = vec![(payload, true)];
            while let Some((tag, is_root)) = pending.pop() {
                let Tag::Compound(fields) = tag else {
                    return Err(mc_world::native::NativeError::Source(format!(
                        "{}: {}: chunk ({}, {}): passenger is not a compound",
                        dimension.name(),
                        path.display(),
                        entity.position.x,
                        entity.position.z,
                    )));
                };
                census.entities += 1;
                if !is_root
                    && let Some(uuid) = nbt_uuid(fields)
                    && !entity_uuids.insert(uuid)
                {
                    return Err(mc_world::native::NativeError::Source(format!(
                        "{}: duplicate entity UUID {uuid}",
                        dimension.name()
                    )));
                }
                if matches!(mc_data::item_nbt::field(fields, "id"),
                    Some(Tag::String(id)) if id == "minecraft:villager")
                {
                    census.villagers += 1;
                }
                if let Some(Tag::Compound(runtime)) =
                    mc_data::item_nbt::field(fields, "krasnokamen:VillagerRuntime")
                    && let Some(Tag::Long(id)) = mc_data::item_nbt::field(runtime, "settlement")
                {
                    members_total += 1;
                    let household = mc_data::item_nbt::field(runtime, "household");
                    let has_household = household.is_some();
                    household_members_total += u64::from(has_household);
                    match settlements.get_mut(&(*id as u64)) {
                        Some(settlement) => {
                            settlement.members += 1;
                            settlement.household_members += u64::from(has_household);
                            if let Some(Tag::String(household)) = household
                                && let Some(detail) =
                                    settlement.household_details.get_mut(household)
                            {
                                detail.members += 1;
                                detail.adults += u64::from(adult(fields));
                            }
                        }
                        None => orphan_members += 1,
                    }
                }
                if let Some(Tag::List(list)) = mc_data::item_nbt::field(fields, "Passengers") {
                    pending.extend(list.elements.iter().map(|tag| (tag, false)));
                }
            }
            Ok(())
        })
        .map_err(storage)?;
    reader
        .visit_chunk_positions(|dimension, _| {
            let dimension = [Dimension::Overworld, Dimension::Nether, Dimension::End]
                .into_iter()
                .find(|candidate| dimension_id(*candidate) == dimension)
                .expect("native reader validates dimension");
            dimensions.get_mut(&dimension).expect("closed set").chunks += 1;
            Ok(())
        })
        .map_err(storage)?;
    // Invariant: every member UUID exactly once across entities and records.
    let mut transit_members = BTreeSet::new();
    for transit in &transits {
        for uuid in &transit.members {
            if entity_uuids.contains(uuid) || !transit_members.insert(*uuid) {
                return Err(invalid(format!(
                    "UUID {uuid} of transit record {} occurs more than once",
                    transit.id
                )));
            }
        }
    }

    let mut tail = 0u64;
    mc_core::journal::visit_complete_batches_after(
        &source.join("journal"),
        manifest.tick,
        mc_core::journal::WalIdentity::new(reader.store_uuid(), &registry.blocks),
        |_| {
            tail += 1;
            Ok(())
        },
    )?;
    if tail != 0 {
        return Err(fail(format!(
            "journal holds {tail} complete batches after checkpoint tick {}; census would not \
             match the reopened world (start and stop the server to fold the journal first)",
            manifest.tick
        )));
    }
    drop(reader);
    drop(store);
    if receipt.input_hashes != hashes(&source)? {
        receipt.source_unchanged = Some(false);
        return Err(fail("source changed during census"));
    }
    receipt.source_unchanged = Some(true);

    let total = |field: fn(&DimensionCensus) -> u64| dimensions.values().map(field).sum::<u64>();
    receipt.counts = BTreeMap::from([
        ("entity_roots".into(), total(|d| d.entity_roots)),
        ("entities".into(), total(|d| d.entities)),
        ("villagers".into(), total(|d| d.villagers)),
        ("chunks".into(), total(|d| d.chunks)),
        ("settlements".into(), settlements.len() as u64),
        ("settlement_members".into(), members_total),
        ("settlement_orphan_members".into(), orphan_members),
        (
            "households".into(),
            settlements.values().map(|s| s.households).sum(),
        ),
        ("household_members".into(), household_members_total),
        ("tombstones".into(), total(|d| d.tombstones)),
        ("settlement_sequence".into(), sequence),
        ("transit_records".into(), transits.len() as u64),
        ("transit_members".into(), transit_members.len() as u64),
    ]);
    receipt.dimensions = dimensions
        .into_iter()
        .map(|(dimension, census)| (dimension.name().into(), census))
        .collect();
    receipt.settlement_details = settlements.into_values().collect();
    receipt.status = "PASS".into();
    Ok(())
}

/// Adult as villager persistence reads it: `Age >= 0`; without `Age`, a
/// baby only when `IsBaby` is set.
fn adult(fields: &[(String, Tag)]) -> bool {
    match mc_data::item_nbt::field(fields, "Age") {
        Some(Tag::Int(age)) => *age >= 0,
        _ => {
            !matches!(mc_data::item_nbt::field(fields, "IsBaby"), Some(Tag::Byte(baby)) if *baby != 0)
        }
    }
}

/// Vanilla `UUID` int-array of one entity compound.
fn nbt_uuid(fields: &[(String, Tag)]) -> Option<uuid::Uuid> {
    let Some(Tag::IntArray(ints)) = mc_data::item_nbt::field(fields, "UUID") else {
        return None;
    };
    let ints: [i32; 4] = ints.as_slice().try_into().ok()?;
    let mut bytes = [0; 16];
    for (chunk, int) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(ints) {
        *chunk = int.to_be_bytes();
    }
    Some(uuid::Uuid::from_bytes(bytes))
}
