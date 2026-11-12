//! Shard accounting of the per-item step timer and the zero-cost-when-off gate. One test: the
//! counters are process-wide statics, so two tests could consume each other's totals.

use super::{Part, finish, start, take};
use std::time::{Duration, Instant};

#[test]
fn gate_matches_the_environment_and_workers_sum_and_reset_on_take() {
    use rayon::prelude::*;
    assert_eq!(start().is_some(), super::enabled());
    take(Part::Step);
    finish(Part::Step, None);
    assert_eq!(
        take(Part::Step),
        (0, 0),
        "an unstarted region records nothing"
    );

    let started = Instant::now() - Duration::from_micros(250);
    (0..64)
        .into_par_iter()
        .for_each(|_| finish(Part::Step, Some(started)));
    finish(Part::Step, Some(started));
    let (ns, items) = take(Part::Step);
    assert_eq!(items, 65);
    assert!(
        ns >= 65 * 250_000,
        "every item carries its own elapsed time: {ns}"
    );
    assert_eq!(take(Part::Step), (0, 0), "take resets the shards");
}
