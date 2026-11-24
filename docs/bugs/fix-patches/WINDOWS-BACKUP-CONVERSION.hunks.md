# WINDOWS-BACKUP-CONVERSION — patch-mode fix identity (Linux-proven parts)

Fix: `docs/bugs/fix-patches/WINDOWS-BACKUP-CONVERSION.patch`
(`fix_commit = "patch:cc59e7252b58d25585849f9e4fbc7860fe3ae48c27e40b483a6f6ad0bf0238eb"`).
Re-audit:
`python3 -m tools.harness ledger promote WINDOWS-BACKUP-CONVERSION --fix-patch docs/bugs/fix-patches/WINDOWS-BACKUP-CONVERSION.patch --dry-run`

The Windows error-33 symptom (a second handle on the leased redb while the same
process holds `LockFileEx`) has no Linux control and is split out into
`WINDOWS-BACKUP-SECOND-HANDLE-ERROR-33` (open; needs the Windows VM control).
This row covers only what a Linux control can prove.

## Why patch mode

The original fix `c1f387bbcb` "fix(storage): preserve leases during backups and
canonicalize conversion paths" no longer reverses on the tip for its code files
(later refactors moved the surrounding code), so the identity is the HEAD
equivalent of each original hunk (README patch-mode step 1).

## Original hunks and their HEAD equivalents

| Original hunk | HEAD equivalent | Carried? |
|---|---|---|
| `hot_backup.rs` database copy through the reader | `copy_world(world_dir, out, database, …)` copies `copy_reader_hashed(database, ..)`; a/-side ignores the reader and copies `world_dir/NATIVE_DATABASE` by path | carried |
| `hot_backup.rs` fsync through the journal helper | `crate::journal::fsync_dir(&directory)`; a/-side `File::open(directory).sync_all()` | carried |
| `core/journal.rs` backup job passes the owned snapshot reader | `&mut store.snapshot_reader()`; a/-side `&mut std::io::empty()` | carried |
| `mc-edge/backup.rs` lease-holding reads | `copy_reader_durable`/`digest_reader`/`rewind` through the lease; a/-side second `File::open` handles | carried |
| `mc-world-conversion/lib.rs` portable paths | `resource_path::slash_relative` keys and the `WORLD_LEASE_FILE_NAME` skip; a/-side `relative.join(entry.file_name())` and no skip | carried |
| `mc-world-conversion/import.rs` saved-data keys | `slash_relative(Path::new(""), &canonical)`; a/-side `canonical.to_str()` | carried |
| `core.rs` `BackupJob.store` + `CoreInner.backup_store` | none: at HEAD `BackupJob` has no store field (`core.rs:340-343`; the write path uses the in-scope `store` in `core/journal.rs`), a later commit removed the plumbing | (c) |
| `mc-world/native.rs` `NativeStore::snapshot_reader` (owned-handle reader) | the hunk is excluded as (b): the kept `native/backup_tests.rs:17` drives `snapshot_reader()` and needs an independent cursor, and the kept `checkpoint_drill_tests.rs:226` builds `NativeStore { file, .. }` with a struct literal, so the a/-side cannot express the pre-fix second handle (a `try_clone()` shares the file offset and produces a bogus image — observed as the `[0,0,0,…] vs redb magic` mismatch at `native/backup_tests.rs:17`, NOT evidence). The HEAD `snapshot_reader` body stays on both sides. | (b) |

## Proving refs (the reds this row owns)

- `cargo:mc-core:hot_backup_tests::copy_reads_the_owned_database_snapshot_not_a_second_path_handle` —
  red at `hot_backup_tests.rs:107`: the a/-side copies the path instead of the
  owned snapshot, so the bytes differ.
- `cargo:mc-world-conversion:path_tests::walked_paths_are_portable_metadata_keys_and_exclude_world_lease` —
  red at `path_tests.rs:22` (`2 != 1`): the lease file is not skipped and the
  keys use OS separators.
- `cargo:mc-world-conversion:roundtrip_tests::nested_paths_import_export_and_census_under_world_leases` —
  red at `roundtrip_tests.rs:528` (`… .krasnokamen-world.lock … unsafe source`).

Coverage, not evidence: `mc-edge backup::tests::offline_backup_restore_copies_database_under_real_file_leases`
is green in the control on Linux (a second handle succeeds there);
`mc-world:native::backup_tests::snapshot_reads_lease_holding_handle_without_releasing_database_custody`
is excluded from the identity and its red under `try_clone` was an artefact.

The last full control run before the split is
`.analysis/validation/20261124T001158-ledger-promote-wtith5sa` (four reds
including the tree above); this narrowed identity re-runs the same refs.

## a/-side adaptations (kept API, reason (b))

`copy_world` keeps its `database: &mut impl Read` parameter (the kept
`hot_backup_tests` call it) and ignores it with `let _ = database;`.

## Commit

`c1f387bbcb` — the original fix; later commits
(`SaveGenerated` → `RequireGenerated`, the store plumbing removal, the native
backend) moved the code, which is why this is a forward-ported identity.
