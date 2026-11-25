# HOT-BACKUP-OFFLINE-STATUS-BEFORE-VERIFY: patch-mode fix identity

`fix_commit = "patch:ddc019bdf22ac21be427b0cb1a51bce575609f4801983cfc4ad34d231422dc21"`
(`docs/bugs/fix-patches/HOT-BACKUP-OFFLINE-STATUS-BEFORE-VERIFY.patch`).

Original fix: `92a28e7186` "fix(persistence): verify a backup from the copy's own
one open" — the **reporting half** of it. The same commit's verification half is
HOT-BACKUP-OFFLINE-RESTORE-UNVERIFIED (`patch:7fde9964…`), whose own `hunks.md`
names this row as the complementary split and requires these hunks to be
carried together.

## Why not the commit itself

The commit is two rows' behaviour. Reversing it whole would revert the
verification path that HOT-BACKUP-OFFLINE-RESTORE-UNVERIFIED proves (its
`restore` ref would turn red on another row's assertion), the `snapshot_reader`
handle work that belongs to WINDOWS-BACKUP-SECOND-HANDLE-ERROR-33, and the
`hot_backup.rs` hunks, which no longer reverse at this HEAD (the sibling row's
`hunks.md` records that). The `mc-edge` reporting hunks still reverse verbatim
on this candidate, so the patch is exactly them.

## Carried (the reporting path, byte-identical to the original)

| original | HEAD |
|---|---|
| `backup.rs` `-226,7`: `run_backup` returns `Result<usize>` | `crates/mc-edge/src/backup.rs:233` |
| `backup.rs` `-268,13`: the final `println!` replaced by `Ok(manifest.files.len())` | `crates/mc-edge/src/backup.rs:271-279` |
| `operator_cli.rs` `-314,15`: `Command::Backup` arm prints one final status after `verify_written_backup` | `crates/mc-edge/src/operator_cli.rs:326-350` |

Blob pair `3450417f7` -> `d3f02c038` is the commit's own pair for the whole
`backup.rs`; the reversed file here is **not** blob `d3f02c038` (it keeps the two
excluded verification hunks), so the patch's `index` line only names the blobs
of the original commit and the hunk headers shift accordingly. `git apply -R
--check` passes on the candidate for all three hunks, and every `a/`-side line
is the pre-fix code verbatim.

## Excluded

- (a) `backup.rs` hunk deleting `backup_expected_tick` and the `verify_backup`
  rewrite (the copy's one verification open): HOT-BACKUP-OFFLINE-RESTORE-UNVERIFIED's
  path; reversing it there is that row's red, and the kept `restore` test asserts it.
- (a)/(c) `crates/mc-core/src/hot_backup.rs` and `crates/mc-world/src/native.rs`
  hunks: the sibling row's verification instance and the Windows second-handle
  row respectively; `hot_backup.rs` was rewritten by later commits and does not
  reverse here.
- `crates/mc-edge/src/backup_tests.rs` is a test file and stays on both sides;
  the control keeps the new parent and child tests.

## The control

Reversed, `run_backup` prints `backup of … complete: N files` and returns `()`
before `operator_cli.rs` verifies the copy and withdraws its manifest on
failure. The registered ref
`cargo:mc-edge:backup::tests::backup_announces_complete_only_after_verification`
re-executes the test binary as `backup::tests::backup_status_child_reexec` with
`--nocapture`, so the real in-process `backup` command prints into a pipe the
parent reads; the child arms the copy interruption hook (`Step::Copied`) to
overwrite the copied `native/state.redb` once it is written and synced, so
`run_backup` succeeds and verification is what fails.

On the reversed tree the parent fails its own assertion at
`crates/mc-edge/src/backup_tests.rs:1653` with the child's pre-fix line
(`.analysis/validation/20261125T191021-ledger-promote-g3pypqj1/ledger-1-run.log:650`):

```
a copy that failed verification was announced complete; stdout
…
test backup::tests::backup_status_child_reexec ... backup of /tmp/…/world into
/tmp/…/backups/copy-1 complete: 4 files
```

On the candidate the same ref passes and the failing command prints no
`complete` line at all; stderr names the withdrawal
(`the backup was withdrawn (no manifest kept)`), which the parent also asserts.
