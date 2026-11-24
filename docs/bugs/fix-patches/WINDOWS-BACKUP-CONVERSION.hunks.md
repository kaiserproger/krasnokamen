# WINDOWS-BACKUP-CONVERSION — patch-mode fix identity (bespoke HEAD form)

Fix: `docs/bugs/fix-patches/WINDOWS-BACKUP-CONVERSION.patch`
(`fix_commit = "patch:2e9076cdbfbf189d5a33f8f56d63b521cb3bb99e6dd6ad1242240b8d275fb599"`).
Re-audit:
`python3 -m tools.harness ledger promote WINDOWS-BACKUP-CONVERSION --fix-patch docs/bugs/fix-patches/WINDOWS-BACKUP-CONVERSION.patch --dry-run`

## Why patch mode

The fix landed in `c1f387bbcb` "fix(storage): preserve leases during backups and
canonicalize conversion paths" (14 files, 768 diff lines). Its hunks no longer
reverse on the tip for six of the code files (`git apply -R --check` fails on
`core.rs`, `core/journal.rs`, `hot_backup.rs`, `mc-edge/backup.rs`,
`mc-world/native.rs`, `mc-world-conversion/lib.rs` — only
`mc-world-conversion/import.rs` still reverses), because later commits renamed
or moved the surrounding code (`SaveGenerated` → `RequireGenerated`, the
store-path plumbing removed, the native backend rewritten). The identity is
therefore the HEAD equivalent of each original hunk, per README patch-mode
step 1.

## What the patch carries (HEAD equivalents)

| File | Reverted (a/) behaviour | Candidate (b/) behaviour |
|---|---|---|
| `mc-edge/src/backup.rs` | `run_backup`/`run_restore` copy and hash the database through a second `File::open` handle while the lease is held | the database is read through the lease-holding handle (`copy_reader_durable`/`digest_reader`, `rewind`), so Windows does not deny the read |
| `mc-core/src/hot_backup.rs` | `copy_world` ignores its `database` reader and copies `world_dir/NATIVE_DATABASE` by path; directories fsync through `File::open(d).sync_all()` | copies the owned snapshot reader (`copy_reader_hashed(database, ..)`) and syncs through `crate::journal::fsync_dir` |
| `mc-core/src/core/journal.rs` | the backup job passes `&mut std::io::empty()` into `copy_world` | passes `&mut store.snapshot_reader()` |
| `mc-world/src/native.rs` | `NativeStore` keeps only the database and reopens it by path: `snapshot_reader()` is a second `File::open(&self.path)`, and `create`/`open`/`open_learning_identity` use `create_file`/`open` | the store owns the locked `Arc<File>` (`NativeFile` backend, `create_with_backend`) and `snapshot_reader()` reads through that handle |
| `mc-world-conversion/src/lib.rs` | `files()`/`walk()` join raw OS separators (`relative.join(entry.file_name())`) and do not skip the world lease | paths are slash-normalized (`resource_path::slash_relative`) and `WORLD_LEASE_FILE_NAME` is skipped |
| `mc-world-conversion/src/import.rs` | saved-data keys use `canonical.to_str()` | keys use `resource_path::slash_relative` |

No original non-test hunk is left out. Docs hunks (`OPERATING.md`, ADR 0013)
and test hunks are kept by `non_test_patch` as usual.

## a/-side adaptations (kept API, reason (b))

- `hot_backup::copy_world` keeps its `database: &mut impl Read` parameter
  (`hot_backup_tests` call it) and ignores it on the a/-side (`let _ =
  database;`).
- `NativeStore::snapshot_reader()` keeps its signature (`mc-world` backup
  tests and `core/journal.rs` call it) and returns a second file handle on the
  a/-side. A `path: PathBuf` field replaces the owned `file` handle there, the
  only added field.
- The `NativeFile` backend and `NativeSnapshotReader` stay defined on the a/-
  side (unused, warnings only), so the diff stays a behaviour reversal rather
  than a rewrite of the type.

## Proving refs

- `cargo:mc-world-conversion:path_tests::walked_paths_are_portable_metadata_keys_and_exclude_world_lease`
  — red on the a/-side at `path_tests.rs:22` (observed in
  `.analysis/validation/20261123T2356*`-era run; the run artifact is recorded
  in the promotion JSON). The a/-side joins `\`-separated keys and keeps the
  lease file, which is exactly the row's `validate_relative_path` symptom.
- `cargo:mc-edge:backup::tests::offline_backup_restore_copies_database_under_real_file_leases`
  — **green on Linux in the control** (a second handle succeeds there). It is
  coverage, not evidence; the Windows `LockFileEx` sharing violation itself
  still needs the Windows VM control.
- The mc-world `native::backup_tests::snapshot_reads_lease_holding_handle_without_releasing_database_custody`
  and the `hot_backup`/`stop_tests` refs run in the restore; expect them green
  on the candidate.

## Fix in several commits

| Commit | Closes |
|---|---|
| `c1f387bbcb` | the path/lease behaviour in all six files above (the original fix) |
| later refactors (`SaveGenerated` → `RequireGenerated`, store plumbing removal, native backend) | moved the code, which is why this identity is the forward-ported form rather than a reversal of `c1f387bbcb` |
