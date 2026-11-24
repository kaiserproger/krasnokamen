# DIVERGED-BACKUP-COPIES-SHARE-STORE-IDENTITY — negative-control patch

Fix: `docs/bugs/fix-patches/DIVERGED-BACKUP-COPIES-SHARE-STORE-IDENTITY.patch`
(`fix_commit = "patch:6a42e81b3d1527dbbcb8af256657ab163e273b3725bca752c78291907ffe2eef"`, sha256 of its
non-test hunks, here the whole file). It is a diff from the pre-fix form to the candidate; `git apply -R`
restores the pre-fix form. The row was promoted on `01ac685ba7` alone, then on `01ac685ba7` + `cee142f323`
(patch `0ec63ae4…`), then with `f0d7f039d` (patch `c1165a63…`), then with `5ae472035` (patch `5f4184a9…`).
Four persistence reviews (`receipts/D123PersistDiv/review/REVIEW-ReviewD123Diverged-Sol1.md`, `-Sol2.md`,
`REVIEW-sol-2.md`, `REVIEW-sol-3.md`) found paths to the row's symptom that those commits left open, so the
row was reopened each time with the completed fix.

## Commits covered

- `01ac685ba7` "persist: give every restored backup its own branch" — `run_restore` mints a branch that is
  neither the root nor the backup's own, moves the copied tail above the checkpoint onto it
  (`journal::rebind_tail`) and writes the marker naming it. Closes the row's main path: two copies of one
  backup, or a copy and its source, refuse each other's tails.
