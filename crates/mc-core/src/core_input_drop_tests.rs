//! B7 (CS341 REPORT-B.md): the bounded input queue used to drop inputs
//! silently and unmonitored. The counter is surfaced on the dashboard; this
//! module pins the rate limiter that keeps the drop path from logging per
//! refused input on every I/O thread.

use super::{DropWarn, INPUT_DROP_WARN_INTERVAL_MS};

#[test]
fn drop_warning_fires_once_per_interval() {
    let warn = DropWarn::default();
    assert!(warn.claim(1_000), "the first drop warns");
    assert!(
        !warn.claim(1_000 + INPUT_DROP_WARN_INTERVAL_MS - 1),
        "a burst inside the interval stays quiet instead of logging per input"
    );
    assert!(!warn.claim(1_000 + INPUT_DROP_WARN_INTERVAL_MS - 1));
    assert!(
        warn.claim(1_000 + INPUT_DROP_WARN_INTERVAL_MS),
        "the next interval warns exactly once"
    );
    assert!(!warn.claim(1_000 + INPUT_DROP_WARN_INTERVAL_MS));
}

#[test]
fn drop_warning_stays_bounded_across_clock_steps_and_an_absent_clock() {
    let warn = DropWarn::default();
    assert!(warn.claim(1_000_000), "warns");
    assert!(
        !warn.claim(5_000),
        "a backwards wall-clock step must not re-arm the warning"
    );
    let absent = DropWarn::default();
    assert!(absent.claim(0), "a zero clock still warns once");
    assert!(
        !absent.claim(0),
        "and cannot storm while the clock reads zero"
    );
}
