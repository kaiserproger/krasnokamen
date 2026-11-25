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
| `native_checkpoint.rs @@ -534,8 +534,8 @@` `open_refusal` message | `crates/mc-core/src/native_checkpoint.rs:604-612` | the refusal text telling operators the import now renames `solaris:*`; carried so no hunk is left out without a class |

The HEAD adaptation carried here (named per README:234-258): the call sites use the module's `Owner`
argument and the file's `use super::pre_rename::Owner;` line, which a later commit introduced when it
gave `rename` an explicit owner. The a-side of the patch removes that import together with the calls,
exactly the pre-fix tree (nothing else in `import.rs` names `Owner`).

## Left out

- `crates/mc-world-conversion/src/pre_rename.rs` (new, 119 lines) — (b) whole module kept: the kept ref
  calls `PreRename::rename`/`current_name` and the pre-rename refusal test drives it directly.
- `crates/mc-world-conversion/src/lib.rs` (`mod pre_rename;`, `ConversionStats.pre_rename`, the
  `pre_rename_keys` count and `PRE-RENAME VALUES` gap blocks) — (b) module declaration, field and
  receipt surface the kept refs read; reversing them stops the control from compiling.
- `crates/mc-world-conversion/src/pre_rename_tests.rs` (283 lines) and `roundtrip_tests.rs` (4 lines) —
  test files, kept so the control compiles and runs the registered ref.
- `docs/MIGRATION.md` — documentation.

## Proving ref (the row's own report/rider assertions)

- `cargo:mc-world-conversion:pre_rename::tests::rc5_export_with_a_player_rider_keeps_its_fields_and_the_rider`
  — the test drives the real `import_anvil` over an RC5-shaped world. With the wiring reversed the
  receipt carries no `pre_rename_keys` count and no rename normalization, so
  `pre_rename_tests.rs:418` (`assert_eq!(receipt.counts.get("pre_rename_keys"), Some(&renamed))`) fails
  first; the same test's later assertions (the imported horse's `mount.riders == [player]`,
  `mentions_pre_rename(exported_player)` false, `krasnokamen:PortalTime` present, `solaris*` absent)
  encode the row's symptom and cannot pass on the control either.
