# MODEL-OPUS-10 — hunk map for the pinned pair

Promotion receipt: `promotions/MODEL-OPUS-10.json` (written by `ledger promote`).
Client pair actually used: control `client:e3678206` (the rig fix reversed at the tip) → fix
`client:3bfbd7b6` (the re-land; `92dcb25d` is its ancestor). Both pins clean and built from the
commits themselves: `pins/e3678206dc82-client-*` and `pins/3bfbd7b6902c-client-*`.

## What the pair carries (18 files)

`git diff e3678206 3bfbd7b6 --stat` = 14 rig JSONs under `assets/krasnokamen/models/entity/` plus
four `src/` files whose hunks are all inside `mod tests`. The non-test hunks are the fourteen JSON
rewrites; the per-file patch ids of every `src/` hunk equal `92dcb25d`'s own (verified 2026-11-25
with `git patch-id --stable`: `6a4e388c…`, `65b03181…`, `c643ca6c…`, `9e8a2ab9…`), so the pair
differs from the original fix by nothing but the rig data.

| file | hunk / change | disposition |
|---|---|---|
| `guardian.json` | whole file: 17 cubes / 102 quads → 22 / 132; the 12x12x12 body becomes the dump's five-cube head (`[-6,10,-8]` 12x12x16, two side fins with one mirrored, two 12x2x12 plates), twelve `spike0..spike11` on the dump's pivots with `[-1,-4.5,-1]` size `[2,9,2]`, `eye`, `tail0..tail2` with the dump's `[1,10.5,3]` 1x9x9 fin | **this row** — the credited subject (guardian spikes) |
| `elder_guardian.json` | the same rewrite | this row (same family, no shot) |
| `bee_baby.json` | whole file: 13 cubes / 78 quads → 9 / 54 (three leg fans instead of six per-side legs; `left_wing` keeps one face) | **this row** — "baby bee limbs" |
| `allay.json` | whole file: 34 → 42 quads | (a) MODEL-OPUS-11 |
| `horse.json`, `horse_body_equipment.json`, `horse_saddle_equipment.json`, `donkey.json`, `donkey_saddle_equipment.json`, `mule.json`, `mule_saddle_equipment.json`, `skeleton_horse.json`, `zombie_horse.json` | whole files | (a) MODEL-OPUS-11-HORSES |
| `cod.json` | whole file: 30 lines | (a) MODEL-OPUS-9 |
| `src/entities/presentation.rs` | one hunk, `@@ -4290,9 +4290,25 @@ mod tests` — a rig-summary test | test hunk, carried (not a production change) |
| `src/entities/asset_model_regression_tests.rs` | `@@ -690,10 +690,12 @@`, `@@ -701,11 +703,11 @@`, `@@ -717,11 +719,11 @@` — bone-name expectations | test hunk |
| `src/entities/krasnokamen_models/tests.rs` | `@@ -800,14 +800,16 @@`, `@@ -898,8 +900,14 @@`, `@@ -916,14 +924,24 @@` | test hunk |
| `src/entities/animation_tests.rs` | `@@ -2462,15 +2462,15 @@` | test hunk |

Nothing else: there is no production hunks outside the rig JSONs, so nothing is class (b) or (c).

## The `tail` alias at `presentation.rs:2115`

The alias `"tail" | "tail_tip" | "tail0" | "tail1" | "tail2"` is **not** a hunk of this pair. It
arrived with `83994480` ("feat(client): integrate ClientNext slices on RC5", found with
`git log -S'"tail" | "tail_tip"'`), and it exists on both sides of the pair. It is the renderer's
bone-name dispatch: it lets the pre-image's `tail` / `tail_tip` names and the fix's `tail0..tail2`
names both take the guardian's tail sway, so the pair does not need it and the fix does not carry
it. Disposition: class (a)-adjacent — another commit's code, present in both halves, no adaptation.

## What the scene proves

`native-rig-guardian` (`tools/harness/rig_guardian_scene.json`) shoots the empty pad, one
`NoAI:1b` guardian and one `Age:-24000` bee at a fixed camera and asserts one thing per frame
(`guardian_single_entity`, `bee_single_entity`, `*_rig_rendered`) before the row's assertion
`guardian_spikes_and_bee_legs_submitted`:

- guardian `EntityCutout` = 792 indices (22 cubes × 6 faces × 6 indices), bee = 324 (9) — the
  pre-image reads 612 (17 cubes) and 468 (13), from the same `entity_indices_by_material` slice;
- the guardian's own cutout bounding height in 1.44..1.53 blocks: 1.4912 derives from the dump (the
  twelve spikes reach y 4.03..27.89 px), the pre-image's shorter spikes read 1.3761 and a rig with
  no spikes 1.0. A lateral term was tried and rejected: the pre-image reaches x = 1.5017 against the
  fix's 1.4841, so the width cannot witness the spikes.

The 78/54 (and 62/49) counts the older text used are the same rigs counted in quads, not cubes:
`krasnokamen_models.rs:265-283`'s `faces` map only overrides UVs, all six faces are always emitted.

## The pair that promoted it

The pair that actually promoted the row ran on server `pins/d4a60da3dd81-server-*` (core
`d4a60da3dd`), receipts `20261125T154503-native-rig-guardian-bejkavea` (control, FAIL at
`guardian_spikes_and_bee_legs_submitted`: guardian 612, bee 468, height **1.369**) and
`20261125T154546-native-rig-guardian-o7decd_1` (fix, PASS: 792, 324, height **1.491**). The earlier
pair on `c20a0d18bee5` (`20261125T152848` / `20261125T152915`, the latter height 1.516) proved the
same thing on a head the promote then refused because a docs commit had moved it.

Three different ids describe this pair and they are not interchangeable:

| id | what it is |
|---|---|
| `0622414eba9c…` | `client_control_delta.fix_patch_id` in `promotions/MODEL-OPUS-10.json`, computed verbatim by `ledger promote` (`ledger.py:1913-1931`) over the two pinned client heads |
| `patch:d4787b5017…` | the non-test digest of the fix commit `3bfbd7b6` AND of `92dcb25d` (both computed with the ledger's own non-test split: 14 files, 4 test hunks) - they are equal, which is what makes the rig re-land the same fix as `92dcb25d` |
| `6cf9f2be…` | `git diff bafbf78b40 0eb3e0ea1e -- assets/ \| git patch-id --stable`, i.e. the plain patch-id of the fourteen rig files only |

No "within 0.3%" claim: the measured fix height 1.491 versus the dump's 1.4912 happens to agree, but
the value is not stable run to run (1.491 at `154546` against 1.516 at `152915` on the same pin,
because the client's spike breathing scales the absolute pivot where vanilla breathes about the head
centre). The window 1.44..1.53 is the analytic envelope for that breathing, and the row records that
a variable gate wants at least three fix-side runs.
