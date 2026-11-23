# IMPORT-ANVIL-PRE-RENAME-KEYS-IGNORED — patch-mode fix identity

Fix: `docs/bugs/fix-patches/IMPORT-ANVIL-PRE-RENAME-KEYS-IGNORED.patch`
(`fix_commit = "patch:8e2d0965f3aa38d090cb3b411ad4b59518716f511032c38f1ee5dcd0b713120f"`).
It is a diff from the pre-fix form to this candidate; `git apply -R` restores
the pre-fix code. Re-audit:
`python3 -m tools.harness ledger promote IMPORT-ANVIL-PRE-RENAME-KEYS-IGNORED --fix-patch docs/bugs/fix-patches/IMPORT-ANVIL-PRE-RENAME-KEYS-IGNORED.patch --dry-run`

## Why patch mode

The fix is `7df454ee1` "fix(conversion): import RC5 solaris:* fields under
their krasnokamen names". Its control cannot run at the fix commit: the tree at
`7df454ee1` no longer builds under this dependency set
(`predicates::Evaluator::conditions`, E0599; artifact
`.analysis/validation/20261123T184749-ledger-promote-qna1vwpj`). The commit is
also deliberately stale: `pre_rename.rs` grew from its 119-line addition to the
owner-aware walk on HEAD, so its raw non-test hunks do not reverse-apply.

## What the patch carries

The behavioural hunk of the fix — the field renaming on the import path.
`PreRename::rename` walks the record and renames `solaris:*`/`Solaris*` to the
Krasnokamen spelling (the HEAD equivalent of the original
`walk`/`current_name` body, which a later commit rewrote to be owner-aware).
Reversed, `rename` is a no-op: the old names stay unread extra data and the
receipt counts nothing, which is the row's symptom.

## Exclusions

- The module's API (`PreRename`, `Owner`, the `rename` signature) and the
  caller wiring in `import.rs`/`lib.rs` stay on both sides: (b) the kept test
  `pre_rename::tests::rc5_export_with_a_player_rider_keeps_its_fields_and_the_rider`
  and the import path call them, so reversing them stops the control from
  compiling.
- `native_checkpoint.rs`'s four-line hunk from the same commit is not carried:
  (c) that call site was rewritten by the later checkpoint/envelope work and the
  row's proving ref never reaches it.

## Proving ref

`cargo:mc-world-conversion:pre_rename::tests::rc5_export_with_a_player_rider_keeps_its_fields_and_the_rider`
goes red at `pre_rename_tests.rs:411`, `assert_eq!(receipt.counts.get("pre_rename_keys"), Some(&renamed))`:
with the rename a no-op the receipt reports zero renamed keys instead of the
player, horse and level keys the export carries.
