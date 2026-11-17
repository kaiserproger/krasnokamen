//! B7 (CS341 REPORT-B.md): the bounded input queue used to drop inputs
//! silently and unmonitored. The counter is surfaced on the dashboard; this
//! module pins the rate limiter that keeps the drop path from logging per
//! refused input on every I/O thread.
//!
//! The limiter measures elapsed monotonic time from its own origin, so these
//! checks drive [`DropWarn::claim_at`] directly: no sleeping, no wall clock,
//! and no clock-step case to get wrong.

use super::{DropWarn, INPUT_DROP_WARN_INTERVAL_MS};

#[test]
fn drop_warning_fires_once_per_interval() {
    let warn = DropWarn::new();
    assert!(warn.claim_at(0), "the first drop warns");
    assert!(
        !warn.claim_at(INPUT_DROP_WARN_INTERVAL_MS - 1),
        "a burst inside the interval stays quiet instead of logging per input"
    );
    assert!(!warn.claim_at(INPUT_DROP_WARN_INTERVAL_MS - 1));
    assert!(
        warn.claim_at(INPUT_DROP_WARN_INTERVAL_MS),
        "the next interval warns exactly once"
    );
    assert!(!warn.claim_at(INPUT_DROP_WARN_INTERVAL_MS));
    assert!(warn.claim_at(2 * INPUT_DROP_WARN_INTERVAL_MS));
}

#[test]
fn drop_warning_claim_takes_the_live_monotonic_clock() {
    // The production entry point: a single claim on a fresh limiter warns, and
    // the next one immediately after it does not, whatever the origin was.
    let warn = DropWarn::new();
    assert!(warn.claim());
    assert!(!warn.claim());
}
