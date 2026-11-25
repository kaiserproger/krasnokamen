# MODEL-ENTITY-FIRE — hunk map for the pinned pair

Promotion receipt: `promotions/MODEL-ENTITY-FIRE.json` (written by `ledger promote`).
Client pair: control `client:e017c320` (the tip with `a3f2a486`'s patch reversed) → restore
`client:de93dc7e` (the tip). Both pins clean; the CM-4 counter commit `b7a31e8d` is in *both* trees,
so the pair's delta is exactly the fix's production hunk.

## Why a committed pair

`a3f2a486` is post-rename but is not the tip's parent; its patch still reverses cleanly at the tip
(`git apply -R --check` with the commit's own diff), so the control is that reversal and the re-land
restores the tip's blob. `git diff e017c320 de93dc7e` is therefore the fix's own patch; its
non-test part has the same patch id as `non_test_patch(f2c3a0f8…)`, i.e. the `append_entity_fire`
rewrite and the three call-site changes, with the inline `#[cfg(test)]` hunks excluded on both
sides by the ledger's own split.

## Hunks carried by the pair (control -> restore)

| file | hunk | role |
|---|---|---|
| `src/entities/presentation.rs` | `append_entity_fire`: the camera-yaw parameter and the billboard `facing` are removed, the overlay becomes `Mat4::from_rotation_y(180 - transform.yaw)`, quads are `unit = bbWidth * scale * 1.4` tall from `y_top = bbHeight * scale / unit` stepping `y_bottom` down by 0.45, half width 0.5 shrinking by 0.9, z from `0.3 - (int)y_top * 0.02` stepping -0.03, sprite `(band / 2) % 2`, UVs mirrored, and the three call sites lose the `camera_rotation[0]` argument | the row's subject: the 26.3 flame feature bands |
| same file, `mod tests` | `fire_bands_follow_the_vanilla_flame_feature_numbers` | excluded from the non-test patch; it stays in both trees and is green on the candidate |

No other production path differs, so nothing is class (a) or (b).

## What the scene proves

`native-entity-fire` (`tools/harness/entity_fire_scene.json`) shoots one stationary pig burning and
then plain at a fixed pad and camera. `check_entity_fire` reads the frame's own
`entity_extraction.entity_layer_geometry` (CM-4): the burning frame must have exactly one
`EntityFire#…` slice carrying all 12 indices (`fire_sprite_pairs_two_bands` — the pre-fix client
splits 6/6 over both sprites), a band span of at least 2.0 blocks, and a bottom at least 0.25 blocks
below the pig's own body bottom (`fire_band_below_the_entity` — 26.3's second band steps down while
the pre-fix overlay starts at the feet); the plain frame must have no fire slice.
