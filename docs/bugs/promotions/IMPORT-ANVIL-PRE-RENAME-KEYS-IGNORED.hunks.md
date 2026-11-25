# IMPORT-ANVIL-PRE-RENAME-KEYS-IGNORED — hunk map (patch mode, kept API)

Row: `IMPORT-ANVIL-PRE-RENAME-KEYS-IGNORED` (gameplay, major) — RC5 `solaris:*` entity/player fields
were kept as unread data and the import receipt reported no gap.

Real fix: `7df454ee1` ("fix(conversion): import RC5 solaris:* fields under their krasnokamen names").
It created `crates/mc-world-conversion/src/pre_rename.rs` (the rename/refusal engine) and wired it into
`import.rs`'s six NBT read points; `lib.rs` gained the module and `ConversionStats.pre_rename`, and the
receipt gained `pre_rename_keys` plus the `PRE-RENAME VALUES` gap.

The whole commit cannot be reversed: deleting `pre_rename.rs` removes the API the kept ref
(`pre_rename::tests::rc5_export_with_a_player_rider_keeps_its_fields_and_the_rider`) drives directly,
and the fix tree does not build under the current dependency set. Patch mode therefore carries only the
import wiring; the module stays compiled and imported (kept API), and the kept ref enters through the
production `import_anvil` path, so its own assertions are the control.

## Carried (HEAD equivalents)

| original hunk (`7df454ee1`) | HEAD equivalent | note |
|---|---|---|
| `import.rs @@ -849,6 +852,9 @@` `level.dat` | `crates/mc-world-conversion/src/import.rs:136-139` | `stats.pre_rename.rename(&mut level, Owner::Level, ..)`; the `Owner` argument is the HEAD form of the original closure (`Owner` type added to the module by a later commit) |
| `import.rs @@ -954,7 +960,8 @@` metadata split files | `import.rs:246-249` | `let mut tag` + `rename(&mut tag, Owner::metadata(..), ..)`; the `mut` is part of the hunk, so the a-side restores `let tag` |
| `import.rs @@ -978,6 +985,9 @@` typed saved data | `import.rs:272-276` | `rename(&mut tag, Owner::metadata(destination), ..)` |
| `import.rs @@ -1334,6 +1345,9 @@` chunk NBT | `import.rs:641-644` | `rename(&mut tag, Owner::Chunk, ..)` in the chunk `visit` |
| `import.rs @@ -1705,6 +1720,9 @@` entity regions | `import.rs:1034-1039` | `rename(&mut tag, Owner::EntityRegion, ..)` in the entity-region `visit` |
| `import.rs @@ -2037,6 +2056,9 @@` player records | `import.rs:1372-1375` | `rename(&mut tag, Owner::Player, ..)` |
| `lib.rs @@ -69,6 +70,7 @@` `ConversionStats.pre_rename` field | `crates/mc-world-conversion/src/lib.rs:89` | carried: the field feeds the receipt blocks below (the field is `pub` inside the crate-private struct, so leaving it would compile but be a kept consumer of the module outside the carried hunks) |
| `lib.rs @@ -395,6 +397,24 @@` the `pre_rename_keys` count, the rename normalization and the `PRE-RENAME VALUES` gap | `lib.rs:414-431` | carried: these are the callers of the module outside the carried hunks (README:255-257), so they belong to the control, not to kept API |
| `native_checkpoint.rs @@ -534,8 +534,8 @@` `open_refusal` message | `crates/mc-core/src/native_checkpoint.rs:604-612` | the refusal text telling operators the import now renames `solaris:*`; carried so no hunk is left out without a class |

The HEAD adaptation carried here (named per README:234-258): the call sites use the module's `Owner`
argument and the file's `use super::pre_rename::Owner;` line, which a later commit introduced when it
gave `rename` an explicit owner. The a-side of the patch removes that import together with the calls,
exactly the pre-fix tree (nothing else in `import.rs` names `Owner`).

## Left out

- `crates/mc-world-conversion/src/pre_rename.rs` (new, 119 lines) — (b) whole module kept: the kept ref
  calls `PreRename::rename`/`current_name` and the pre-rename refusal test drives it directly.
- `crates/mc-world-conversion/src/lib.rs` `@@ -22,6 +22,7 @@` (`mod pre_rename;`) — (b) the module
  declaration the kept ref needs (`pre_rename::tests::...` runs inside it); everything else in
  `lib.rs` is carried above. Reversing the declaration would stop the control from compiling and
  running the ref, which is the class-(b) test, not the false claim earlier made for the field/receipt
  blocks.
- `crates/mc-world-conversion/src/pre_rename_tests.rs` (283 lines) and `roundtrip_tests.rs` (4 lines) —
  test files, kept so the control compiles and runs the registered ref.
- `docs/MIGRATION.md` — documentation.

## Proving ref (the row's own report/rider assertions)

- `cargo:mc-world-conversion:pre_rename::tests::rc5_export_with_a_player_rider_keeps_its_fields_and_the_rider`
  — the test drives the real `import_anvil` over an RC5-shaped world. With the wiring reversed the
  receipt carries no `pre_rename_keys` count and no rename normalization, so
  `pre_rename_tests.rs:418` (`assert_eq!(receipt.counts.get("pre_rename_keys"), Some(&renamed))`) with
  `left: None, right: Some(4)`, class `assertion`. That is the receipt value the row names ("reports no
  gap"): pre-fix the receipt carries no `pre_rename_keys` at all. The test stops at :418, so its later
  assertions (the imported horse's `mount.riders == [player]`, the export's `krasnokamen:PortalTime` and
  `solaris*` absence) are **not** claimed as control evidence. Measured twice: on the reversed worktree
  before the promote (`.analysis/codex-logs/import-control2/run.log`) and by the promote (see below).
  The restore is green.

## Superseded promotion

The first promotion (`patch:00479158…`, artifact `20261125T220349-ledger-promote-fso_8gx6`) carried only
the six calls and left the `lib.rs` field/receipt blocks labelled (b), which was false. It is superseded
by this patch (`patch:166abc98…`) and a fresh promote; its receipt stays as evidence of the same red.
