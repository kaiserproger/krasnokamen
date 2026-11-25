# MODEL-SHIELD-GUI — hunk map for the pinned pair

Promotion receipt: `promotions/MODEL-SHIELD-GUI.json` (written by `ledger promote`).
Client pair: control `client:d1eb6064` (the lane tip `980b8618` with the shield rig put back to its
pre-fix blob) → restore `323f801c` (the same tip, restored). Both pins clean.

## Why a committed pair, and what it carries

The row's real fix is `client:f2c3a0f8` ("author the shield rig as the vanilla 26.3 ShieldModel",
2026-11-15). It is post-rename, but it is not the parent of the lane tip and the tip's neighbour
commits are unrelated, so the pair is built at the tip: one commit puts the *real pre-image blob*
(a byte-identical copy of `c53b0d40:assets/krasnokamen/models/entity/shield.json`) into the tree and
the next restores the fix's blob. `git diff d1eb6064 323f801c` is therefore exactly the fix's
production hunk, which is what `ledger promote` checks by patch id.

## Hunks carried by the pair (control -> restore)

| file | hunk | role |
|---|---|---|
| `assets/krasnokamen/models/entity/shield.json` | the whole rig file: `plate` cube `[-5,-8,-0.5]` size `[10,16,1]` uv `[0,6]` → `[-6,-11,-2]` size `[12,22,1]` uv `[0,0]`; `grip` with two cubes → vanilla `handle` cube `[-1,-3,-1]` size `[2,6,6]` uv `[26,0]` | the row's subject: the plate the GUI cell draws |

No other production path differs between the pair, so nothing is class (a) or (b).

## The fix commit's test hunk (not carried)

`client:f2c3a0f8` also adds `src/entities/asset_model_regression_tests.rs::shield_plate_and_handle_match_the_vanilla_263_model`.
It stays in both trees (the pair does not touch it), so on the control client it is red
(`39 is shorter than 40 px`-class failure at the authored layer) and on the restore green; the
promotion's red control is the scene ref, and the unit ref is the extra evidence.

## What the scene proves

`native-shield-gui` (`tools/harness/shield_gui_scene.json`) shoots an empty hotbar and then a shield
in slot 0 beside a paper in slot 1 at one camera, one window (1280x720) and GUI scale 3, so the
icon rect is a fixed 16x16 GUI px. The check measures the shield icon's own bounding box in the
first cell against the empty frame: the `item/shield` gui transform (rotation `[15,-25,-5]`, scale
`0.65`) draws 26.3's 12x22 plate at 46x25 px and the pre-fix 10x16 plate at 34x21 px (measured on
both pinned clients, 2026-11-25). The assertion floor is 40 px tall (13% below the candidate, 18%
above the control); the paper in the second cell is the fixture check that the item pipeline drew.
