# WINDOWS-BACKUP-CONVERSION — patch-mode fix identity (Linux-proven parts)

Fix: `docs/bugs/fix-patches/WINDOWS-BACKUP-CONVERSION.patch`
(`fix_commit = "patch:cc59e7252b58d25585849f9e4fbc7860fe3ae48c27e40b483a6f6ad0bf0238eb"`).
Re-audit:
`python3 -m tools.harness ledger promote WINDOWS-BACKUP-CONVERSION --fix-patch docs/bugs/fix-patches/WINDOWS-BACKUP-CONVERSION.patch --dry-run`

The Windows error-33 symptom (a second handle on the leased redb while the same
process holds `LockFileEx`) has no Linux control and is split out into
`WINDOWS-BACKUP-SECOND-HANDLE-ERROR-33` (open; needs the Windows VM control).
This row covers only what a Linux control can prove: the `/backup` path
copying the live redb by path instead of the owned snapshot, and the world lease
file being walked as a source by import/export/census. The platform-separator
(`\`) part is Windows-only too and lives in the same new row.

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
| `mc-world-conversion/lib.rs` lease skip | `WORLD_LEASE_FILE_NAME` skipped in `files()`; a/-side does not skip it, so the lease enters the sources (red at `path_tests.rs:22`, `2 != 1`) | carried |
| `mc-world-conversion/lib.rs` portable keys | `resource_path::slash_relative(root, ..)` keys; a/-side `relative.join(entry.file_name())` | carried, but no Linux red: the separator is `/` on Linux either way, so `path_tests.rs:26` passes and only the count assertion above reddens |
| `mc-world-conversion/import.rs` saved-data keys | `slash_relative(Path::new(""), &canonical)`; a/-side `canonical.to_str()` | carried; gives no Linux red (same `/` separators) — Windows-only symptom, see below |
| `mc-world/native.rs` owned-handle backend: `NativeFile`, the `file: Arc<File>` field, `create_with_backend` in `create`/`open`/`open_learning_identity` | HEAD keeps all of them; the a/-side keeps the same body (class (b): the kept `checkpoint_drill_tests.rs:226` builds `NativeStore { file, .. }` and `native/tests.rs:472` reads `store.file`; removing the field fails E0560/E0609, adding one E0063) | not carried — HEAD body on both sides |
| `core/journal.rs` `fsync_dir` helper move | HEAD's `crate::journal::fsync_dir`; a/-side `File::open(directory).sync_all()` (the pre-fix form) | carried (hot_backup.rs row above) |
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
  red at `roundtrip_tests.rs:528` (`.krasnokamen-world.lock: unsupported active
  or unsafe source file`), i.e. the same lease-skip symptom, not the separator.

Coverage, not evidence: `mc-edge backup::tests::offline_backup_restore_copies_database_under_real_file_leases`
is green in the control on Linux (a second handle succeeds there), and the
`\`-separator hunks give no Linux red either;
`mc-world:native::backup_tests::snapshot_reads_lease_holding_handle_without_releasing_database_custody`
is excluded from the identity and its red under `try_clone` was an artefact.

Control receipts: `.analysis/validation/20261124T003631-ledger-promote-hbf6r2g_`
(dry-run: `path_tests` and `hot_backup` red, the native snapshot test green) and
the promotion `.analysis/validation/20261124T003857-ledger-promote-tecmcjv1`.

## a/-side adaptations (kept API, reason (b))

`copy_world` keeps its `database: &mut impl Read` parameter (the kept
`hot_backup_tests` call it) and ignores it with `let _ = database;`. The
`core/journal.rs` call site passes `&mut std::io::empty()` to that parameter on
the a/-side — the second constructor line outside the original pre-image, along
with `let _ = database;`.

## Commit

`c1f387bbcb` — the original fix; later commits
(`SaveGenerated` → `RequireGenerated`, the store plumbing removal, the native
backend) moved the code, which is why this is a forward-ported identity.
