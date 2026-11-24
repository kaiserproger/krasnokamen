# PERSIST-METADATA-RECAPTURE-BYTE-ORDER: fix identity

`fix_commit = "patch:f03a1064d02a9eb53624b22ec599c74b4517ab1374ce7eb12689f82a1f51d840"`
(`docs/bugs/promotions/PERSIST-METADATA-RECAPTURE-BYTE-ORDER.patch`).

The hunk is `c4d8c0d01`'s `level_metadata::put` change, byte-identical to the
tip: the pre-image removes then appends (moving a refreshed unchanged owned
field to the end, so `DataVersion`/`data` order flips in the saved compound),
the candidate keeps the first slot and drops duplicate keys. Blob pair
`cb69e329b` -> `2953184e2`; no test file changes.

An earlier attempt carried the file's second hunk (the
`sim_debt_anchored_since` initializer) too; reversing it left E0063 because
`weather.rs` keeps the field, so only the `put` hunk is recorded. That
excluded initializer is class (a): it is TIME-FROZEN-CATCHUP-DEBT's
held-anchor behaviour (`ClimateImage::capture` writes
`sim_debt_anchored_since`, weather.rs:88, read back by the same row's
restore), owned by TIME-DEBT-HELD-ANCHOR-PERSISTENCE, and reversing it would
turn that row's assertions red.

Control at head: `native_debt_game_clock_running_nonzero_epoch_capture_load_capture_is_byte_identical`
red by assertion at `crates/mc-core/src/native_game_clock_drill_tests.rs:402`
(`assertion left == right failed: native domain level_metadata`); the
two-output tail ref stays green there (its fixture does not refresh an
unchanged field) and is a green regression. Restore green.
Artifact `20261124T050126-ledger-promote-1_9cka_p`.
