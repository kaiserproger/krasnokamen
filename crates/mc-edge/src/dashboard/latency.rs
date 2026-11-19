//! Latency statistics over the bounded tick-interval windows.

use super::*;

use std::time::{SystemTime, UNIX_EPOCH};

/// UNIX wall timestamp, with zero explicitly representing an unavailable clock.
pub(crate) fn unix_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

pub(super) fn push_window(window: &mut VecDeque<u64>, value: u64) {
    if window.len() >= 1200 {
        window.pop_front();
    }
    window.push_back(value);
}

fn sorted_us(intervals: &VecDeque<u64>) -> Vec<u64> {
    let mut v: Vec<u64> = intervals.iter().copied().collect();
    v.sort_unstable();
    v
}

pub(super) fn percentile(sorted: &[u64], q: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (q * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

pub(super) fn summarize(intervals: &VecDeque<u64>, source_tick: u64) -> TickLatency {
    let sorted = sorted_us(intervals);
    TickLatency {
        source_tick,
        p50_us: percentile(&sorted, 0.50),
        p95_us: percentile(&sorted, 0.95),
        p99_us: percentile(&sorted, 0.99),
        max_us: sorted.last().copied().unwrap_or(0),
        samples: sorted.len(),
    }
}

/// Core TPS over the window: tick-number delta / elapsed time, clamped to
/// [0, 20]. Fewer than two arrivals report 0 so a fresh boot never claims
/// full rate.
pub(super) fn tps_of(arrivals: &VecDeque<(u64, Instant)>) -> f64 {
    let (Some(&(first_tick, first_at)), Some(&(last_tick, last_at))) =
        (arrivals.front(), arrivals.back())
    else {
        return 0.0;
    };
    let secs = last_at.saturating_duration_since(first_at).as_secs_f64();
    if secs <= 0.0 {
        return 0.0;
    }
    (last_tick.saturating_sub(first_tick) as f64 / secs).clamp(0.0, 20.0)
}
