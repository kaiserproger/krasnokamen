# HOT-BACKUP-STATE-DATABASE-COPY-UNVERIFIED: bespoke reversible form

`fix_commit = "patch:bfab16b165d49e12c75742231c886f96b9241fafbc7457ebccd22f556a40ff17"`
(`docs/bugs/promotions/HOT-BACKUP-STATE-DATABASE-COPY-UNVERIFIED.patch`), approved
by Main as the bespoke form for this row.

## Why not the shipped patch a00ae565

Its `a/`-side removes `hot_backup::verify_copy`, which the registered refs call
(`hot_backup_tests.rs:420` and `:442`), so the control build fails with E0425
and no assertion can run (`.analysis/validation/20261123T161102-ledger-promote-uqz3cx65/ledger-0-list.log`).
Keeping the API but making verification a no-op was rejected as a
hand-weakened variant.

## Carried (1 hunk)

| HEAD |
|---|
| `core.rs:1780-1787` `Core::backup` verifies the copy (`verify_restorable`) before reporting success |

`verify_copy`/`verify_restorable` keep their signatures on both sides (class
(b): the registered refs read them). No counter or assertion text is invented:
both sides run the registered tests unchanged.

## Excluded

- (a) the offline/CLI path, a different behaviour of the same fix series:
  `92a28e7186` (verify the copy from the copy's own one open) and
  `57f90fd920` (backup and restore read the operator's `--config`). They are
  HOT-BACKUP-OFFLINE-RESTORE-UNVERIFIED (open), and its ref
  `mc-edge backup::tests::the_restore_command_accepts_a_hot_backup_verified_with_content`
  was green in this control, so it proved nothing here and the ref lives on
  that row.

## The control

`hot_backup_tests::backup_refuses_a_copy_that_does_not_restore` fails at
`hot_backup_tests.rs:352` with `an unrestorable copy is not a backup` for both
`test_hooks` modes (`DATABASE`, `UNREPLAYABLE_BATCH`): the pre-fix backup
reported the unrestorable copy as a success. Receipt
`.analysis/validation/20261124T042601-ledger-promote-24i6p8zj/ledger-1-run.log:385`
(the control run of the promotion recorded in
`docs/bugs/promotions/HOT-BACKUP-STATE-DATABASE-COPY-UNVERIFIED.json`).
