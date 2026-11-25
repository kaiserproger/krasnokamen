//! Per-chunk simulation clock and persisted catch-up debt (decision 001-f).
//!
//! `last_sim` advances only during resident active simulation. Inactive time
//! is materialized as `remaining` on activation, excluding the activation's
//! live tick. The tick merge owner spends one bounded Delta for the whole
//! chunk and journals debt minus Delta alongside its effects. No catch-up
//! path requests chunk loading. Accepted non-catch-up block/container writes
//! and item births/chunk crossings cancel old debt in that same WAL batch;
//! no target fingerprints or revisions are needed.
//!
//! Active boundary images project their live anchor to the recovered WAL
//! tick; inactive boundaries freeze. Remaining debt never projects. Every
//! authority change pins the chunk with a storage-only revision until a
//! matching checkpoint ACK. Native extras and checkpoint/WAL images carry
//! the same absolute anchors, including zero images after cancellation.

use crate::config::CoreError;
use mc_world::Chunk;

/// Root-extra key for the authoritative last-simulated tick.
pub(crate) const LAST_SIM_KEY: &str = "KrasnokamenSimLastTick";
/// Root-extra key for outstanding pre-existing catch-up debt.
pub(crate) const REMAINING_KEY: &str = "KrasnokamenSimDebt";
/// Held-tick prefix at `last_sim`, including an explicitly stored zero.
pub(crate) const HELD_ANCHOR_KEY: &str = "KrasnokamenSimHeldTicks";

/// Per-chunk runtime debt authority. Active membership is recomputed by
/// [`crate::world::ChunkWorld::set_active_chunks`]; the WAL image separately
/// persists the open clock interval so recovery can project it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SimDebtRecord {
    /// Live simulation anchor; outstanding abstract time is separate.
    pub last_sim: u64,
    pub held_anchor: Option<u64>,
    /// Dormant catch-up time still owed to this entire chunk.
    pub remaining: u64,
    /// Newer than the last acknowledged checkpoint; pins eviction.
    pub dirty: bool,
    /// Resident+member at the last [`crate::world::ChunkWorld::set_active_chunks`].
    pub active: bool,
}

/// Native NBT longs cannot represent authority above the signed range.
/// Validate before any mutation so a failed capture cannot normalize debt.
pub(crate) fn native_values(
    pos: mc_world::ChunkPos,
    last_sim: u64,
    remaining: u64,
) -> Result<(i64, i64), CoreError> {
    let encode =
        |key, value| {
            i64::try_from(value).map_err(|_| CoreError::Storage(format!(
            "simulation debt at chunk ({}, {}): {key} value {value} exceeds native Long range",
            pos.x, pos.z,
        )))
        };
    Ok((
        encode(LAST_SIM_KEY, last_sim)?,
        encode(REMAINING_KEY, remaining)?,
    ))
}

/// Project authority values into a captured chunk's root extras (durable
/// only; never read back on the tick path, which uses the record map).
pub(crate) fn project(
    chunk: &mut Chunk,
    last_sim: u64,
    remaining: u64,
    held_anchor: Option<u64>,
) -> Result<(), CoreError> {
    let (last_sim, remaining) = native_values(chunk.pos, last_sim, remaining)?;
    let held_anchor = held_anchor.map(i64::try_from).transpose().map_err(|_| {
        CoreError::Storage(format!(
            "simulation debt at chunk ({}, {}): {HELD_ANCHOR_KEY} exceeds native Long range",
            chunk.pos.x, chunk.pos.z,
        ))
    })?;
    chunk
        .extras
        .retain(|(key, _)| key != LAST_SIM_KEY && key != REMAINING_KEY && key != HELD_ANCHOR_KEY);
    chunk
        .extras
        .push((LAST_SIM_KEY.into(), mc_nbt::Tag::Long(last_sim)));
    chunk
        .extras
        .push((REMAINING_KEY.into(), mc_nbt::Tag::Long(remaining)));
    if let Some(held_anchor) = held_anchor {
        chunk
            .extras
            .push((HELD_ANCHOR_KEY.into(), mc_nbt::Tag::Long(held_anchor)));
    }
    chunk.mark_dirty();
    Ok(())
}

