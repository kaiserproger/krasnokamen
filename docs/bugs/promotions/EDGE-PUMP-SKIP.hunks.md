# EDGE pump rows — fix-hunk map

Rows: `EDGE-OUTBOX-STALL-WATCH-HALTS-LIVE-LAGGING-PUMP` (promoted) and
`EDGE-SLOW-PUMP-HOLDS-JOIN-BEHIND-STALE-OUTPUTS` (open, fixed-unproven). Both name the same patch,
`patch:a9098920f0a64040d39be8913ab65bcdcf042080bf53ce17b777702144b113dd`
(`EDGE-SLOW-PUMP-HOLDS-JOIN-BEHIND-STALE-OUTPUTS.patch` /
`EDGE-OUTBOX-STALL-WATCH-HALTS-LIVE-LAGGING-PUMP.patch`, identical content). It reverse-applies on
the candidate.

## The fix

`bfdc23f1c` added the pump's queue budget: `TickTimes::published_at` stamped at broadcast, a `stale()`
predicate (waited past `PUBLISH_QUEUE_BUDGET` = 1.6 s **and** a newer output queued) and a skip arm in
`next_publication` that drops such an output like a broadcast `Lagged`. `523b97aa5` made `stale()` read
the age first and split the receive loop into `next_publication`.

## The one hunk the patch carries (`crates/mc-edge/src/server/pump.rs`)

| hunk | a/-side |
|---|---|
| `next_publication`'s stale arm (`Ok(out) if stale(...)` → skip; `Ok(out) => return`) | removed: `match rx.recv().await { Ok(out) => return Some(out), … }` — the HEAD equivalent of the pre-image inline loop (`523b97aa5` split that loop; reason (c) applies to the split itself) |

`stale()` is **not** in the patch (review N1, 2026-11-24): `bfdc23f1c` created it, so the pre-image
has neither it nor `published_at`. It stays at HEAD as a kept helper whose only remaining callers are
the registered unit tests (`pump_join_lag_tests.rs:593,597,604,605`) — the class "a kept helper whose
callers are all among the carried hunks". On the control it still answers correctly, so those tests
stay green there; they are **not evidence**.

## Classes of the rest (review N2)

| item | class |
|---|---|
| `TickTimes::published_at` field (`boundary.rs`) | **(b)** — the kept tests read/construct it: `explosion_queue_tests.rs:382`, `pump_join_lag_tests.rs:572`, `dashboard/tests.rs:116` |
| the stamp `output.times.published_at = Some(now)` (`core/tick_loop.rs:476`) | kept as **(b)** API-for-tests too: with the arm removed nothing in production reads the field, and the kept tests drive `next_publication` themselves; the write changes no behaviour on either side. Labelled here rather than left unnamed |
| `record_pump_stale_skipped` / `pump_stale_skipped_ticks` (dashboard) | **(b)** — the counter is read by `dashboard/tests.rs:116`; its only production caller is the removed arm |
| the join-linkage debug lines and the test-gate plumbing of `523b97aa5` | **(a)** — other `EDGE-*` rows and harness work |
| `pump_join_lag_tests.rs` | test file, kept whole by the splitter |

## Proving refs

| row | ref | assertion |
|---|---|---|
| EDGE-OUTBOX | `cargo:mc-edge:server::pump_join_lag_tests::a_live_pump_behind_a_lone_batch_does_not_trip_the_stall_watch` | `pump_join_lag_tests.rs:427` — `skipped > 0`, "the release never skipped a stale output": without the arm the live pump never skips and the stall watch halts the core; the test drives the real pump and observes the halt |
| EDGE-SLOW | `harness:join-lag-gate:join_behind_slow_pump` (client) | **no outcoming red exists**: under the shared patch the pre-fix halt happens during setup (waiting for `tick_rate_applied`) before a join is attempted, and `join_lag.py:446-472` sets `join = None`, so the control records no outcome (`20261124T124056`: "no join was attempted before the halt"). The row stays open with that wording |

**Not evidence** (`review B1`): `explosion_queue_tests.rs:417` (a knockback-close precondition, the
mechanism not the symptom), `pump_join_lag_tests.rs:567` (`skipped > 0`, a counter), and the two
"never stale" unit refs, which are green on both sides now that `stale()` is untouched.

## Gate caveat (review N3)

In the control `scenario.json` the witness reports `in_play: false` although it joined
(`player_present: true`, landed 1.95 s): the halt branch clears `in_play` after the join, and the
ledger reads that as "never reached play" and marks the harness ref `unrunnable`. Fixing that needs
`tools/harness/backends/join_lag.py` to keep `in_play` once `player_present` is set; it is recorded,
not fixed here.

## Battle-core trade-off (review N6, already in `main`; unchanged by this delta)

Every skipped output carrying an explosion knockback disconnects its recipients
(`pump.rs:605-642`, `skipped-explosion-knockback`), and the stale skip fires at output age 1.6 s, not
only on a broadcast `Lagged`. At low TPS in a TNT battle that means players see kicks. Recorded as a
trade-off of the fix, not a new defect.
