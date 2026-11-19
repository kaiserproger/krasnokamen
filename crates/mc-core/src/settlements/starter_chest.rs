//! Founding grants follow the player's live settlement count across dimensions.
//! The registry owns durable granted counts and a command-phase ownership view.
//! Dimension ticks move this global authority, never clone it.
use std::collections::{BTreeMap, BTreeSet};

use crate::config::CoreError;
use crate::dimensions::Dimension;
use crate::journal::{JournalEntry, Replay};
use crate::tick::TickState;
use mc_world::native::NativeDomainRecord;
use uuid::Uuid;

#[derive(Debug, Default)]
pub(super) struct Grants {
    players: BTreeMap<Uuid, u64>,
    touched: BTreeSet<Uuid>,
    /// Derived before command phases and refreshed after each dimension's turn.
    owned: BTreeMap<Dimension, BTreeMap<Uuid, u64>>,
}

pub(crate) fn granted(state: &TickState, player: Uuid) -> u64 {
    state
        .settlements
        .starter_chests
        .players
        .get(&player)
        .copied()
        .unwrap_or(0)
}

fn local_owned(state: &TickState) -> BTreeMap<Uuid, u64> {
    let mut counts = BTreeMap::new();
    for record in state.settlements.records() {
        if let Some(owner) = record
            .owner
            .as_deref()
            .and_then(|owner| owner.parse::<Uuid>().ok())
        {
            *counts.entry(owner).or_default() += 1;
        }
    }
    counts
}

/// The coordinator still owns every registry here, before taking child worlds.
/// Only command-bearing ticks build this derived view; movement never scans towns.
pub(crate) fn prepare_commands(state: &mut TickState) {
    let mut owned = std::mem::take(&mut state.settlements.starter_chests.owned);
    owned.clear();
    for world in
        std::iter::once(&*state).chain(state.other_dimensions.values().map(|world| &**world))
    {
        owned.insert(world.dimension, local_owned(world));
    }
    state.settlements.starter_chests.owned = owned;
}

pub(crate) fn finish_commands(state: &mut TickState) {
    let owned = local_owned(state);
    state
        .settlements
        .starter_chests
        .owned
        .insert(state.dimension, owned);
}

/// Called after the new settlement has been created successfully.
pub(crate) fn eligible(state: &TickState, player: Uuid) -> bool {
    let local = state
        .settlements
        .records()
        .filter(|record| {
            record
                .owner
                .as_deref()
                .and_then(|owner| owner.parse::<Uuid>().ok())
                == Some(player)
        })
        .count() as u64;
    let elsewhere: u64 = state
        .settlements
        .starter_chests
        .owned
        .iter()
        .filter(|(dimension, _)| **dimension != state.dimension)
        .map(|(_, owners)| owners.get(&player).copied().unwrap_or(0))
        .sum();
    granted(state, player) < local + elsewhere
}

/// Called only after the new empty chest was placed in the same committed tick.
pub(crate) fn record(state: &mut TickState, player: Uuid) {
    let grants = &mut state.settlements.starter_chests;
    *grants.players.entry(player).or_default() += 1;
    grants.touched.insert(player);
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
    for (&player, &count) in &state.settlements.starter_chests.players {
        let payload = serde_json::to_vec(&(player, count))
            .map_err(|error| CoreError::Storage(format!("native starter chest: {error}")))?;
        rows.insert(key(player), crate::native_checkpoint::record(tick, payload));
    }
    Ok(())
}

/// Global grants are drained after all dimension ticks restore global custody.
pub(crate) fn journal_entries(state: &mut TickState, entries: &mut Vec<JournalEntry>) {
    if state.dimension != Dimension::Overworld {
        return;
    }
    for player in std::mem::take(&mut state.settlements.starter_chests.touched) {
        entries.push(JournalEntry::StarterChestGranted {
            player,
            count: state.settlements.starter_chests.players[&player],
        });
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
        let (value, count): (Uuid, u64) = serde_json::from_slice(&row.payload)
            .map_err(|error| invalid(format!("row {row_key}: {error}")))?;
        if value != player || count == 0 {
            return Err(invalid(format!(
                "row {row_key}: invalid player or grant count"
            )));
        }
        grants.players.insert(player, count);
    }
    grants.players.extend(
        replay
            .starter_chest_grants
            .iter()
            .map(|(&player, &count)| (player, count)),
    );
    grants.touched.clear();
    Ok(())
}
