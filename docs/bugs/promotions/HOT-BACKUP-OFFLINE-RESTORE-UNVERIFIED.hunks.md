# HOT-BACKUP-OFFLINE-RESTORE-UNVERIFIED: patch-mode fix identity

`fix_commit = "patch:7fde9964184cd1b21cfbb23cf918a80d824fb2803923e6a9797375c0751ee571"`
(`docs/bugs/fix-patches/HOT-BACKUP-OFFLINE-RESTORE-UNVERIFIED.patch`).

Original fix: `92a28e7186` "fix(persistence): verify a backup from the copy's own
one open" (the offline half of the row; the in-process half is
HOT-BACKUP-STATE-DATABASE-COPY-UNVERIFIED, `5cdafd6588`).

## Why not the commit itself

The commit's `hot_backup.rs` hunks no longer reverse at this HEAD — the file was
rewritten by later commits (`git show 92a28e7186 -- crates/mc-core/src/hot_backup.rs
| git apply -R --check` fails), and `verify_copy` / `open_verification` are
introduced by that commit, so they have no pre-image at all. The `mc-edge`
verification path, which is what the row's ref runs, still reverses; the patch is
that path's real pre-image forward-ported to the candidate.

## Carried (the mc-edge verification path, byte-identical to the original)

| original | HEAD |
|---|---|
| backup.rs -288,37 delete `backup_expected_tick` | backup.rs:288-319 |
| backup.rs -327,25 `verify_backup` body: two read-only `NativeStore` reads + `hot_backup::verify_restorable` | backup.rs:327-344 |

Blob pair `3450417f7` -> `d3f02c038` is the commit's own pair for the whole
file. Reversed, the file is **not** blob `3450417f7`: it is that blob plus the
two excluded `run_backup` hunks (the signature and the final `println!`), so the
`index` line in the patch only names the blobs of the original commit and the
patch's hunk headers shift accordingly. `git apply -R --check` passes on the
candidate for both carried hunks, and their `a/`-side text is the pre-fix code
verbatim: `verify_backup` opens `native/state.redb` through
`NativeStore::read_identity` and `read_marker_fields` — a second read-only open
of a copy no serve has ever opened, which redb refuses to repair for — and then
runs `hot_backup::verify_restorable`.

## Excluded

- (a) `backup.rs` hunk 1 (`run_backup` returns `usize`) and hunk 2 (the final
  `println!` replaced by `Ok(len)`), with `operator_cli.rs`'s `Command::Backup`
  arm: they are the reporting half of the commit — another row's behaviour,
  split into **HOT-BACKUP-OFFLINE-STATUS-BEFORE-VERIFY** on the wave-2 review
  (B1). That row must carry the two `backup.rs` hunks and the `operator_cli.rs`
  arm together, so its red lands on its own assertion. (Reversing the signature
  alone would not compile either: the kept `operator_cli.rs` consumes the
  `usize`; it is not carried here.)
- (c) `hot_backup.rs`: rewritten by later commits (does not reverse) and
  `verify_copy` did not exist before the fix.
- (a) `mc-world/src/native.rs` `snapshot_reader` hunks: the owned-handle reader
  is WINDOWS-BACKUP-SECOND-HANDLE-ERROR-33's coverage, not this path's.
- (a) `57f90fd920` ("backup and restore read the operator's `--config`"): its
  files are `operator_cli.rs` and `backup_tests.rs` only — the CLI
  configuration plumbing, not the verification path this row proves. Its
  `operator_cli.rs` hunk reverses at this HEAD but carries no part of this
  row's red; it is excluded as another concern, and the verifying `restore`
  path is unchanged by it.
- `backup_tests.rs` is a test file and stays on both sides.

## The control

Reversed, `run_restore` verifies the hot copy through the pre-fix second
read-only open and refuses it. The registered ref
`cargo:mc-edge:backup::tests::the_restore_command_accepts_a_hot_backup_verified_with_content`
fails the assertion at `crates/mc-edge/src/backup_tests.rs:668` with

```
assertion failed: the restore command verifies and restores a hot backup: native source: /tmp/.../native/state.redb: world was not shut down cleanly; start and stop the server once to repair it, then retry
```

(the assertion was made explicit instead of an `expect` so the control red is an
assert macro, not an unnamed panic). On the candidate the same ref passes in the
promotion restore: the restore command accepts the hot backup, keeps the copied
checkpoint marker, the copied WAL tail and reopens the restored world.
