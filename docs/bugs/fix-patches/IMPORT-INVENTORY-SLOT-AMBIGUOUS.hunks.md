# IMPORT-INVENTORY-SLOT-AMBIGUOUS — patch-mode fix identity

Fix: `docs/bugs/fix-patches/IMPORT-INVENTORY-SLOT-AMBIGUOUS.patch`
(`fix_commit = "patch:e71738fc39a6c19be3e974c6cd51437ce4df3ea8d3839106e63c1de9fefe9993"`).
It is a diff from the pre-fix form to this candidate; `git apply -R` restores the
pre-fix code (the acceptance of a missing, repeated or unsupported custody slot).
Re-audit:
`python3 -m tools.harness ledger promote IMPORT-INVENTORY-SLOT-AMBIGUOUS --fix-patch docs/bugs/fix-patches/IMPORT-INVENTORY-SLOT-AMBIGUOUS.patch --dry-run`

This patch supersedes `patch:f37f927a…` (which left the moved
`validate_legacy_player_slots` definition and the `admit_player` message out of
the identity with a "path relocation" label, not an (a)/(b)/(c) reason; the
fifth revision carries both) and the earlier shipped `patch:7a364f8e…`, whose
hunks no longer reversed on the candidate. Every original non-test hunk is carried
unless it is another row's behaviour (a), API the kept tests call (b), or code a
later commit rewrote with no HEAD equivalent (c); each exclusion is named below.

## Original commits

- `1e13fffe5` "fix(import): refuse ambiguous player inventory custody" — the
  `Inventory` decode, the `EnderItems` decode, the equipment map, the
  legacy-slot pre-fold validation and its two call sites.
- `ce786e6fb` "fix(custody): refuse malformed player recovery images" — the
  `stack_from_player_fields` helper, the duplicate-field helper, the
  `player_from_tag` equipment/`krasnokamen:Carried` strictness, the recovery
  image validation and the WAL batch checks.
- `fb19cab5e` "fix(custody): refuse duplicate player custody in WAL opaque
  fields" — `validate_owned_custody_fields` (incl. `recipeBook.recipes`) and the
  per-entry opaque/WAL-inventory-slot checks in `recover_with_clock`.

## Hunk mapping

| Patch hunk (new side) | Original | What it is |
|---|---|---|
| `mc-core/src/ender_chest.rs:40-68` | 1e13fffe5 | `contents`: named per-entry `Slot` failures, duplicate field / out-of-range / repeat slot refused instead of a silent `Err` or overwrite |
| `mc-core/src/journal/player_nbt.rs:44-50` | ce786e6fb | `player_image_to_tag` validates owned custody fields before projecting them into the opaque record |
| `mc-core/src/journal/player_nbt.rs:304-365` | ce786e6fb + fb19cab5e | `validate_owned_custody_fields` and `validate_player_opaque_custody` (duplicate owned key, duplicate `recipeBook.recipes`, trailing NBT), re-written here on HEAD's extended body |
| `mc-core/src/journal/player_nbt.rs:372-383` | 1e13fffe5 | `player_from_tag` validates the root compound's owned fields and refuses an unsupported equipment name instead of dropping the stack |
| `mc-core/src/journal/player_nbt.rs:437-467` | ce786e6fb | `krasnokamen:Carried` and `krasnokamen:CraftingGrid` decode through `stack_from_player_fields`, so payload without an id is refused |
| `mc-core/src/native_checkpoint.rs:866-878` | ce786e6fb | recovery wraps a player decode error with the player uuid |
| `mc-core/src/native_checkpoint.rs:1010-1037` | fb19cab5e | WAL recovery batch validates opaque custody and the 41-slot inventory length per player entry |
| `mc-data/src/player_nbt.rs:125-160` | ce786e6fb | `stack_from_player_fields` (nonempty payload without a string id is an error) and `custody_field` (a repeated key is an error) |
| `mc-data/src/player_nbt.rs:223-266` | 1e13fffe5 | `player_from_fields`: `Inventory`/`equipment` keyed decode, missing/out-of-range/repeated `Slot` refused, equipment names pinned to the eight supported slots |
| `mc-world-conversion/src/import.rs:171-177` | 1e13fffe5 | the embedded-player path validates legacy slots before `upgrade_player` folds equipment |
| `mc-world-conversion/src/import.rs:1350-1356` | 1e13fffe5 | the player-file path validates legacy slots before the DFU upgrade |
| `mc-world-conversion/src/import/players.rs:104-149` | 1e13fffe5 | the moved `validate_legacy_player_slots` definition itself (the a/-side deletes it and drops its name from the `use` line in `import.rs`, so the control has the pre-fix shape: the function does not exist and legacy slots are not validated) |
| `mc-world-conversion/src/import/players.rs:166-171` | 1e13fffe5 | the moved `admit_player` decode-error message carries the player uuid (a/-side reverts to `{path}: {error}`) |

## Exclusions

None. Both hunks the previous identity called "relocations" are carried at
their HEAD paths above.

## a/-side adaptations

None. The pre-fix `Inventory`/`equipment` decode is the historical text of
`1e13fffe5`, kept verbatim; `stack_from_fields` (which it calls) stays in the
file; the moved definition and message are reversed at their HEAD paths, and
the `use` line in `crates/mc-world-conversion/src/import.rs:33` drops the
deleted name — the one adaptation the post-fix split forces, because the
pre-image's single `import.rs` had no separate `use players::{..}` line.

## Proving refs

The control goes red by assertion on `player_slot_import_tests` and on the
mc-core custody tests (`promote` receipt
`.analysis/validation/20261123T235118-ledger-promote-70x6am7i`):
`native_player_custody_tests.rs:425` (duplicate recipe book),
`:137` (long/short WAL inventory), `:301` (duplicate crafting grid / carried
before first match), `:347` (missing item id with payload), `:112` (duplicate
carried / crafting grid in WAL opaque). For example, in
`player_slot_import_tests`, e.g.
`missing_inventory_slot_refuses_import_before_load_save` reaches
`refused()`'s `expect_err("ambiguous custody must refuse import before load/save")`
(`crates/mc-world-conversion/src/player_slot_import_tests.rs:57`) because the
decode silently skips the stack and the import succeeds; the same list carries
`duplicate_*_wal_opaque` and `missing_item_id_with_payload` refs that lose their
guards with the `journal`/`native_checkpoint` hunks reversed. The mc-world ref
`native::checkpoint_tests::player_duplicate_custody::repeated_player_custody_names_refuse_raw_checkpoint_recovery`
is green in the control: that duplicate-name guard lives in mc-world's own
reader, outside this identity's hunks, so it is coverage here, not a
discriminating red. Measured receipts
are in `docs/bugs/promotions/IMPORT-INVENTORY-SLOT-AMBIGUOUS.json` after the
promotion.
