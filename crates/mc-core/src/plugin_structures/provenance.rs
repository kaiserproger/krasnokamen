//! Provenance lifetime of settlement preparation cells outside the owning
//! village's territory (PrepLifecycle §2.3, owner D27 p.4 and D28).
//!
//! Inside the territory everything a player built is the village's property
//! and block history is never read. Outside it (a founding site, cells beyond
//! the sphere's edge), a manifest surveyed under block-log capture may keep
//! destroying blocks only while the log still retains every attributed edit
//! made since the survey ([`crate::block_log::BlockLog::covers`]). When that
//! coverage lapses, the remaining plan is admitted again under the survey's
//! own history rule: it continues under a fresh admission, or the commit is
//! refused with `Blocked` and nothing is stored. A refusal then follows the
//! ordinary stall path (unpaid work is cancelled and requeued, paid work waits
//! until the blocking player edit leaves the log). No pause is persisted.
use super::*;
use crate::settlements::SettlementId;

/// Block-log evidence a structure's manifest was surveyed under. Required in
/// every ledger record (no serde default), so it fences the record family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Admission {
    /// A plugin structure: its owner's bounds grant is its provenance.
    Plugin,
    /// Surveyed with capture off: attribution is never proven, at any age
    /// (the documented residual).
    Uncaptured,
    /// Surveyed with capture on: every later attributed edit is sealed with a
    /// block-log sequence of at least `since`.
    Captured { since: u64 },
}

impl Admission {
    /// What a settlement manifest surveyed now records.
    pub(super) fn survey(state: &TickState) -> Self {
        state
            .block_log
            .admission_seq()
            .map_or(Self::Uncaptured, |since| Self::Captured { since })
    }
}

/// May the live block at `at` be worked under `admission`? Inside the owning
/// settlement's territory the answer is yes, without reading history.
pub(crate) fn cell_ok(
    state: &TickState,
    settlement: SettlementId,
    admission: Admission,
    at: [i32; 3],
) -> bool {
    inside(state, settlement, at)
        || match admission {
            Admission::Captured { since } => state.block_log.covers(since),
            Admission::Uncaptured | Admission::Plugin => true,
        }
}

/// The commit gate before the debit. It considers only the cells this commit
/// destroys outside the territory. Once their admission no longer covers them,
/// every remaining destructive cell outside the territory must pass the
/// survey's history rule (no retained player edit). The result is the fresh
/// admission to store with this commit's receipt, or `Blocked` before anything
/// is cut or credited.
pub(super) fn admit(
    state: &TickState,
    settlement: SettlementId,
    admission: Admission,
    cells: &[Vec<Cell>],
    [index, offset]: [usize; 2],
    portion: &[Cell],
) -> Result<Option<Admission>, Failure> {
    let mut lapsed = false;
    for cell in portion {
        if destructive(state, cell)? && !cell_ok(state, settlement, admission, cell.at) {
            lapsed = true;
            break;
        }
    }
    if !lapsed {
        return Ok(None);
    }
    let current = cells
        .get(index)
        .map_or(&[][..], |stage| &stage[offset.min(stage.len())..]);
    for cell in current.iter().chain(cells.iter().skip(index + 1).flatten()) {
        if !inside(state, settlement, cell.at)
            && destructive(state, cell)?
            && state.block_log.player_touched(cell.at)
        {
            return Err(Failure::Blocked);
        }
    }
    Ok(Some(Admission::survey(state)))
}

fn inside(state: &TickState, settlement: SettlementId, at: [i32; 3]) -> bool {
    state
        .settlements
        .get(settlement)
        .is_some_and(|record| record.contains(at))
}

/// Working the cell destroys its live block: neither air nor what it places.
fn destructive(state: &TickState, cell: &Cell) -> Result<bool, Failure> {
    let live = state
        .world
        .block_at(cell.at[0], cell.at[1], cell.at[2])
        .ok_or(Failure::Unloaded)?;
    Ok(live != state.registry.states.air && live != cell.state)
}

#[cfg(test)]
#[path = "provenance_tests.rs"]
mod tests;
