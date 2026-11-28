# DATAPACK-NEW-PACK-WORLDGEN-SKIP — control on bbf5b71e5

Fix identity `patch:7cbe9205537a5db15a7239da475c757211071d08c1c50173a76fa8847613b51b`
(`docs/bugs/fix-patches/DATAPACK-NEW-PACK-WORLDGEN-SKIP.patch`, one production
file). The patch keeps `PackSet::open`'s signature and turns the caller trial's
start refusal into the skip-and-name record; reversing it restores
`Err(error) => return Err(error)`.

- `git apply -R --check` passes at `bbf5b71e5`.
- RED (prior tree, same refusal): `20261126T132012-test-h800hvvo` — the start
  refuses with `Worldgen("Datapack worldgen requires the vanilla gameplay cache")`;
  the strengthened assertion reports the same refusal at
  `datapacks_worldgen_tests.rs:424`.
- RED on `bbf5b71e5`: `.analysis/validation/20261128T092226-test-wdggxjsd` FAIL,
  exit 100, the row's own assertion at `datapacks_worldgen_tests.rs:422`,
  `Some(Worldgen("Datapack worldgen requires the vanilla gameplay cache"))` —
  the start refusal instead of the skip-and-name.
- GREEN: `.analysis/validation/20261128T092429-test-07wzg06v` PASS on this tree.

## Handoff: returned (proof shape)

The row is `repro = manual` and the ledger requires a graphical/real-client
check for such rows, while this is a server-start path with no scene ref. That
is the POL-2 / MECH-13 proof-shape decision for Main; the control itself is
complete and the patch is committed.
