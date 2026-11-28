# PERF-RECEIPT-HAS-NO-CLIENT-OBSERVED-TICK-ARRIVAL — evidence

- Branch: `wt/D145Harness`, base `2f4d802c08eb6aa9ab18222582c9c39f13f39d55`.
- Fix commit in tree: `2941bc91f88f537c0bf113c43cbac46717ac2b79`
  (`feat(coreperf): receipts carry tick-thread wait, WAL fsync cost, error rollup and tick arrival`).
  `git log -S 'def client_tick_arrival' -- tools/coreperf/profiles.py` names only this commit.
- Regression ref added (was empty):
  `python:tools.coreperf.test_profiles:WindowReceiptFieldsTests.test_client_tick_arrival_reports_each_window_distribution`
  (`tools/coreperf/test_profiles.py:1423`).

## Green ref on HEAD

    python3 -m unittest tools.coreperf.test_profiles.WindowReceiptFieldsTests.test_client_tick_arrival_reports_each_window_distribution

`Ran 1 test ... OK` (`ref-green.log`). It covers `profiles.client_tick_arrival`: each
window's `tick_period_us` distribution is lifted from the bot report, and a
missing window is reported `unavailable` instead of being omitted.

## Control

None constructible, recorded rather than faked: the fix **created**
`client_tick_arrival`, so its pre-image is the function's absence. Reversing the
`2941bc91f` hunk (and it no longer reverse-applies cleanly on HEAD — a later
commit rewrote `profiles.py`, checked with `git apply -R --check`) leaves the ref
calling a missing attribute, i.e. **unrunnable, not red**. README: a row whose
fix created the function has no faithful control and stays open with the reason
recorded.

The bin-level Rust tests in
`crates/mc-test-harness/src/bin/player_scale_bots_tests.rs`
(`tick_period_normalises_a_heartbeat_by_its_game_time_delta`,
`tick_arrival_reports_the_percentiles_of_a_recorded_window`) cannot be cited as
`cargo:` refs because `mc-test-harness` declares several `[[bin]]` targets and
the ref resolver fails closed on an ambiguous target.

## What is now guarded

Even open, the new python ref runs in `run regressions` on every integration, so
the client-observed tick-arrival reporting cannot silently regress while the row
waits for a faithful control route.
