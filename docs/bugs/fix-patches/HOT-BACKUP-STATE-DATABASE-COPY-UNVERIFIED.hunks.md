# HOT-BACKUP-STATE-DATABASE-COPY-UNVERIFIED — negative-control patch

Fix: `docs/bugs/fix-patches/HOT-BACKUP-STATE-DATABASE-COPY-UNVERIFIED.patch`
(`fix_commit = "patch:<sha256 of that file>"`).

## Original commits

The row's path was closed in two commits, and the patch carries the non-test
hunks of the second one, which is the part a HEAD control must reverse:

- `5cdafd6588` — `Core::backup` refuses a copy that does not restore: the
  in-process path. Its hunks (`core.rs`: the `CoreInner.config` field, the
  `verify_restorable` call) are still in the tree, but `hot_backup.rs` was
  rewritten afterwards, so the commit's own hunks no longer reverse on HEAD.
  The ref it left red is still registered (`hot_backup_tests::
  backup_refuses_a_copy_that_does_not_restore`); it is not the control used here.
- `92a28e7186` — offline/CLI verification from the copy's own one open.
- `7988d0db4b` — the same path under the plugins a serve resolves: `verify_copy`
  uses `native_checkpoint::recovery_registry` (content + plugin startup
  contribution) and `verify_opened` applies `plugin::check_startup_identity`.
  The patch carries these hunks too: reversing only `92a28e7186` no longer
  applies on HEAD, and a reversed copy must not be "verified" under a bare
  registry either.

## Hunk mapping

| Patch area | HEAD file | What it is |
|---|---|---|
| the `Verification`/`open_verification`/`OpenedStore` rewrite | `crates/mc-core/src/hot_backup.rs` | one instance over the copy, read-only or repaired scratch in the copy's parent |
| `verify_copy`, `published_tick_through` | `crates/mc-core/src/hot_backup.rs` | offline verification learns identity/store id/checkpoint from the copy and expects the highest published tick |
| `verify_restorable` identity check | `crates/mc-core/src/hot_backup.rs` | the hot path keeps its "the copy is this world" check |
| — (excluded, see below) | `crates/mc-world/src/native.rs` | `open_learning_identity`/`validate_header_shape` |
| `verify_backup` | `crates/mc-edge/src/backup.rs` | offline verify calls `verify_copy`; the read-only `read_identity`/`read_marker_fields` pair is gone, and with it `backup_expected_tick` |
| `Backup` arm | `crates/mc-edge/src/operator_cli.rs` | final status printed after verification, no-content mode marked |

Reversed, these files return to `4785cbc83e`, which builds: `verify_restorable`
keeps its `Core::backup` caller and the refused copy still yields an error, so
`hot_backup_tests::backup_refuses_a_copy_that_does_not_restore` stays green.
`recovery_registry` and `check_startup_identity` stay defined (they live in
`native_checkpoint.rs` and `plugin.rs`, outside this patch) and become unused —
warnings, not errors. `backup_expected_tick` comes back with `backup.rs`.

## Exclusions

- `crates/mc-world/src/native.rs` (`open_learning_identity`, `validate_header_shape`):
  reason (b) — the kept tests call it (`journal::test_wal_identity`,
  `read_complete_batches_after_test`), so reversing it stops the control from
  compiling. `hot_backup.rs` at `4785cbc83e` does not use it, so the reversed
  tree still builds.

- `crates/mc-edge/src/backup_tests.rs` and
  `crates/mc-edge/src/operator_cli_tests.rs` are test hunks; the ledger keeps
  them and reverses only production code.
- The WAL frame and journal refusal hunks are other rows
  (`WAL-BATCH-HAS-NO-STORE-IDENTITY`, `WAL-BLOCK-STATE-IDS-HAVE-NO-PALETTE-IDENTITY`).

## a/-side adaptation

None.

## Proving ref

`cargo:mc-edge:backup_tests::the_restore_command_accepts_a_hot_backup_verified_with_content`
— the real `restore` command on a hot backup with installed content. Reversed,
`verify_backup` opens the archive read-only first, a hot copy returns
`RepairAborted`, and the test fails at
`crates/mc-edge/src/backup_tests.rs:648` (`expect("the restore command verifies
and restores a hot backup")`). That is the reviewer's finding: a backup the
service called complete could not pass the server's own `restore`.
