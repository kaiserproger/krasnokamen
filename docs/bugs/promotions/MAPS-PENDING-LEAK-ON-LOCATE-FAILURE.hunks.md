# MAPS-PENDING-LEAK-ON-LOCATE-FAILURE — fix-hunk map

The production-only patch `MAPS-PENDING-LEAK-ON-LOCATE-FAILURE.patch` is one hunk set, SHA-256
`patch:e469b131c4a80a7c1f26daa02370320b07747b15db41fd6dd03de60098d73fc5`. Reverse-applies on the
candidate (`git apply -R --check` on `bb01751d9d`, clean tree).

## The fix and the hunks the patch carries

The behaviour fix is `8499f457a`: the three terminal `LocateCompletion::Callback` paths (metadata
error, wrong dimension, worker loss) call the caller's `on_failure` continuation, which clears the
container's `pending_loot`/`pending_trades`. The patch reverses exactly the three
`if let Some(release) = on_failure { release(state); }` calls and leaves every other line (including
the patterns, which only drop the unused binding) as HEAD's.

## The a/-side against the pre-image

Neither real-commit control builds, which is why the patch exists:

- reversing `8499f457a` removes the `on_failure`/`reply` fields the kept tests use
  (`variant LocateCompletion::Callback has no field named on_failure`, receipt
  `20261124T051238-ledger-promote-ryvf34zi`);
- reversing `94410e6d4` (the test-hook + test commit) removes
  `command_locate::take_pending_callback_release`, which the kept tests call
  (`cannot find function take_pending_callback_release`, receipt
  `20261124T050849-ledger-promote-fblm3xz0`).

The a/-side therefore keeps the enum fields, the test hook and both tests, and differs from HEAD only
in the three calls (plus the doc comment lines next to them, which describe the removed behaviour).

## Why this control is honest

The registered refs are
`cargo:mc-core:command_locate::tests::worker_loss_releases_a_map_loot_callback_so_the_container_opens_again`
and `cargo:mc-core:tick_tests::maps_tests::a_released_locate_plan_lets_the_container_open_again`. With
the release calls gone the plan stays pending, so the first test fails at its own
`worker loss must release the pending loot/trade plan` assertion (receipt below), and the second stays
green (it drives the release hook directly) — the red is the row's symptom, not a counter or a
precondition.

## Observed control and restore

`ledger promote MAPS-PENDING-LEAK-ON-LOCATE-FAILURE --fix-patch …`, receipt
`20261124T051510-ledger-promote-ck54oi3x` on `bb01751d9d` (`tree_dirty: false`, `reversed_files` =
`command_locate.rs`):

- control (`ledger-1-run.log`) **FAIL** — `worker loss must release the pending loot/trade plan` at
  `command_locate_tests.rs:951`;
- restore green (`ledger-3-run.log`).

## What stays out of the row

The `LocateCompletion` fields, the test hook and `maps/tests.rs` are untouched; the fix changes only
which terminal paths release the plan. No wall-clock claim is made.
