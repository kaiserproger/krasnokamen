//! The tick's authoritative plugin-login lock set with a change counter.

use crate::boundary::ConnectionId;
use std::collections::BTreeSet;

/// The tick's authoritative plugin-login lock set, with a change counter.
///
/// The set is private so every mutation advances
/// [`generation`](Self::generation): the tick projects the edge-visible
/// unlocked-session mirror only when a lock changed, instead of rebuilding it
/// under an exclusive lock on every tick.
#[derive(Debug, Default)]
pub(crate) struct SessionLocks {
    locked: BTreeSet<ConnectionId>,
    generation: u64,
}

impl SessionLocks {
    pub(crate) fn insert(&mut self, connection: ConnectionId) -> bool {
        let inserted = self.locked.insert(connection);
        if inserted {
            self.generation += 1;
        }
        inserted
    }

    pub(crate) fn remove(&mut self, connection: &ConnectionId) -> bool {
        let removed = self.locked.remove(connection);
        if removed {
            self.generation += 1;
        }
        removed
    }

    /// The authoritative set, for read-only callers (plugin events, entity
    /// phase) that take it as a plain `BTreeSet`.
    pub(crate) fn as_set(&self) -> &BTreeSet<ConnectionId> {
        &self.locked
    }

    #[must_use]
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }
}

impl std::ops::Deref for SessionLocks {
    type Target = BTreeSet<ConnectionId>;

    fn deref(&self) -> &Self::Target {
        &self.locked
    }
}
