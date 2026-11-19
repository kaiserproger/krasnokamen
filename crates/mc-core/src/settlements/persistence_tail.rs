//! Settlement/transit and starter-grant WAL folds, owned by the domain.
//! No serde layout lives here: the existing record/image codecs stay unchanged.
use super::{RecordImage, Row, SettlementRecord, checkpoint_sequence, row_key};
use crate::config::CoreError;
use crate::dimensions::Dimension;
use crate::journal::Replay;
use crate::native_checkpoint::record;
use mc_world::native::NativeDomainRecord;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Per-dimension absolute post-images since the checkpoint. The host routes
/// envelopes; the owner alone interprets their records, tombstones and floor.
#[derive(Debug, Default)]
pub struct Tail {
    pub(in crate::settlements) settlements: BTreeMap<u64, SettlementRecord>,
    pub(in crate::settlements) removed: BTreeSet<u64>,
    pub(in crate::settlements) high_water: u64,
    pub(in crate::settlements) transits: BTreeMap<uuid::Uuid, Option<Arc<String>>>,
    pub(in crate::settlements) starter_chest_grants: BTreeMap<uuid::Uuid, u64>,
}

impl Tail {
    pub(crate) fn fold_settlement(&mut self, image: &RecordImage, high_water: u64) {
        self.high_water = self.high_water.max(high_water).max(image.id.0);
        match &image.value {
            Some(record) => {
                self.settlements.insert(image.id.0, record.clone());
                self.removed.remove(&image.id.0);
            }
            None => {
                self.settlements.remove(&image.id.0);
                self.removed.insert(image.id.0);
            }
        }
    }

    pub(crate) fn fold_transit(&mut self, id: uuid::Uuid, record: &Option<Arc<String>>) {
        self.transits.insert(id, record.clone());
    }

    pub(crate) fn fold_starter_chest_grant(&mut self, player: uuid::Uuid, count: u64) {
        self.starter_chest_grants.insert(player, count);
    }
}

/// The existing offline settlement fold: keep decode/re-encode (including
/// defaults and field order), home dimensions, tombstones and allocator max.
/// Transit rows keep their existing representation; custody readers separately
/// fold their raw images without hydrating gameplay state.
pub(crate) fn fold_checkpoint(
    globals: &mut BTreeMap<String, NativeDomainRecord>,
    dimensions: &BTreeMap<Dimension, Replay>,
    checkpoint_tick: u64,
    tick: u64,
) -> Result<(), CoreError> {
    let mut sequence = checkpoint_sequence(globals, checkpoint_tick).unwrap_or(0);
    for (&dimension, replay) in dimensions {
        let tail = &replay.settlement_tail;
        sequence = sequence.max(tail.high_water);
        for (&id, settlement) in &tail.settlements {
            sequence = sequence.max(id).max(settlement.id.0);
            globals.insert(
                row_key(settlement.id),
                record(
                    tick,
                    serde_json::to_vec(&Row {
                        dimension,
                        value: settlement,
                    })
                    .map_err(|e| CoreError::Storage(e.to_string()))?,
                ),
            );
        }
        for id in &tail.removed {
            if !tail.settlements.contains_key(id) {
                globals.remove(&format!("settlement/{id}"));
            }
        }
    }
    let root = &dimensions[&Dimension::Overworld];
    for (&player, &count) in &root.settlement_tail.starter_chest_grants {
        let (key, row) = crate::settlements::starter_chest::checkpoint_row(player, count, tick)?;
        globals.insert(key, row);
    }
    globals.insert(
        "settlement_sequence".into(),
        record(
            tick,
            serde_json::to_vec(&sequence).map_err(|e| CoreError::Storage(e.to_string()))?,
        ),
    );
    Ok(())
}

/// Rows deleted by WAL since the last capture. Their removal shares the native
/// checkpoint transaction with the surviving images and the allocator floor.
pub(crate) fn stale_rows(
    globals: &BTreeMap<String, NativeDomainRecord>,
    captured: &BTreeMap<String, NativeDomainRecord>,
) -> Vec<String> {
    globals
        .keys()
        .filter(|key| {
            (key.starts_with("settlement/") || key.starts_with("transit/"))
                && !captured.contains_key(key.as_str())
        })
        .cloned()
        .collect()
}

pub(crate) fn image_heap(image: &RecordImage) -> usize {
    std::mem::size_of::<RecordImage>()
        + image
            .value
            .as_ref()
            .map_or(0, SettlementRecord::heap_estimate)
}

pub(crate) fn transit_image_heap(record: &Option<Arc<String>>) -> usize {
    record
        .as_ref()
        .map_or(0, |value| std::mem::size_of::<String>() + value.capacity())
}
