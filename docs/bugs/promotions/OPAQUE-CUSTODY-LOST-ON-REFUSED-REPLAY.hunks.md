# OPAQUE-CUSTODY-LOST-ON-REFUSED-REPLAY: patch-mode fix identity

Original fix: none. This row is a guard older than the finding (docs/bugs/README.md, step 1, "Guard older than the finding"): the finding was a
missing regression, and the code that keeps the custody loss out was already there. `fix_commit = "patch:9208d0d2a9339c83d370cfecaa968a1f2c5b67cbee0c8b7f70d0631dc195184d"`
names the guarding hunks at HEAD in `docs/bugs/promotions/OPAQUE-CUSTODY-LOST-ON-REFUSED-REPLAY.patch` (`git apply -R` removes the guard). Re-audit:
`python3 -m tools.harness ledger promote OPAQUE-CUSTODY-LOST-ON-REFUSED-REPLAY --fix-patch docs/bugs/promotions/OPAQUE-CUSTODY-LOST-ON-REFUSED-REPLAY.patch --dry-run`.

Guard: recovery.rs:310-312 at HEAD, `if let Some(error) = tick_state.world.load_errors().next() { return Err(CoreError::Storage(..)) }`
at the end of `load_dimension_state` (one hunk, 3 added lines, nothing else). Introducing commit: the initial import 1ada2ceb
(`git blame -C -C -L 310,312 6cbab9ab -- crates/mc-core/src/core/recovery.rs`). No original hunk is left out, and there is no a/-side adaptation.

Path: replay into a chunk refused at load, then `set_opaque_block_entity` returns early (world.rs:850-852, `!materialize`) before
`mark_block_entity_dirty`. Without the guard the world opens, ticks and checkpoints, the WAL is truncated and the record never
reaches the store.

Proving ref: `core::checkpoint_custody_tests::opaque_replay_into_a_chunk_refused_at_load_fails_recovery_and_keeps_the_wal` goes red with
only the guard removed, at core_checkpoint_custody_tests.rs:1510-1514 "custody lost: the campfire's beef did not reach the store" (left `None`).
It is green with the guard. The custody assertion comes before `refusals.len() == 2` (:1524), so the control turns red on the loss itself,
not on a refusal count (the v1 control's red at :1436 did that and was replaced).