/// Validate only the winning authority. A manifest/WAL record supersedes the
/// older native extras, including pre-upgrade anchorless extras.
pub(crate) fn adopt_native_extras(
    pos: mc_world::ChunkPos,
    chunk: &std::sync::Arc<Chunk>,
    records: &mut rustc_hash::FxHashMap<mc_world::ChunkPos, SimDebtRecord>,
    tick: u64,
    held_ticks: u64,
) -> Result<(), CoreError> {
    if records.contains_key(&pos) {
        return Ok(());
    }
    validate_extras(pos, &chunk.extras, records, tick, held_ticks)
}

/// Read one chunk's stored extras. Shared by admission (`adopt_native_extras`,
/// which needs a record map) and the offline import's fail-closed check.
fn validate_extras(
    pos: mc_world::ChunkPos,
    extras: &[(String, mc_nbt::Tag)],
    records: &mut rustc_hash::FxHashMap<mc_world::ChunkPos, SimDebtRecord>,
    tick: u64,
    held_ticks: u64,
) -> Result<(), CoreError> {
    let read = |key: &str| -> Result<Option<u64>, CoreError> {
        let mut values = extras.iter().filter(|(name, _)| name == key);
        let Some((_, tag)) = values.next() else {
            return Ok(None);
        };
        let invalid = |reason| {
            CoreError::Storage(format!(
                "simulation debt at chunk ({}, {}): {key} {reason}",
                pos.x, pos.z,
            ))
        };
        if values.next().is_some() {
            return Err(invalid("is duplicated"));
        }
        match tag {
            mc_nbt::Tag::Long(value) => u64::try_from(*value)
                .map(Some)
                .map_err(|_| invalid("must be nonnegative")),
            _ => Err(invalid("must be a Long")),
        }
    };
    let held_anchor = read(HELD_ANCHOR_KEY)?;
    let (last_sim, remaining) = match (read(LAST_SIM_KEY)?, read(REMAINING_KEY)?) {
        (None, None) if held_anchor.is_none() => return Ok(()),
        (Some(last_sim), Some(remaining)) => (last_sim, remaining),
        _ => {
            return Err(CoreError::Storage(format!(
                "simulation debt at chunk ({}, {}): {LAST_SIM_KEY} and {REMAINING_KEY} must both be present",
                pos.x, pos.z,
            )));
        }
    };
    validate_anchor(pos, last_sim, held_anchor, tick, held_ticks)?;
    records.entry(pos).or_insert(SimDebtRecord {
        last_sim,
        held_anchor: Some(held_anchor.unwrap_or(0)),
        remaining,
        dirty: false,
        active: false,
    });
    Ok(())
}

/// Validate one stored chunk's simulation-debt root extras against the clock a
/// reader will serve at `tick` with `held_ticks` already held.
///
/// An offline Anvil import runs this before the chunk enters custody: an export
/// whose held-time witness cannot serve its own anchors refuses the conversion
/// instead of publishing a store that quarantines the chunk at every admission.
/// Admission re-runs this same check through `adopt_native_extras`; this is the
/// earlier, louder boundary, never a relaxation of it.
pub fn validate_chunk_sim_debt(chunk: &Chunk, tick: u64, held_ticks: u64) -> Result<(), CoreError> {
    let mut records = rustc_hash::FxHashMap::default();
    validate_extras(chunk.pos, &chunk.extras, &mut records, tick, held_ticks)
}

/// A held prefix cannot exceed either its native anchor or the current prefix;
/// the intervening hold cannot be longer than the native interval.
pub(crate) fn validate_anchor(
    pos: mc_world::ChunkPos,
    last_sim: u64,
    held_anchor: Option<u64>,
    tick: u64,
    held_ticks: u64,
) -> Result<(), CoreError> {
    let anchor = held_anchor.unwrap_or(0);
    if last_sim > tick
        || anchor > last_sim
        || anchor > held_ticks
        || held_ticks - anchor > tick - last_sim
        || anchor > i64::MAX as u64
    {
        return Err(CoreError::Storage(format!(
            "simulation debt at chunk ({}, {}): invalid held anchor {anchor} at native {last_sim}; current tick {tick}, held ticks {held_ticks}",
            pos.x, pos.z,
        )));
    }
    Ok(())
}

#[cfg(test)]
#[path = "debt_game_clock_tests.rs"]
mod game_clock_tests;
