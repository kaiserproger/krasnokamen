# WAL-FSYNC-LATENCY-AND-BYTES-UNMEASURED — fix-hunk map

The row's path was closed by two commits; the promotion carries both production changes under one
identity, `patch:d1e5635eea93ad593987adbce48e761a8c20949a99418386ee3bb754f43f4a08`
(`docs/bugs/fix-patches/WAL-FSYNC-LATENCY-AND-BYTES-UNMEASURED.patch`), re-promoted 2026-11-27 on
tree `84fc5c955ca0` (artifact `.analysis/validation/20261127T095712-ledger-promote-qwru2xho/promote.json`).

## The fix

| commit | file | hunk |
|---|---|---|
| `2941bc91f` | `crates/mc-core/src/journal.rs` | `@@ -955,8 +955,13 @@` — `append`: `fsync_start`, `record_fsync`, `WAL_FILE_FSYNCS`, `WAL_APPEND_BYTES`, `WAL_BATCH_BYTES_LAST` |
| `2941bc91f` | `crates/mc-core/src/journal.rs` | `@@ -993,7 +998,10 @@` — `flush`: `fsync_start`, `record_fsync`, `WAL_DIR_FSYNCS` |
| `2941bc91f` + `154914e44` | `tools/coreperf/profiles.py` | `@@ -1669,6 +1669,48 @@` — first added `journal_rollup`; later counted ticks between samples, corrected the mean divisor to file + directory fsyncs, and named the maximum `fsync_ms_max_since_start` |
| `2941bc91f` + `154914e44` | `tools/coreperf/profiles.py` | `@@ -1718,6 +1760,7 @@` — first registered `"journal"` in `summarize`; later passed all tick starts rather than the window's tick count |

## Omitted hunks (reasons from `docs/bugs/README.md` rule 1)

| original hunk | reason |
|---|---|
| `2941bc91` `journal.rs` `@@ -1583,6 +1591,54 @@` (`JournalWriteStats`, `journal_write_stats`, `record_fsync`) | **(b)** the kept cargo refs call the API; kept as the a/-side, so a reversed tree still builds and the counters read zero |
| `2941bc91` `dashboard.rs`, `dashboard/state.rs`, `dashboard/prometheus.rs` | **(b)/(a)** the WAL fields are the kept payload API; `close_causes` belongs to PERF-RECEIPT-HAS-NO-ERROR-COUNTER-ROLLUP |
| `2941bc91` `server/registry.rs`, `server/pump.rs`, `player_scale_bots.rs` | **(a)** other rows' behaviour (error rollup, tick arrival) |
| `154914e4` `bench/remote.py` `thread_listing` | **(a)** TICK-PHASE-LAPS… row's behaviour |

## Control (measured)

Reversing the two production changes reds both proving refs at their own assertions:

- `cargo:mc-core:journal_tests::append_counts_published_bytes_and_fsync_service_time` red at
  `crates/mc-core/src/journal_tests.rs:2463` ("one append fsyncs its batch file").
- `python:tools.coreperf.test_profiles:WindowReceiptFieldsTests.test_receipt_records_wal_fsync_service_time_and_bytes`
  red at `tools/coreperf/test_profiles.py:1395` (`'journal' not found`).

`cargo:mc-edge:dashboard::tests::stats_carry_wal_write_cost_and_close_causes` stays green on the
control (it compares the payload with `journal_write_stats()`, which reads zero when the counting
calls are gone) and is therefore the compatibility check, excluded from `proving_refs`. All three are
green on the restore.
