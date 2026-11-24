# DIVERGED-BACKUP-COPIES-SHARE-STORE-IDENTITY — negative-control patch

Fix: `docs/bugs/fix-patches/DIVERGED-BACKUP-COPIES-SHARE-STORE-IDENTITY.patch`
(`fix_commit = "patch:0ec63ae44ee635104a5c5cbc5897a7c69ab0566e42f3ee2e06bdb450af38b7ff"`, sha256 of its
non-test hunks, here the whole file). It is a diff from the pre-fix form to the candidate; `git apply -R`
restores the pre-fix form. The row was promoted once on `01ac685ba7` alone; the persistence review
(`receipts/D123PersistDiv/review/REVIEW-ReviewD123Diverged-Sol1.md`, blockers 1-2) showed that an
interrupted restore of that commit can leave a copy that opens on the backup's own branch, the row's
symptom by another path, so the row was reopened with the completed fix.

## Commits covered

- `01ac685ba7` "persist: give every restored backup its own branch" — `run_restore` mints a branch that is
  neither the root nor the backup's own, moves the copied tail above the checkpoint onto it
  (`journal::rebind_tail`) and writes the marker naming it. Closes the row's main path: two copies of one
  backup, or a copy and its source, refuse each other's tails.
- `cee142f323` "persist: publish a restored world from an unpublished staging directory" — the same steps
  run in `<target>.restore-staging`, which no open admits (the backup's marker is verified from memory and
  never copied; the copy's own marker is written last), every directory is fsynced and one rename
  publishes the finished copy; every restored copy has `journal/` and replay refuses a restored copy
  without one; an offline backup keeps an empty `journal/`. Closes the interrupted-restore path: before
  it, a restore killed between its copies and its marker write left the backup's store, marker and tail
  on the backup's branch at the target, and a lost `journal/` read as an empty tail.

## Carried hunks

`crates/mc-edge/src/backup.rs`, a/-side = the base blob `d3f02c0381a1` (`9b6ddef165`) plus the two
`#[cfg(test)]` copy-hook lines of `742550fb0f` in `copy_reader_durable` (not a fix commit; context on both
sides, so no a/-side text outside the original pre-image):

- module doc (`cee142f323`): "builds the world in a staging directory … one rename";
- `run_backup` keeps an empty `journal/` (`cee142f323`);
- `run_restore` (`01ac685ba7` call of `start_branch`, rewritten by `cee142f323`): a/ = the base in-place
  copy, pair check against the copied marker and cleanup; b/ = staging path, mount-point refusal,
  leftover-staging refusal, `stage_world`, `publish`, cleanup, post-rename parent fsync and the
  `Publishing`/`Published` interruption points;
- `staging_path`, `refuse_mount_point`, `stage_world`, `start_branch` (`01ac685ba7`, rewritten to take the
  backup's marker from memory and run in the staging directory), `sync_tree`, `publish` (a/: absent).

`crates/mc-core/src/journal.rs`, a/-side = the blob at `742550fb0f` (no a/-side adaptation):

- `visit_complete_batches_after`: a missing journal directory is an empty tail only on the root branch
  (`cee142f323`; a/: always an empty tail) and its doc;
- the `rebind_tail` doc sentence on where restore runs it (`cee142f323`).

## Not carried, with the ground (docs/bugs/README.md classes)

- (b) API the kept tests call: `rebind_tail`'s `before_batch` hook and `StoreMarker::admits`
  (`742550fb0f`, not a fix commit) and the `backup_tests.rs` interruption machinery (`Step`,
  `interruption_point`) stay; the hook call sites inside `start_branch`, `run_restore` and `publish` are
  part of the carried hunks and disappear with them.
- Not a fix commit: `be1ab01c71` (64-bit `mint_branch`, docs) and the format-3 frame of `49d3f0d53e`
  (infrastructure the refs need to compile and the base restore never touches).
- Docs only: persistence.md, ADR 0013, OPERATING.md, MIGRATION.md.

## Proving assertions (red in the control)

- `backup::tests::a_restored_copy_refuses_the_journal_of_another_copy_of_its_backup`, the refusal assertion
  (`backup_tests.rs` "the left copy replayed the right copy's journal"): with no branch minted the sibling
  tail replays.
- `backup::tests::a_restore_interrupted_before_its_first_rebind_leaves_nothing_that_opens`,
  `assert_no_world_at` ("an interrupted restore left … at the target"): the base restore killed after its
  last copy leaves the backup's store, marker and tail on the backup's branch in the target.
- `journal_tests::a_missing_journal_is_an_empty_tail_only_on_the_root_branch`, its `expect_err` ("a restored
  copy without its journal lost its tail"): the reversed journal hunk reads a missing journal as empty.

Not evidence (red only because the control has no such step): `a_restore_interrupted_inside_its_tail_rebind_*`,
`*_before_publication_*` and `*_after_publication_*` fail at `assert!(interrupted, …)`;
`backup_roundtrip_preserves_real_core_store_wal_and_metadata` fails at the branch assertion (no branch minted).
Green in the control and regressions only: `a_backup_of_a_restored_copy_with_an_empty_journal_verifies`, the
mc-core frame-binding and rebind refs, the mc-world marker refs.
