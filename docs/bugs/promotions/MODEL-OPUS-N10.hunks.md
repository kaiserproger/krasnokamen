# MODEL-OPUS-N10 — hunk map for the committed reversal pair

Promotion: `docs/bugs/promotions/MODEL-OPUS-N10.json` (written by `ledger promote`).
Scene: `native-entity-stray` (`tools/harness/stray_scene.json`), proving assertion
`rig_layer_indices`.

| half | client commit | content |
|---|---|---|
| control | `client:c921de7c` (`wt/D123PP3N10Ctl`) | `git apply -R` of `client:6d4540e3`'s non-test hunks at `ba1971d76215`: `cow.json`, `glow_squid.json`, `mooshroom.json`, `skeleton.json`, `stray.json` |
| restore (= `fix_commit`) | `client:6d4540e3` | the real fix; `git diff c921de7c ba1971d76215` is that commit's whole patch (patch id recorded by `ledger promote`) |

`6d4540e3` carries no test hunk; the row's unit ref
`cow_horn_cube_and_humanoid_hat_follow_the_26_3_layers` comes from `23a158e9` and is red with the fix
reverted (row `repro_check`).

## What is separable and what is not

Measured from the rig JSONs on both sides (six quads per cube, nested bones included):

| rig | pre-image | fix | a frame can separate? |
|---|---|---|---|
| `skeleton` / `stray` | 216 indices, 36 quads | **252 / 42** | **yes** — the hat cube's six quads |
| `cow` / `mooshroom` | 216 / 36 | 216 / 36 | no: the horn is a `mirror` flag (UV winding), no count or bounds change |
| `glow_squid` | 216 / 36 | 216 / 36 | no: a tentacle pivot rounds to 1/16 px and the body gains `inflate: 0.02`, below any stable frame tolerance |

So the row's promotable half is the skeleton/stray hat cube, asserted as the frame's own largest
`EntityCutout` slice. The mob must be a **stray**: `minecraft:skeleton` is a `crowd::KINDS` member and the
GPU crowd renderer draws it, so its frame reports `crowd: 1`, `submitted: 0` and no layer geometry at all
(measured on the same scene before the switch, `20261126T140624-native-entity-skeleton-1y8_2dv0`);
`minecraft:stray` is not in KINDS and carries the same `stray.json`. The cow/mooshroom horn mirror and the glow-squid tentacle yaw stay with the
unit test; the row's `repro_check` already records that test's red-with-revert receipt.
