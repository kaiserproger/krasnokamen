# EDGE pump rows — fix-hunk map (shared by two rows)

Rows: `EDGE-SLOW-PUMP-HOLDS-JOIN-BEHIND-STALE-OUTPUTS` and
`EDGE-OUTBOX-STALL-WATCH-HALTS-LIVE-LAGGING-PUMP`. Both name the same patch,
`patch:223a8c4f1a290df07e9613f2e3b969ff3683701df7091905583e1bdab2b36aff`
(`EDGE-SLOW-PUMP-HOLDS-JOIN-BEHIND-STALE-OUTPUTS.patch` /
`EDGE-OUTBOX-STALL-WATCH-HALTS-LIVE-LAGGING-PUMP.patch`, identical content). It reverse-applies on
the candidate.

## The fix

`bfdc23f1c` added the pump's queue budget: `TickTimes::published_at` stamped at broadcast, a `stale()`
predicate (waited past `PUBLISH_QUEUE_BUDGET` = 1.6 s **and** a newer output queued) and a skip arm in
`next_publication` that drops such an output like a broadcast `Lagged`. Without it a pump slower than
the tick sits at the oldest retained output, the reliable outbox drains only through that tick, and a
`Joined` waits a whole broadcast window (~10.3 s at 3 TPS), tripping the 10 s join deadline and the
10 s outbox stall watch. `523b97aa5` made `stale()` read the age first (it asks `rx.is_empty()` only
for an old output).

## Hunks the patch carries (`crates/mc-edge/src/server/pump.rs`)

| hunk | a/-side |
|---|---|
| `stale()` | returns `false` — the pre-image had no queue-budget skip, so an output is never stale. Keeping the *function* is the README (b) kept-API constructor: `pump_join_lag_tests.rs:584,593,597,604,605` call it |
| `next_publication`'s stale arm | removed, so the receive loop returns every output (the HEAD equivalent of the pre-image inline loop that `523b97aa5` later split into `next_publication`; reason (c) applies to the split itself) |

Left out, behaviour-neutral in place: `TickTimes::published_at` (boundary.rs) and its stamp
(tick_loop.rs) — with no reader they change nothing, and the kept tests set the field; the dashboard's
`record_pump_stale_skipped`/`pump_stale_skipped_ticks` counter, whose only caller is the removed arm;
the join-linkage debug lines and the test-gate plumbing of the same commits (other `EDGE-*` rows and
harness work, reason (a)).

## Proving refs

| row | registered refs |
|---|---|
| EDGE-SLOW-PUMP | `cargo:…:a_slow_pump_does_not_hold_a_join_behind_stale_outputs`, `…:a_join_lands_at_three_tps_behind_a_slow_pump_without_a_halt`, `…:a_stale_skip_keeps_reliable_changes_in_order_and_never_drains_ahead`, `…:the_queue_budget_is_exclusive`, and `harness:join-lag-gate:join_behind_slow_pump` |
| EDGE-OUTBOX-STALL-WATCH | `cargo:…:a_live_pump_behind_a_lone_batch_does_not_trip_the_stall_watch`, `…:a_hung_pump_still_halts_the_core`, and `harness:join-lag-gate:live_pump_does_not_halt` |

`the_newest_output_is_never_stale` and `a_fresh_output_after_a_long_tick_is_not_stale` are green on
both trees (they assert "never stale" cases) and are not evidence.

## What is still missing

The client-level receipt. `harness:join-lag-gate:*` runs two native windowed clients against a
`KRASNOKAMEN_SERVER_BINARY` pin of the candidate and needs a **second pin of the reversed tree** for
the control (`docs/development/real-client-gates.md`, `realtime-gates` section: "negative control: run
the same command on a control tree that differs only in the fix hunks"). Neither pin exists for this
head yet; the cargo refs above are green on the candidate (probe `20261124T092509-ledger-probe-cadi2cru`).
