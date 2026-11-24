# DATAPACK-WORLDGEN-OVERRIDES: patch-mode fix identity

Original fix: `1d8e57dd8` "fix(core): new datapacks never stop a start or a
reload". `fix_commit = "patch:d4c9b9ebd0d80b85ef87099692dd633c50880e2b99777c468d6e525a483af70c"`
names the reversible part of that fix on the candidate in
`docs/bugs/promotions/DATAPACK-WORLDGEN-OVERRIDES.patch` (a diff from the
pre-fix form to the candidate; `git apply -R` restores the pre-fix code).
Re-audit:
`python3 -m tools.harness ledger promote DATAPACK-WORLDGEN-OVERRIDES --fix-patch docs/bugs/promotions/DATAPACK-WORLDGEN-OVERRIDES.patch --dry-run`

## Why not the commit itself

Reversing the commit's non-test hunks on the tip does not compile:

```
error[E0425]: cannot find function `open` in module `crate::datapacks::worldgen`
   --> crates/mc-core/src/native_checkpoint/game_clock_upgrade.rs:276:46
```

`251276077` ("world: explicitly convert legacy frozen clocks on disposable
copies") calls `worldgen::open` — added by this fix — with seven arguments, and
its call site is outside the fix's file set. A compile failure is not a red
control (docs/bugs/README.md:203-208), so the identity carries only the part
whose reversal keeps the tree building.

## Carried (2 files, 2 hunks)

| original | HEAD |
|---|---|
| datapacks.rs -719,11 `unsupported` collection and per-pack refusal naming the entries (first eight plus a count) | datapacks.rs:810-838 |
| datapacks_worldgen.rs -930,6 `open`: `inputs.validate(registry)` before a pack is admitted | datapacks_worldgen.rs:986-989 |

The a/-side of these two hunks is the tip's own text with those two pieces
removed, so the tree still builds and the *registered tests are byte-identical*
on both sides: nothing is invented on the a/-side.

## Excluded (not reversed at all)

- `datapacks.rs::PackSet::open`'s generic `admit` closure and the doc rewrite,
  `datapacks_worldgen.rs::open`/`Opened`, `core/recovery.rs` and
  `native_checkpoint.rs` — class (b): `251276077`'s call site reads the new
  `worldgen::open` signature, so keeping it is what lets the control build. The
  a/-side therefore still *has* the trialling plumbing; only the two pieces that
  make unsupported worldgen refuse-or-name are gone.
- `datapacks_worldgen.rs::validate_resource`'s `ResourceError` enum — class (b):
  `PackSet::load` (still on the a/-side) matches on it.

## Behaviour of the a/-side, and the control

- A selected pack with an unsupported worldgen/dimension entry no longer
  refuses: the entry is ignored and the pack loads.
- `datapacks::worldgen::tests::unsupported_worldgen_refusal_names_every_entry_and_supported_packs_load`
  is **red by assertion** there (`datapacks_worldgen_tests.rs:453`,
  `Option::unwrap()` on `None`: `PackSet::load(...).err()` returned `None`).
  Observed: `cargo nextest -p mc-core` with both row refs in the re-break
  worktree — 1 failed by assertion, 1 passed.
- `start_skips_a_new_worldgen_pack_it_cannot_prepare_and_loads_the_rest`
  stays **green** in the control: it exercises the fix's prepare-failure path
  (`prepare_worldgen`), which is outside the reversible part. It is recorded as
  a green regression on the candidate, not as control evidence.
- Restore: `native-v2-datapack-reload` passed on the candidate pin
  `d4c3449e6448-server-x86_64-unknown-linux-gnu-incr`
  (`.analysis/validation/20261124T030444-native-datapack-reload-pvejghdn`,
  server binary `ecbea02267b8a505…`).
