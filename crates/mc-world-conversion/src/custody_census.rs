//! Durable custody census of a disposable crash copy (ADR 0023): where every
//! entity UUID and every migration transit member is at the durable boundary,
//! the pinned checkpoint plus its contiguous published WAL tail.
//!
//! Scope: the replay records of offline recovery (`recover_offline`: stored
//! entities hydrated to records, WAL images folded in order with its gap,
//! overlap and checksum refusals). Nothing is restored into a live world and no
//! checkpoint is encoded, so the census never runs `restore_entity`. Transit
//! records and settlement households are read as raw JSON rows and WAL images,
//! never through the transit codec whose custody they check. Opening repairs
//! redb crash metadata in place: the world must be a disposable copy.
use std::collections::{BTreeMap, BTreeSet};

use mc_core::dimensions::Dimension;
use mc_core::native_checkpoint as native;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{CoreConfig, CoreError};

const SCOPE: &str = "disposable crash copy: pinned checkpoint plus contiguous published WAL as \
folded by offline recovery, before any entity is restored; transit records and settlement \
households from raw rows and WAL images; every entity UUID and transit member must occur once";

/// One living villager entity at the durable boundary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VillagerCustody {
    pub dimension: String,
    pub wire_id: i32,
    pub settlement: Option<u64>,
    pub household: Option<String>,
}

/// One migration transit record at the durable boundary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransitCustody {
    pub dimension: String,
    pub source: u64,
    pub destination: u64,
    pub status: String,
    pub departed: u64,
    pub arrives: u64,
    pub household: Option<String>,
    pub members: Vec<String>,
}

/// Durable custody evidence; only a fully checked copy reports PASS.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CustodyReceipt {
    pub status: String,
    pub operation: String,
    pub scope: String,
    pub checkpoint_tick: Option<u64>,
    pub durable_tick: Option<u64>,
    /// Entities of every kind at the durable boundary.
    pub entities: u64,
    /// Villager entities by UUID.
    pub villagers: BTreeMap<String, VillagerCustody>,
    /// Transit records by record id.
    pub transits: BTreeMap<String, TransitCustody>,
    pub error: Option<String>,
}

/// Census of `config.world_dir`, a disposable copy of a stopped or killed
/// world. Never fails: a refusal is a FAIL receipt naming its reason.
pub fn custody_census(config: &CoreConfig) -> CustodyReceipt {
    let mut receipt = CustodyReceipt {
        status: "FAIL".into(),
        operation: "custody-census".into(),
        scope: SCOPE.into(),
        ..Default::default()
    };
    match census(config, &mut receipt) {
        Ok(()) => receipt.status = "PASS".into(),
        Err(error) => receipt.error = Some(error.to_string()),
    }
    receipt
}

fn invalid(reason: impl std::fmt::Display) -> CoreError {
    CoreError::Storage(format!("custody census: {reason}"))
}

fn census(config: &CoreConfig, receipt: &mut CustodyReceipt) -> Result<(), CoreError> {
    let path = config.world_dir.join("native/state.redb");
    drop(
        mc_world::native::NativeStore::open_checked(
            &config.world_dir,
            native::native_identity(config),
        )
        .map_err(|error| native::open_refusal(config, &path, error))?,
    );
    let world = native::recover_offline(config)?;
    receipt.checkpoint_tick = Some(world.manifest.tick);
    receipt.durable_tick = Some(world.dimensions[&Dimension::Overworld].tick);

    // The recovery fold already refuses an entity UUID stored twice.
    let mut bodies = BTreeSet::new();
    for (&dimension, replay) in &world.dimensions {
        for (&wire_id, entity) in &replay.entities {
            let uuid: uuid::Uuid = entity
                .uuid
                .parse()
                .map_err(|_| invalid(format!("entity {wire_id} has UUID {:?}", entity.uuid)))?;
            bodies.insert(uuid);
            if let Some(villager) = entity.villager.as_deref() {
                receipt.villagers.insert(
                    uuid.to_string(),
                    VillagerCustody {
                        dimension: dimension.name().into(),
                        wire_id,
                        settlement: villager.settlement.map(|id| id.0),
                        household: villager.household.clone(),
                    },
                );
            }
        }
    }
    receipt.entities = bodies.len() as u64;

    // Settlement households: checkpoint rows, then each dimension's WAL images.
    let households = mc_core::settlements::persistence::custody_households(
        &world.checkpoint.globals,
        &world.dimensions,
    )?;
    let settled: BTreeSet<&str> = households.values().flatten().map(String::as_str).collect();

    // Transit records per (dimension, id), as recovery hydrates each dimension:
    // its checkpoint rows, then its own WAL images and tombstones. Uniqueness
    // across dimensions is checked afterwards, never masked by the fold.
    let records = mc_core::settlements::persistence::custody_transits(
        &world.checkpoint.globals,
        &world.dimensions,
    )?;
    let mut members: BTreeMap<uuid::Uuid, (Dimension, uuid::Uuid)> = BTreeMap::new();
    let mut travelling: BTreeMap<String, uuid::Uuid> = BTreeMap::new();
    for (&(dimension, id), value) in &records {
        let custody = transit(dimension, value)
            .map_err(|reason| invalid(format!("transit record {id}: {reason}")))?;
        let uuids = custody
            .members
            .iter()
            .map(|member| member.parse::<uuid::Uuid>())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| invalid(format!("transit record {id}: member UUID: {error}")))?;
        for &uuid in &uuids {
            if bodies.contains(&uuid) {
                return Err(invalid(format!(
                    "member {uuid} of transit record {id} is also a world entity"
                )));
            }
            if let Some((other_dimension, other)) = members.insert(uuid, (dimension, id)) {
                return Err(invalid(format!(
                    "member {uuid} is held by transit records {other} ({}) and {id} ({})",
                    other_dimension.name(),
                    dimension.name()
                )));
            }
        }
        if uuids.iter().min() != Some(&id) {
            return Err(invalid(format!(
                "transit record {id} is not keyed by its lowest member"
            )));
        }
        if let Some(household) = custody.household.as_deref() {
            if settled.contains(household) {
                return Err(invalid(format!(
                    "household {household} is in transit record {id} and in a settlement"
                )));
            }
            if let Some(other) = travelling.insert(household.to_owned(), id) {
                return Err(invalid(format!(
                    "household {household} is in transit records {other} and {id}"
                )));
            }
        }
        receipt.transits.insert(id.to_string(), custody);
    }
    Ok(())
}

/// The custody fields of one raw transit record.
fn transit(dimension: Dimension, value: &Value) -> Result<TransitCustody, String> {
    let number = |field: &str| {
        value[field]
            .as_u64()
            .ok_or_else(|| format!("missing {field}"))
    };
    let household = match &value["household"] {
        Value::Null => None,
        family => Some(
            family["id"]
                .as_str()
                .ok_or("household without id")?
                .to_owned(),
        ),
    };
    let members = value["members"]
        .as_array()
        .filter(|members| !members.is_empty())
        .ok_or("no members")?
        .iter()
        .map(|member| {
            member["uuid"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| "member without uuid".to_owned())
        })
        .collect::<Result<_, _>>()?;
    Ok(TransitCustody {
        dimension: dimension.name().into(),
        source: number("source")?,
        destination: number("destination")?,
        status: value["status"].as_str().ok_or("missing status")?.to_owned(),
        departed: number("departed")?,
        arrives: number("arrives")?,
        household,
        members,
    })
}
