//! One free founding chest per player, across settlement removal and dimensions.
//! The registry owns the global UUID set; dimension ticks move it, never clone it.
//! Each grant has its own native row and one append-only WAL image.
use std::collections::{BTreeMap, BTreeSet};

use crate::config::CoreError;
use crate::dimensions::Dimension;
use crate::journal::{JournalEntry, Replay};
use crate::tick::TickState;
use mc_world::native::NativeDomainRecord;
use uuid::Uuid;

#[derive(Debug, Default)]
pub(super) struct Grants {
    players: BTreeSet<Uuid>,
    touched: BTreeSet<Uuid>,
}

pub(crate) fn received(state: &TickState, player: Uuid) -> bool {
    state.settlements.starter_chests.players.contains(&player)
}

/// Called only after the new empty chest was placed in the same committed tick.
pub(crate) fn record(state: &mut TickState, player: Uuid) {
    let grants = &mut state.settlements.starter_chests;
    if grants.players.insert(player) {
        grants.touched.insert(player);
    }
}

pub(crate) fn swap_globals(root: &mut TickState, world: &mut TickState) {
    std::mem::swap(
        &mut root.settlements.starter_chests,
        &mut world.settlements.starter_chests,
    );
}

fn key(player: Uuid) -> String {
    format!("settlement_chest/{player}")
}

pub(super) fn checkpoint_rows(
    state: &TickState,
    tick: u64,
    rows: &mut BTreeMap<String, NativeDomainRecord>,
) -> Result<(), CoreError> {
    for player in &state.settlements.starter_chests.players {
        let payload = serde_json::to_vec(player)
            .map_err(|error| CoreError::Storage(format!("native starter chest: {error}")))?;
        rows.insert(
            key(*player),
            crate::native_checkpoint::record(tick, payload),
        );
    }
    Ok(())
}

/// Global grants are drained after all dimension ticks restore global custody.
pub(crate) fn journal_entries(state: &mut TickState, entries: &mut Vec<JournalEntry>) {
    if state.dimension != Dimension::Overworld {
        return;
    }
    for player in std::mem::take(&mut state.settlements.starter_chests.touched) {
        entries.push(JournalEntry::StarterChestGranted { player });
    }
}

pub(super) fn hydrate(
    state: &mut TickState,
    checkpoint: &BTreeMap<String, NativeDomainRecord>,
    replay: &Replay,
) -> Result<(), CoreError> {
    if state.dimension != Dimension::Overworld {
        return Ok(());
    }
    let invalid = |reason: String| CoreError::Storage(format!("native starter chest: {reason}"));
    let grants = &mut state.settlements.starter_chests;
    for (row_key, row) in checkpoint {
        let Some(suffix) = row_key.strip_prefix("settlement_chest/") else {
            continue;
        };
        let player =
            Uuid::parse_str(suffix).map_err(|error| invalid(format!("row {row_key}: {error}")))?;
        if key(player) != *row_key || row.version != 1 {
            return Err(invalid(format!(
                "invalid row header or identity: {row_key}"
            )));
        }
        let value: Uuid = serde_json::from_slice(&row.payload)
            .map_err(|error| invalid(format!("row {row_key}: {error}")))?;
        if value != player {
            return Err(invalid(format!("row {row_key}: player identity mismatch")));
        }
        grants.players.insert(player);
    }
    grants
        .players
        .extend(replay.starter_chest_grants.iter().copied());
    grants.touched.clear();
    Ok(())
}
