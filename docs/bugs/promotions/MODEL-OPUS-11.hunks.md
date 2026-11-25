# MODEL-OPUS-11 — hunk map for the pinned pair

Promotion receipt: `promotions/MODEL-OPUS-11.json` (written by `ledger promote`).
Client pair: control `client:4092ac53` (the fix commit's parent) → restore `client:92dcb25d`. Both
pins clean, both post-rename.

## What the pair carries

`92dcb25d` re-authors ten rigs to the vanilla 26.3 layer geometry. For this row:

| file | change | role |
|---|---|---|
| `assets/krasnokamen/models/entity/allay.json` | 34 quads → 42: the wing cubes gain their faces (the pre-fix rig restricted `left_wing` to one face) and the head is reached through the layer root | the row's first subject ("allay ... incorrect silhouettes") |
| `assets/krasnokamen/models/entity/{donkey,mule,horse,skeleton_horse,zombie_horse}.json` + the three `*_equipment.json` overlays | re-authored to the vanilla equine bones, with the 26.1 `MeshTransformer` scale folded into the top bones | the row's horse family |
| `src/entities/presentation.rs`, `krasnokamen_models/tests.rs`, `asset_model_regression_tests.rs`, `animation_tests.rs` | bone renames and the migrated assertions | the row's unit refs |

## Hunks carried for other rows (class (a))

| file | disposition |
|---|---|
| `guardian.json`, `elder_guardian.json` | (a) MODEL-OPUS-10's subject |
| `bee_baby.json`, `cod.json` | (a) MODEL-OPUS-10 / MODEL-OPUS-9 subjects |

## What the scene proves

`native-rig-parity`'s `allay_wing_faces_submitted`: the allay's submitted indices minus the vex's in
the same run, 204-252 = -48 before and 252-252 = 0 after, floor -20. The vex's rig is unchanged by
the commit, so the difference isolates the allay's added faces, and the per-entity shadow quad
cancels. Limits recorded in the scenario entry: the rabbit/fox/panda/vex silhouettes (counts
unchanged) and the crowd-rendered horse family keep their unit refs; the horse family has no CPU
indices to assert.
