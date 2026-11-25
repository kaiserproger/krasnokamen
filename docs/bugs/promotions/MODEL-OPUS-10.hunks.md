# MODEL-OPUS-10 — hunk map for the pinned pair

Promotion receipt: `promotions/MODEL-OPUS-10.json` (written by `ledger promote`).
Client pair: control `client:4092ac53` (the fix commit's parent, pinned before this pack) → restore
`client:92dcb25d` (the row's `fix_commit`). Both pins clean, both post-rename.

## What the pair carries

`92dcb25d` ("vanilla bones for the horse family, allay, bee baby, cod and guardians") re-authors ten
rigs. For this row the production parts are:

| file | change | role |
|---|---|---|
| `assets/krasnokamen/models/entity/guardian.json` | 17 cubes / 102 quads → 22 / 132: the body cube becomes a five-cube vanilla head plus twelve spikes on `spike0..spike11`, tails on `tail0..tail2` | the row's credited subject ("guardian spikes ... missing or malformed") |
| `assets/krasnokamen/models/entity/elder_guardian.json` | same rewrite | the same family |
| `assets/krasnokamen/models/entity/bee_baby.json` | 13 cubes / 62 quads → 9 / 49 (three leg fans replace six per-side legs; `left_wing` keeps one face) | the row's "baby bee limbs" |
| `src/entities/krasnokamen_models/tests.rs`, `asset_model_regression_tests.rs`, `animation_tests.rs`, `presentation.rs` | test/code renames to the vanilla bone names | the row's unit refs |

## Hunks carried for other rows (class (a))

| file | disposition |
|---|---|
| `allay.json`, `donkey*`, `mule*`, `horse*`, `skeleton_horse.json`, `zombie_horse.json` | (a) MODEL-OPUS-11's subjects |
| `cod.json` | (a) MODEL-OPUS-9's subject |
| `bee.json`, `hoglin.json` (touched only through the shared test file) | (a) the same family rows |

## What the scene proves

`native-rig-parity` (`tools/harness/rig_parity_scene.json`) shoots one stationary mob per shot at a
fixed pad and camera. `check_rig_parity` asserts `guardian_spikes_and_bee_legs_submitted`: the
guardian's own submitted indices minus the baby bee's, 612-372 = 240 before and 792-294 = 498 after,
floor 350. The difference cancels the per-entity shadow quad, and the count is the frame's own
submission, so host load cannot move it. The hoglin frame is evidence only (its count is unchanged).
