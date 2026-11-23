# WAL-OPAQUE-OWNED-ITEM-FIELD-DROPPED — negative-control patch

Fix: `docs/bugs/fix-patches/WAL-OPAQUE-OWNED-ITEM-FIELD-DROPPED.patch`
(`fix_commit = "patch:81e40fc89c6a483fc964c376b54afc072fcd1aabf99f1d19a3273c06da141e21"`, sha256 of that file).
`git apply -R` restores the pre-fix form.

## Multi-commit production patch

The single hunk's two blocks come from two commits laid over each other:

- `0f7c003b` (root-only fix, control id `0f7c003bdc17…` in the row): refuses recovery/export when a single opaque
  inventory field carries repeated or missing `Slot`s, and when a nonempty opaque `krasnokamen:Carried` sits next to an
  empty typed cursor. Four refusal assertions fail with it reversed.
- `d3ba4cdc` (recipe-only reversal `d3ba4cdce0a0…`): keeps the typed `recipeBook.recipes` list as the authority instead
  of letting export replace it from the unlocked set. Other recipeBook fields remain retained.

## Proving ref

`cargo:mc-world-conversion:wal_owned_player_fields_tests::repeated_inventory_slot_in_single_opaque_field_refuses_wal_recovery_and_export`,
assertion at `crates/mc-world-conversion/src/wal_owned_player_fields_tests.rs:78`; the module's six refusals pass in
the restored tree (module 6/6) and the clean synthetic WAL/export passes.

## Custody evidence

- `receipts/ImportSlotFix/server-clean-wal-inspection.json` — status `passed`: 14 preserved real server frames, one
  Player post-image at tick 410 with 41 typed slots and empty opaque data; same-pin restart restores
  `minecraft:stone x16` in slot 36.
- `receipts/ImportSlotFix/REPORT.md:31-43` and
  `receipts/ImportSlotFix/server-clean/20261115T095907-protocol-matrix-_d313gvc/result.json` (status `passed`).
- Independent custody re-review: Opus, `ReviewD123Journal1`, 2026-11-23 — reviewed the fail-closed policy and accepted
  the promotion; artifact `.analysis/validation/20261123T160616-ledger-promote-qop51n37`.

## Not carried

No production hunk is excluded here: the patch is the full multi-commit production change, and the registered refs
compile against both sides (the pre-fix form keeps the typed structs the tests name).