- `cee142f323` "persist: publish a restored world from an unpublished staging directory" — the same steps
  run in `<target>.restore-staging` (the backup's marker is verified from memory and never copied; the
  copy's own marker is written last), every directory is fsynced and one rename publishes the finished copy;
  every restored copy has `journal/` and replay refuses a restored copy without one; an offline backup keeps
  an empty `journal/`. Closes the interrupted-restore path: before it, a restore killed between its copies
  and its marker write left the backup's store, marker and tail on the backup's branch at the target, and a
  lost `journal/` read as an empty tail.
- `f0d7f039d` "persist: refuse restore staging by name, keep the target lease through cleanup, refuse lost
  journals" — closes the three paths of the second review: (1) the staged copy, once its own marker is
  written, opened as a world before publication (`store_marker::refuse_unpublished` in `verify`/`check`,
  `Core::open`, `NativeStore::open_checked`/`check_marker_file`, `run_backup`; restore and conversion refuse
  a staging-named target); (2) a failed publication's cleanup could recursively delete a target another
  process had taken (the target lease is kept through publication and cleanup, which removes only the
  restore's own staging directory and a target it created with `remove_dir`; the Windows
  release-remove-rename publication is gone and restore refuses on non-Unix hosts); (3) backup and restore
  without a content cache turned a restored copy's lost `journal/` into a fresh empty one (both refuse it).
- `5ae472035` "persist: keep journal/ through the game-clock upgrade; own only a target restore created" —
  closes the third review's path: `world upgrade-game-clock` renamed a restored copy's whole `journal/` away,
  leaving a copy that every open, backup and restore refuses (the `journal/` rule of `cee142f323`); it now
  moves the batch files and keeps the directory. Also: `created_target` comes from restore's own exclusive
  mkdir, and census, probe and the upgrade refuse a staging directory before creating a lease file.
- `fb408c908` "persist: pin the converted game-clock checkpoint in the marker before archiving the WAL" —
  closes the fourth review's path: the upgrade archived the WAL while the marker still named the old
  checkpoint `K`, so a store rolled back from the converted tick `C` to `K` would pass the marker check and
  open without its tail. The marker is now written at `C` (same store id and branch) before the first move; a
  failed write leaves the WAL in place.

## Carried hunks

`crates/mc-edge/src/backup.rs`, a/-side = the base blob `d3f02c0381a1` (`9b6ddef165`) plus the two
`#[cfg(test)]` copy-hook lines of `742550fb0f` in `copy_reader_durable` (not a fix commit; context on both
sides, so no a/-side text outside the original pre-image):

- module doc (`cee142f323`): "builds the world in a staging directory … one rename";
- `run_backup` (`cee142f323`, `f0d7f039d`): staging-name refusal, the non-root `journal/` refusal, keeps an
  empty `journal/`;
- `run_restore` (`01ac685ba7` call of `start_branch`, rewritten by `cee142f323`, `f0d7f039d` and
  `5ae472035`): a/ = the base in-place copy, pair check against the copied marker and cleanup; b/ = non-Unix
  refusal, staging-name target refusal, staging path, mount-point refusal, leftover-staging refusal, target
  creation through `create_target`, `stage_world`, `publish` under the kept target lease, ownership-safe
  cleanup, post-rename parent fsync and the `Publishing`/`Published` interruption points;
- `staging_path`, `create_target` (`5ae472035`), `refuse_mount_point`, `stage_world` (incl. the non-root
  `journal/` refusal of `f0d7f039d`), `start_branch` (`01ac685ba7`, rewritten to take the backup's marker
  from memory and run in the staging directory), `sync_tree`, `publish` (a/: absent).

`crates/mc-core/src/journal.rs`, a/-side = the blob at `742550fb0f` (no a/-side adaptation):

- `visit_complete_batches_after`: a missing journal directory is an empty tail only on the root branch
  (`cee142f323`; a/: always an empty tail) and its doc;
- the `rebind_tail` doc sentence on where restore runs it (`cee142f323`).

`f0d7f039d`, a/-side = each file at its parent `6815b1b153` (no a/-side adaptation):

- `crates/mc-core/src/core.rs` `Core::open`: the shared refusal (a/: the closure that knew only
  `.krasnokamen-conversion-*`);
- `crates/mc-world/src/native.rs` `open_checked`, `check_marker_file`: refuse a staging directory before the
  store is opened or repaired;
- `crates/mc-world/src/store_marker.rs` `verify` (and so `check`): the `refuse_unpublished` call and its doc
  sentence;
- `crates/mc-world-conversion/src/lib.rs` `convert`: the destination refusal through `refuse_unpublished`
  and the stage name from `CONVERSION_STAGING_PREFIX` (a/: the literal prefix check).

`5ae472035` and `fb408c908`, a/-side = each file at `c1f366056` (no a/-side adaptation):

- `crates/mc-world-tools/src/game_clock_upgrade.rs`: the early staging refusal, the marker write at the
  converted tick (with its `#[cfg(test)]` fault call) and `archive_covered_journal` (a/: no marker write,
  then a rename of the whole `journal/` directory);
- `crates/mc-world-conversion/src/census.rs`, `probe.rs`: the early staging refusal before the lease.

## Not carried, with the ground (docs/bugs/README.md classes)

- (b) API the kept tests call: `rebind_tail`'s `before_batch` hook and `StoreMarker::admits`
  (`742550fb0f`, not a fix commit), the `backup_tests.rs` interruption machinery (`Step`,
  `interruption_point`, `observed_restore`), and from `f0d7f039d` `store_marker::RESTORE_STAGING_SUFFIX`,
  `CONVERSION_STAGING_PREFIX`, `MarkerError::Unpublished` and the `refuse_unpublished` function itself
  (`store_marker_tests` names them); every call site of that function is carried.
- Not a fix commit: `be1ab01c71` (64-bit `mint_branch`, docs) and the format-3 frame of `49d3f0d53e`
  (infrastructure the refs need to compile and the base restore never touches).
- Docs only: persistence.md, ADR 0013, OPERATING.md, MIGRATION.md.

## Proving assertions (red in the control)

- `backup::tests::a_restored_copy_refuses_the_journal_of_another_copy_of_its_backup`, the refusal assertion
  (`backup_tests.rs:762`, "the left copy replayed the right copy's journal: None"): with no branch minted
  the sibling tail replays.
- `backup::tests::a_restore_interrupted_before_its_first_rebind_leaves_nothing_that_opens`,
  `assert_no_world_at` (`backup_tests.rs:865`, "an interrupted restore left [native, journal,
  checkpoint.marker] at the target, marker … branch [0; 8]"): the base restore killed after its last copy
  leaves the backup's store, marker and tail on the backup's branch in the target.
- `journal_tests::a_missing_journal_is_an_empty_tail_only_on_the_root_branch`, its `expect_err`
  (`journal_tests.rs:2445`, "a restored copy without its journal lost its tail: 7"): the reversed journal
  hunk reads a missing journal as an empty tail.
- `store_marker_tests::a_staging_directory_is_refused_even_with_a_valid_marker` (`store_marker_tests.rs:238`,
  `unwrap_err` on `Ok`): the reversed `verify` admits a staging-named directory with a valid marker.

Not evidence (red only because the control has no such step): `a_restore_interrupted_inside_its_tail_rebind_*`,
`*_before_publication_*` and `*_after_publication_*` fail at `assert!(interrupted, …)`;
`a_staged_copy_with_its_own_marker_is_refused_by_every_open_at_publication` (`:1060`, "the restore never
reached publication") and `a_failed_publication_removes_only_the_restores_own_staging` (`:1093`, the base
restore succeeds) find no publication step in the base restore;
`backup_roundtrip_preserves_real_core_store_wal_and_metadata` fails at the branch assertion (`:188`, no branch
minted). Red at a `journal/` assertion or setup step of the carried staging hunk, not the row's symptom:
`a_backup_of_a_restored_copy_with_an_empty_journal_verifies` (`backup_tests.rs:1141`, the base restore
publishes no `journal/`), `a_backup_refuses_a_restored_copy_that_lost_its_journal` (`:1160`, no `journal/` to
remove in the base restore's copy) and `a_restore_refuses_a_backup_of_a_restored_copy_without_its_journal`
(`:1181`, the base backup keeps no empty `journal/`). Green in the control and regressions only: the mc-core
frame-binding and rebind refs, the other mc-world marker refs.

## Control of `f0d7f039d` reversed on its own

Its production hunks reversed on its own tests (everything else at `f0d7f039d`; the kept API above stays):
`receipts/D123PersistDiv/rework2/rework-control.patch`, run `.analysis/validation/20261124T090343-test-eg3mzj_4`
(log copied to `receipts/D123PersistDiv/rework2/rework-control-test.log`), 8 of 47 red:

- `a_staged_copy_with_its_own_marker_is_refused_by_every_open_at_publication` (`backup_tests.rs:879`, at the
  `Publishing` point: "Core::open … : None", the staged copy opened) and
  `a_restore_interrupted_before_publication_leaves_the_target_untouched` (same assertion on the leftover) —
  review blocker 1;
- `a_failed_publication_removes_only_the_restores_own_staging` (`backup_tests.rs:1096`, "the failed restore's
  cleanup removed data it does not own", left `None`) — blocker 2;
- `a_backup_refuses_a_restored_copy_that_lost_its_journal` (`:1162`, backup returned `Ok(2)`) and
  `a_restore_refuses_a_backup_of_a_restored_copy_without_its_journal` (`:1183`, restore returned `Ok`) —
  blocker 3;
- `store_marker_tests::a_staging_directory_is_refused_even_with_a_valid_marker` (`:238`) — blocker 1 at the
  marker check;
- not evidence: `a_restore_interrupted_before_its_first_rebind_*` and `*_inside_its_tail_rebind_*` at
  `:879`, refused there by the missing marker instead of the reserved name.

## Control of `5ae472035` reversed on its own

Its production hunks reversed on its own tests: `receipts/D123PersistDiv/rework3/rework3-control.patch`, run
`.analysis/validation/20261124T092808-test-6iac4qv_` (log `receipts/D123PersistDiv/rework3/rework3-control-test.log`):

- `a_restored_copy_upgraded_from_a_legacy_clock_keeps_its_journal` (`backup_tests.rs:1282`, "the upgraded
  restored copy does not open": "journal … is missing, but this world is a restored copy on branch …") — the
  third review's blocker;
- not a row ref: `game_clock_upgrade::tests::copy_command_checkpoints_legacy_tail_then_archives_identical_wal`
  (`game_clock_upgrade_tests.rs:94`, the root copy's `journal/` was renamed away; the test now pins the kept,
  empty directory).

The `backup_tests.rs:1282` above is that run's file; the round-4 marker assertion was inserted before it since.

## Control of `fb408c908` reversed on its own

Its production hunk reversed on its own tests (the upgrade without the marker write, as at `5ae472035`):
`receipts/D123PersistDiv/rework4/rework4-control.patch`, run `.analysis/validation/20261124T100511-test-cjqalhn7`
(log `receipts/D123PersistDiv/rework4/rework4-control-test.log`), 3 of 5 red:

- `a_restored_copy_upgraded_from_a_legacy_clock_keeps_its_journal` (`backup_tests.rs:1280`, "the WAL was
  archived with the marker still at the old checkpoint": marker tick 1, converted tick 2, same store id and
  branch) and, root branch, `game_clock_upgrade::tests::copy_command_checkpoints_legacy_tail_then_archives_identical_wal`
  (`game_clock_upgrade_tests.rs:135`, same message) — the fourth review's blocker, read before any open;
- `a_failed_marker_write_leaves_the_wal_in_the_journal` (`game_clock_upgrade_tests.rs:178`, the upgrade
  returned `Ok` and archived the WAL: the pre-image writes no marker, so nothing stops the move).

In the full-patch control `a_restored_copy_upgraded_from_a_legacy_clock_keeps_its_journal` is not evidence:
the base restore mints no branch, so it stops at its setup (`backup_tests.rs:1265`); nor is
`a_failed_marker_write_leaves_the_wal_in_the_journal` (`game_clock_upgrade_tests.rs:178`: the pre-image has
no marker write to fail).

Promotion run: `.analysis/validation/20261124T100818-ledger-promote-4ar7mzw0` (tree `c9a119d769`, clean,
`control_tree: head`, nine reversed files); control `ledger-1-run.log` 15 of 19 red (4 green: the two mc-core
frame/rebind refs, `a_minted_branch_uses_all_64_bits`, `the_crash_window_rewrite_keeps_the_branch`; proving
reds at `backup_tests.rs:762`, `:865`, `journal_tests.rs:2445`, `store_marker_tests.rs:238`), restore
`ledger-3-run.log` 19 of 19 green.
