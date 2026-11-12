//! Per-item cost attribution under `MC_CORE_PHASE_LOG` (diagnostic only).
//!
//! Leaf module by design: it depends on nothing in `crate`, so the callers that already sit in the
//! tick's module cycle do not pull it in (the code-health cycle ratchet).
//!
//! Item entities pass through every entity pipeline stage mixed with mobs, so the
//! stage laps alone cannot say what an item costs. This module adds item-only
//! counters at the stages that own item work: the step (parallel CPU time), the
//! merge scan, expiry removal and the wire-view publication, plus a per-tick
//! census of the resident item population. Each stage prints its own
//! `items ...` line to stderr just before the dimension's `phase ms:` line, so a
//! reader attaches every `items` line to the next `phase ms:` line.
//!
//! Disabled (variable unset) the cost is one cached boolean test per call site:
//! no clock, no counter write, no allocation, no log line.

use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Whether `MC_CORE_PHASE_LOG` was set when the first call ran.
pub(crate) fn enabled() -> bool {
    static ON: LazyLock<bool> = LazyLock::new(|| std::env::var_os("MC_CORE_PHASE_LOG").is_some());
    *ON
}

/// Stages with a parallel or scattered timer; each accumulates CPU nanoseconds
/// and the number of items it covered.
#[derive(Clone, Copy)]
pub(crate) enum Part {
    /// `item_motion::step`, run on the rayon workers (summed CPU, not wall).
    Step,
}

const PARTS: usize = 1;
const SHARDS: usize = 64;

/// One cache line per shard so rayon workers never share a counter line.
#[repr(align(64))]
struct Shard {
    ns: [AtomicU64; PARTS],
    count: [AtomicU64; PARTS],
}

#[allow(clippy::declare_interior_mutable_const)]
const SHARD: Shard = Shard {
    ns: [const { AtomicU64::new(0) }; PARTS],
    count: [const { AtomicU64::new(0) }; PARTS],
};
static SHARDS_TABLE: [Shard; SHARDS] = [SHARD; SHARDS];

/// Clock start for one measured region; `None` when the log is off.
pub(crate) fn start() -> Option<Instant> {
    enabled().then(Instant::now)
}

/// Adds the region since `started` to `part` (one item).
pub(crate) fn finish(part: Part, started: Option<Instant>) {
    let Some(started) = started else { return };
    let ns = started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
    let shard = &SHARDS_TABLE[rayon::current_thread_index().unwrap_or(SHARDS - 1) % SHARDS];
    shard.ns[part as usize].fetch_add(ns, Ordering::Relaxed);
    shard.count[part as usize].fetch_add(1, Ordering::Relaxed);
}

/// Takes and resets the `(cpu_ns, items)` total of `part`.
fn take(part: Part) -> (u64, u64) {
    SHARDS_TABLE.iter().fold((0, 0), |(ns, count), shard| {
        (
            ns + shard.ns[part as usize].swap(0, Ordering::Relaxed),
            count + shard.count[part as usize].swap(0, Ordering::Relaxed),
        )
    })
}

/// Cost of one `start`/`finish` pair in nanoseconds: every per-item timer
/// carries it, so a reader subtracts `items * clock_ns` from the CPU total.
fn clock_ns() -> u64 {
    static NS: LazyLock<u64> = LazyLock::new(|| {
        const PAIRS: u32 = 20_000;
        let begin = Instant::now();
        let mut sink = 0u64;
        for _ in 0..PAIRS {
            let started = Instant::now();
            sink = sink.wrapping_add(started.elapsed().as_nanos() as u64);
        }
        std::hint::black_box(sink);
        (begin.elapsed().as_nanos() / u128::from(PAIRS)) as u64
    });
    *NS
}

/// Resident item population of one dimension tick.
#[derive(Default)]
pub(crate) struct Census {
    pub(crate) items: u64,
    pub(crate) alive: u64,
    pub(crate) ground: u64,
    pub(crate) moving: u64,
    pub(crate) mature: u64,
    pub(crate) stack_count: u64,
}

/// Prints the census with the step CPU of the tick that just ran. Called once per
/// simulated dimension tick from the merge pass, after the step and commit stages
/// and before the views and phase lines.
pub(crate) fn print_census(tick: u64, dimension: impl std::fmt::Debug, census: &Census) {
    let (step_ns, step_items) = take(Part::Step);
    eprintln!(
        "items census: tick={tick} dimension={dimension:?} items={} alive={} ground={} moving={} \
         mature={} stack_count={} step_items={step_items} step_cpu_ms={:.3} clock_ns={}",
        census.items,
        census.alive,
        census.ground,
        census.moving,
        census.mature,
        census.stack_count,
        step_ns as f64 / 1e6,
        clock_ns(),
    );
}

/// The merge pass's own split: cell gather and sort, the neighbour scan, and what
/// the scan found. Printed only on the merge cadence tick.
pub(crate) struct MergeScan {
    pub(crate) gather_sort: std::time::Duration,
    pub(crate) scan: std::time::Duration,
    pub(crate) examined: usize,
    pub(crate) candidates: u64,
    pub(crate) overlapping: u64,
    pub(crate) merged: u64,
}

pub(crate) fn print_merge(tick: u64, dimension: impl std::fmt::Debug, scan: &MergeScan) {
    eprintln!(
        "items merge: tick={tick} dimension={dimension:?} examined={} gather_sort_ms={:.3} \
         scan_ms={:.3} candidates={} overlapping={} merged={}",
        scan.examined,
        scan.gather_sort.as_secs_f64() * 1000.0,
        scan.scan.as_secs_f64() * 1000.0,
        scan.candidates,
        scan.overlapping,
        scan.merged,
    );
}

/// Expiry removal (age pass or despawn pass): `items` of the `removed` rows were item entities.
pub(crate) fn print_expiry(
    tick: u64,
    dimension: impl std::fmt::Debug,
    pass: &str,
    items: usize,
    removed: usize,
    took: std::time::Duration,
) {
    eprintln!(
        "items expiry: tick={tick} dimension={dimension:?} pass={pass} items={items} \
         removed={removed} remove_ms={:.3}",
        took.as_secs_f64() * 1000.0,
    );
}

/// Wire-view publication candidates: how many were queued, how many were items,
/// and how many images actually changed.
pub(crate) fn print_views(
    pending: usize,
    items_pending: usize,
    changed: usize,
    items_changed: usize,
) {
    eprintln!(
        "items views: pending={pending} items_pending={items_pending} changed={changed} \
         items_changed={items_changed}"
    );
}

#[cfg(test)]
#[path = "item_perf_tests.rs"]
mod tests;
