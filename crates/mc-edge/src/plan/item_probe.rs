//! Item-entity fanout attribution under `MC_CORE_PHASE_LOG` (diagnostic only).
//!
//! Counts, per publish, the visits, packets and framed bytes the planner spends
//! on item entities, so a reader can split the viewer-planning cost into
//! "per tracked item per viewer per tick" and "per item packet". Disabled
//! (variable unset) a viewer plan pays one cached boolean test and writes
//! nothing; no counter, clock or log line exists.

use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};

use super::PlannedPacket;
use super::entity_type_flags::is_item;

/// Whether `MC_CORE_PHASE_LOG` was set when first read.
pub(crate) fn enabled() -> bool {
    static ON: LazyLock<bool> = LazyLock::new(|| std::env::var_os("MC_CORE_PHASE_LOG").is_some());
    *ON
}

static VISITS: AtomicU64 = AtomicU64::new(0);
static ALL_VISITS: AtomicU64 = AtomicU64::new(0);
static SPAWN_PACKETS: AtomicU64 = AtomicU64::new(0);
static SPAWN_BYTES: AtomicU64 = AtomicU64::new(0);
static MOVE_PACKETS: AtomicU64 = AtomicU64::new(0);
static MOVE_BYTES: AtomicU64 = AtomicU64::new(0);
static OTHER_PACKETS: AtomicU64 = AtomicU64::new(0);
static OTHER_BYTES: AtomicU64 = AtomicU64::new(0);

/// One viewer plan's item counters; flushed to the shared totals on drop.
#[derive(Default)]
pub(super) struct Tally {
    on: bool,
    visits: u64,
    all_visits: u64,
    spawn: (u64, u64),
    moves: (u64, u64),
    other: (u64, u64),
}

fn framed(packets: &[PlannedPacket]) -> (u64, u64) {
    (
        packets.len() as u64,
        packets.iter().map(|packet| packet.data.len() as u64).sum(),
    )
}

impl Tally {
    pub(super) fn new() -> Self {
        Self {
            on: enabled(),
            ..Self::default()
        }
    }

    /// One visible entity row visited by the per-viewer update loop.
    pub(super) fn visit(&mut self, type_id: u32) -> bool {
        if !self.on {
            return false;
        }
        self.all_visits += 1;
        let item = is_item(type_id);
        self.visits += u64::from(item);
        item
    }

    /// Packets (`AddEntity` plus its metadata) planned to introduce one item.
    pub(super) fn spawned(&mut self, type_id: u32, introduced: &[PlannedPacket]) {
        if self.on && is_item(type_id) {
            let (packets, bytes) = framed(introduced);
            self.spawn = (self.spawn.0 + packets, self.spawn.1 + bytes);
        }
    }

    /// Packets planned for one visited item: its movement packet apart from
    /// the metadata and head packets.
    pub(super) fn updated(&mut self, update: &[PlannedPacket], movement_bytes: Option<usize>) {
        let (packets, bytes) = framed(update);
        if let Some(moved) = movement_bytes {
            self.moves = (self.moves.0 + 1, self.moves.1 + moved as u64);
            self.other = (
                self.other.0 + packets.saturating_sub(1),
                self.other.1 + bytes.saturating_sub(moved as u64),
            );
        } else {
            self.other = (self.other.0 + packets, self.other.1 + bytes);
        }
    }
}

impl Drop for Tally {
    fn drop(&mut self) {
        if !self.on {
            return;
        }
        VISITS.fetch_add(self.visits, Ordering::Relaxed);
        ALL_VISITS.fetch_add(self.all_visits, Ordering::Relaxed);
        SPAWN_PACKETS.fetch_add(self.spawn.0, Ordering::Relaxed);
        SPAWN_BYTES.fetch_add(self.spawn.1, Ordering::Relaxed);
        MOVE_PACKETS.fetch_add(self.moves.0, Ordering::Relaxed);
        MOVE_BYTES.fetch_add(self.moves.1, Ordering::Relaxed);
        OTHER_PACKETS.fetch_add(self.other.0, Ordering::Relaxed);
        OTHER_BYTES.fetch_add(self.other.1, Ordering::Relaxed);
    }
}

/// Item entities among the entities each viewer holds, summed over viewers.
pub(crate) fn tracked_item_pairs<'a>(
    viewers: impl Iterator<Item = &'a super::ViewerState>,
) -> (u64, u64) {
    viewers.fold((0, 0), |(items, all), viewer| {
        (
            items
                + viewer
                    .tracked_entities
                    .values()
                    .filter(|tracked| is_item(tracked.type_id))
                    .count() as u64,
            all + viewer.tracked_entities.len() as u64,
        )
    })
}

/// Takes and resets this publish's `key=value` item counters.
pub(crate) fn take_line() -> String {
    let take = |counter: &AtomicU64| counter.swap(0, Ordering::Relaxed);
    format!(
        "item_visits={} all_visits={} item_spawn_pkts={} item_spawn_bytes={} item_move_pkts={} \
         item_move_bytes={} item_meta_pkts={} item_meta_bytes={}",
        take(&VISITS),
        take(&ALL_VISITS),
        take(&SPAWN_PACKETS),
        take(&SPAWN_BYTES),
        take(&MOVE_PACKETS),
        take(&MOVE_BYTES),
        take(&OTHER_PACKETS),
        take(&OTHER_BYTES),
    )
}
