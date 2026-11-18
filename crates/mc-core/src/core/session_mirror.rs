//! Edge-visible projection of the tick's unlocked sessions.
//!
//! `Core::session_locked` reads this mirror per packet and per command, so the
//! tick publishes it only when its inputs changed: a join, a leave, a
//! dimension move or a plugin-login unlock advances a generation, and an
//! unchanged tick only compares two counters. The projection is built outside
//! the mirror lock and swapped in, so a reader never waits for work
//! proportional to the connection count.

use crate::boundary::ConnectionId;
use crate::tick::TickState;
use std::collections::BTreeSet;
use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};

/// Shared with the edge: a reader tests membership under the read guard, and
/// the published set is replaced whole, never mutated.
#[derive(Debug)]
pub(super) struct SessionMirror {
    unlocked: parking_lot::RwLock<Arc<BTreeSet<ConnectionId>>>,
    /// Published projections. An unchanged tick publishes none. Test-only: it
    /// exists so a regression can count projections instead of timing them.
    #[cfg(test)]
    projections: AtomicU64,
}

impl SessionMirror {
    pub(super) fn new() -> Self {
        Self {
            unlocked: parking_lot::RwLock::new(Arc::default()),
            #[cfg(test)]
            projections: AtomicU64::new(0),
        }
    }

    /// Whether the tick observed this exact session joined and unlocked.
    pub(super) fn contains(&self, connection: ConnectionId) -> bool {
        self.unlocked.read().contains(&connection)
    }

    /// Replace the projection; built by the tick outside this lock. The
    /// replaced `Arc` is freed outside the lock, so a reader holding it never
    /// makes the tick wait for a deallocation of the whole set.
    pub(super) fn publish(&self, unlocked: BTreeSet<ConnectionId>) {
        let replaced = std::mem::replace(&mut *self.unlocked.write(), Arc::new(unlocked));
        drop(replaced);
        #[cfg(test)]
        self.projections.fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(super) fn projections(&self) -> u64 {
        self.projections.load(Ordering::Relaxed)
    }
}

/// The authoritative states that decide the mirror, root first.
fn worlds(state: &TickState) -> impl Iterator<Item = &TickState> {
    std::iter::once(state).chain(state.other_dimensions.values().map(|world| &**world))
}

/// The mirror's inputs: world membership and the plugin-login lock set, over
/// every dimension. Both parts only advance and dimensions sum, so an
/// unchanged stamp means an unchanged projection.
pub(super) fn stamp(state: &TickState) -> (u64, u64) {
    worlds(state).fold((0, 0), |(membership, locks), world| {
        (
            membership.wrapping_add(world.connections.membership_generation()),
            locks.wrapping_add(world.locked_sessions.generation()),
        )
    })
}

/// Every connection joined and unlocked in some dimension (the previous
/// in-place `retain` + `extend` rebuild, as one fresh set).
pub(super) fn unlocked_sessions(state: &TickState) -> BTreeSet<ConnectionId> {
    let mut unlocked = BTreeSet::new();
    for world in worlds(state) {
        unlocked.extend(
            world
                .connections
                .all_connections()
                .filter(|connection| !world.session_locked(*connection)),
        );
    }
    unlocked
}
