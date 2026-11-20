//! Pure persisted settlement readers. Custody projections deliberately inspect
//! raw JSON independently of the typed transit hydration/arrival codec.
use super::{Row, parse_row_key, validate_record};
use crate::config::CoreError;
use crate::dimensions::Dimension;
use crate::journal::Replay;
use mc_world::native::NativeDomainRecord;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Validated checkpoint settlement rows, in native-key order. Hydration adds
/// its registry-dependent pending-order check at the same stage as before.
pub fn checkpoint_records(
    checkpoint: &BTreeMap<String, NativeDomainRecord>,
) -> impl Iterator<Item = Result<Row, CoreError>> + '_ {
    checkpoint
        .iter()
        .filter_map(|(key, record)| parse_row_key(key).map(|id| (key, record, id)))
        .map(|(key, record, id)| {
            let invalid =
                |reason: String| CoreError::Storage(format!("native settlement: {reason}"));
            if record.version != 1 {
                return Err(invalid(format!("row {key} has unsupported version")));
            }
            let row: Row = serde_json::from_slice(&record.payload)
                .map_err(|error| invalid(format!("row {key} unreadable: {error}")))?;
            if row.value.id != id {
                return Err(invalid(format!("row {key} identity mismatch")));
            }
            validate_record(&row.value).map_err(|error| invalid(format!("row {key}: {error}")))?;
            Ok(row)
        })
}

fn invalid(reason: impl std::fmt::Display) -> CoreError {
    CoreError::Storage(format!("custody census: {reason}"))
}

/// Raw household identity projection used by the independent custody census.
/// Preserve its checkpoint-then-tail order and refusal context.
pub fn custody_households(
    globals: &BTreeMap<String, NativeDomainRecord>,
    dimensions: &BTreeMap<Dimension, Replay>,
) -> Result<BTreeMap<u64, BTreeSet<String>>, CoreError> {
    let mut households: BTreeMap<u64, BTreeSet<String>> = BTreeMap::new();
    for (key, row) in globals {
        let Some(id) = key.strip_prefix("settlement/") else {
            continue;
        };
        let id: u64 = id
            .parse()
            .map_err(|_| invalid(format!("row {key} has no numeric id")))?;
        let row: Value = serde_json::from_slice(&row.payload)
            .map_err(|error| invalid(format!("row {key} unreadable: {error}")))?;
        let keys = row["value"]["households"]
            .as_object()
            .ok_or_else(|| invalid(format!("row {key} has no households")))?;
        households.insert(id, keys.keys().cloned().collect());
    }
    for replay in dimensions.values() {
        let tail = &replay.settlement_tail;
        for id in &tail.removed {
            households.remove(id);
        }
        for (&id, record) in &tail.settlements {
            households.insert(id, record.households.keys().cloned().collect());
        }
    }
    Ok(households)
}

/// Raw transit images after each dimension's tail; uniqueness across dimensions
/// remains the custody census's responsibility, never hidden by this fold.
pub fn custody_transits(
    globals: &BTreeMap<String, NativeDomainRecord>,
    dimensions: &BTreeMap<Dimension, Replay>,
) -> Result<BTreeMap<(Dimension, uuid::Uuid), Value>, CoreError> {
    let mut records: BTreeMap<(Dimension, uuid::Uuid), Value> = BTreeMap::new();
    for (key, row) in globals {
        let Some(id) = key.strip_prefix("transit/") else {
            continue;
        };
        let id: uuid::Uuid = id
            .parse()
            .map_err(|_| invalid(format!("row {key} has no record UUID")))?;
        let mut row: Value = serde_json::from_slice(&row.payload)
            .map_err(|error| invalid(format!("row {key} unreadable: {error}")))?;
        let dimension: Dimension = serde_json::from_value(row["dimension"].take())
            .map_err(|error| invalid(format!("row {key} dimension: {error}")))?;
        records.insert((dimension, id), row["value"].take());
    }
    for (&dimension, replay) in dimensions {
        for (id, image) in &replay.settlement_tail.transits {
            match image {
                Some(payload) => {
                    let value = serde_json::from_str(payload).map_err(|error| {
                        invalid(format!(
                            "WAL image of transit record {id} unreadable: {error}"
                        ))
                    })?;
                    records.insert((dimension, *id), value);
                }
                None => {
                    records.remove(&(dimension, *id));
                }
            }
        }
    }
    Ok(records)
}
