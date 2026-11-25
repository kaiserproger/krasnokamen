# MODEL-ENTITY-FIRE-BANDS — hunk map for the pinned pair

Promotion receipt: `promotions/MODEL-ENTITY-FIRE-BANDS.json` (written by `ledger promote`).
Client pair: control `client:e017c320` (the tip with `a3f2a486`'s patch reversed) → restore
`client:de93dc7e` (the tip). Both pins clean; the CM-4 counter commit is in *both* trees, so the
pair's delta is exactly the fix's production hunk. `fix_commit` on the row is `client:a3f2a486` —
the original, not the re-land — because naming the re-land made the tool's patch-id comparison
compare the commit with itself.

## Why a committed pair

`a3f2a486` is post-rename but is not the tip's parent; its patch still reverses cleanly at the tip,
so the control is that reversal and the re-land restores the tip's blob. `git diff e017c320
de93dc7e` is therefore the fix's own patch; its non-test part must carry the same patch id as
`a3f2a486`'s non-test part (the ledger's own split excludes the `#[cfg(test)]` hunks on both sides).

## Hunks carried by the pair (control -> restore)

| file | hunk | role |
|---|---|---|
| `src/entities/presentation.rs` | `append_entity_fire`: the control's camera-height billboard (`facing`, `unit = bbWidth * scale` height, sprite `row % 2`) becomes the 26.3 bands — `unit = bbWidth * scale * 1.4`, quads 1.4 units tall from `y_top = bbHeight * scale / unit`, `y_bottom` stepping 0.45 so each later band sits higher, half width 0.5 shrinking by 0.9, z from `0.3 - (int)y_top * 0.02` stepping -0.03, and the sprite/UV choice of the day (`(band / 2) % 2`); the call sites pass the camera rotation | the row's subject: the flame feature's band geometry |
| same file, `mod tests` | `fire_bands_follow_the_vanilla_flame_feature_numbers` | excluded from the non-test patch; present in both trees |

No other production path differs, so nothing is class (a) or (b).

## What the scene proves — and what it does not

`native-entity-fire` (`tools/harness/entity_fire_scene.json`) shoots one stationary pig burning and
then plain at a fixed pad and camera. `check_entity_fire` reads the frame's own
`entity_extraction.entity_layer_geometry` (CM-4) and asserts, in this order:

- `plain_frame_has_no_fire_layer` — the unlit frame has no `EntityFire` slice;
- `fire_layer_present` — the burning frame has one (this is the assertion MODEL-ENTITY-FIRE's title
  needs; the control here draws an overlay, so it is green on both halves and proves nothing for
  that row);
- `fire_overlay_is_one_layer` — one slice, not two (renamed from `fire_sprite_pairs_two_bands`,
  which passed on the control because both sprites share one texture layer);
- `fire_band_count` — 12 indices;
- `fire_band_span` — 2.2..2.5 blocks: 1.85 scaled units at the pig's 1.26 scale = 2.331, where the
  pre-fix loop reaches 1.827. **This is the row's discriminator.**

Not witnessed here: the sprite parity, the u mirroring and the camera-facing rotation. Both halves
submit a single fire layer, and the plane looks the same in both trees, so those three are covered
by the unit test alone. The 2026-11-25 client fix corrected all three against `javap` on the local
26.3 jar (sprite `band % 2` with u mirrored when `(band / 2) % 2 == 0`; rotation
`Mth.rotationAroundAxis(Y_AXIS, camera.orientation)`), which is why the hunk above describes the
older `(band / 2) % 2` sprite choice: the pair was cut before that correction and must be re-run on
the pinned tree that carries both it and this row.
