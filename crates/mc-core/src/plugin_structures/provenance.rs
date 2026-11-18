//! Provenance lifetime of settlement preparation cells outside the owning
//! village's territory (PrepLifecycle §2.3, owner D27 p.4 and D28).
//!
//! Inside the territory everything a player built is the village's property
//! and block history is never read. Outside it (a founding site, cells beyond
//! the sphere's edge) every block a commit destroys must carry no retained
//! player edit, and the manifest's survey must still be covered by the log
//! ([`crate::block_log::BlockLog::covers`]). When that coverage lapses, the
//! remaining plan is admitted again exactly as a fresh survey would admit it:
//! it continues under a fresh admission, or the commit is refused with
//! `Blocked` and nothing is stored. A refusal then follows the ordinary stall
//! path (unpaid work is cancelled and requeued, paid work waits until the
//! blocking player edit leaves the log, and its owner is told the cell once
//! per stall episode). No pause is persisted. Like a fresh survey, a
//! re-admission cannot see a player edit the log already evicted; that
//! residual covers only a block that still has its surveyed identity, since
//! a changed cell is refused while [`covered`] is false.
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

/// Coverage since a structure's survey (`record.admission`) still holds: the
/// log retains every attributed edit made after it. Always true for a survey
/// without capture (the documented residual) and for plugin structures. A
/// changed cell outside the territory is refused while this is false.
pub(super) fn covered(state: &TickState, admission: Admission) -> bool {
    match admission {
        Admission::Captured { since } => state.block_log.covers(since),
        Admission::Uncaptured | Admission::Plugin => true,
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
    inside(state, settlement, at) || covered(state, admission)
}

/// What the commit gate decides for one commit.
enum Verdict {
    Keep,
    Readmit(Admission),
    Refused([i32; 3]),
}

/// The commit gate before the debit, over the cells this commit destroys
/// outside the territory. Each must carry no retained player edit. When their
/// admission no longer covers them, every remaining destructive cell outside
/// the territory must pass the same rule, and the result is the fresh
/// admission to store with this commit's receipt. A re-admission without
/// capture keeps the stored admission: it proves nothing that would demote a
/// captured survey to the residual class. Otherwise `Blocked`, before anything
/// is cut or credited.
pub(super) fn admit(
    state: &TickState,
    settlement: SettlementId,
    admission: Admission,
    cells: &[Vec<Cell>],
    cursor: [usize; 2],
    portion: &[Cell],
) -> Result<Option<Admission>, Failure> {
    match judge(state, settlement, admission, cells, cursor, portion)? {
        Verdict::Keep => Ok(None),
        Verdict::Readmit(admission) => Ok(Some(admission)),
        Verdict::Refused(_) => Err(Failure::Blocked),
    }
}

/// The cell [`admit`] refuses for this commit, if any: the owner's notice
/// names it.
pub(super) fn refused_cell(
    state: &TickState,
    settlement: SettlementId,
    admission: Admission,
    cells: &[Vec<Cell>],
    cursor: [usize; 2],
    portion: &[Cell],
) -> Option<[i32; 3]> {
    match judge(state, settlement, admission, cells, cursor, portion) {
        Ok(Verdict::Refused(at)) => Some(at),
        _ => None,
    }
}

fn judge(
    state: &TickState,
    settlement: SettlementId,
    admission: Admission,
    cells: &[Vec<Cell>],
    [index, offset]: [usize; 2],
    portion: &[Cell],
) -> Result<Verdict, Failure> {
    let mut lapsed = false;
    for cell in portion {
        if !destructive(state, cell)? || inside(state, settlement, cell.at) {
            continue;
        }
        if state.block_log.player_touched(cell.at) {
            return Ok(Verdict::Refused(cell.at));
        }
        lapsed |= !covered(state, admission);
    }
    if !lapsed {
        return Ok(Verdict::Keep);
    }
    let current = cells
        .get(index)
        .map_or(&[][..], |stage| &stage[offset.min(stage.len())..]);
    for cell in current.iter().chain(cells.iter().skip(index + 1).flatten()) {
        if !inside(state, settlement, cell.at)
            && destructive(state, cell)?
            && state.block_log.player_touched(cell.at)
        {
            return Ok(Verdict::Refused(cell.at));
        }
    }
    Ok(state
        .block_log
        .admission_seq()
        .map_or(Verdict::Keep, |since| {
            Verdict::Readmit(Admission::Captured { since })
        }))
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
