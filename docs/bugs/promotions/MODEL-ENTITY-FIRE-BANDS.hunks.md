# MODEL-ENTITY-FIRE-BANDS — hunk map for the pinned pair (rebuilt on the current head)

Promotion receipt: `promotions/MODEL-ENTITY-FIRE-BANDS.json` (written by `ledger promote`).
Client pair on the current client head `07d879cd`: control **`client:8f54183f`** (the geometry block
replaced by the real pre-image body) → restore **`client:9308bb29`** (exactly that diff re-applied;
this is the row's `fix_commit`). Both pins are clean (`pins/8f54183f2f75-client-*`,
`pins/9308bb29bf07-client-*`). The pair's production delta is `patch:e37f4b12…`
(`git diff 8f54183f 9308bb29 -- src/entities/presentation.rs | git patch-id --verbatim`).

## Why the pair was rebuilt

The first pair (`client:e017c320` → `client:de93dc7e`, the tip with `a3f2a486`'s patch reversed)
measured span 2.331 on BOTH halves (receipts `20261125T165330/165340/165434` vs
`20261125T165457/165519/165541`): `54ca14f778` had already rewritten the lines `a3f2a486` touched
(rotation source, sprite parity, the `camera_yaw` parameter), so reversing `54ca14f778` reversed
none of the band geometry. That pair also could not run against the HEAD check (its pins emit the
pre-CM-4 string-keyed `entity_layer_geometry`, while `scene_shot_checks.py:309-310` reads the CM-4
list). The rebuilt pair therefore reverses **only** `a3f2a486`'s geometry onto HEAD, and the
control is committed on the branch so each half has its own clean pin — no hand re-break, no
dirty tree.

## The measured pair (2026-11-25, server pin `pins/2bd91efa9122-server-*` = core `2bd91efa9`)

Three takes per side on `native-entity-fire`, all six on the same clean server pin:

| side | receipts | result | span |
|---|---|---|---|
| control `client:8f54183f` | `20261125T175643` / `175710` / `175734` | FAIL at `fire_band_span` on all three | **1.827** (y 160.000..161.827, 12 indices) |
| restore `client:9308bb29` | `20261125T175800` / `175827` / `175854` | PASS on all three | **2.331** (y 160.000..162.331, 12 indices) |

Both numbers are the analytic values derived above, and every other assertion (`fire_layer_present`,
`fire_overlay_is_one_layer`, `fire_band_count`, `plain_frame_has_no_fire_layer`, the fixture camera
and GPU checks) is green on both halves — so the red is the band geometry and nothing else.
`ledger promote MODEL-ENTITY-FIRE-BANDS` receipt `20261125T175952-ledger-promote-oqob4f51`;
`promote.json` records `fix_patch_id e37f4b12…` and the client unit ref green on the restore.

## Original commit `a3f2a486` (`fix(client): draw entity fire with the vanilla flame feature bands`)

Its non-test hunks, each with its HEAD equivalent or omission class:

| # | file `@@` header | what the hunk did | HEAD equivalent | class |
|---|---|---|---|---|
| 1 | `src/entities/presentation.rs` `@@ -636,12 +636,16 @@ fn append_experience_orb` | added the `append_entity_fire` doc paragraph; removed the `camera_yaw` parameter | HEAD has the paragraph (rewritten 2026-11-25 to name `prepare`, the camera yaw and the u mirroring) and the parameter is back as `transform, camera_yaw` (`54ca14f778`) | kept, not reversed: docs and the signature are not the band geometry; the parameter removal is **(c)** — `54ca14f778` re-added it |
| 2 | `@@ -652,30 +656,31 @@ fn append_entity_fire` | body: `facing`→`rotation` by `transform.yaw`; `width`→`unit`; `y`/`half`/`row`→`y_top`/`y_bottom`/`half`/`z`/`band`; quads 1.4 units tall; UV swap; `p * unit` | HEAD carries the same geometry, except `rotation` uses `camera_yaw` (`54ca14f778`) and the sprite is `textures[band % 2]` with `(band / 2) % 2` u mirroring (`54ca14f778`, fixture `234673dd`/`7c366236`) | the control reverses exactly the geometry lines (`unit`/`y_top`/`y_bottom`/`half`/`z`/quad height/step); the rotation source, sprite alternation and mirroring stay HEAD's |
| 3 | `@@ -683,9 +688,11 @@ fn append_entity_fire` | tail: `y_top -= 0.45; y_bottom -= 0.45; z -= 0.03; band += 1` | same lines in HEAD | reversed by the control (`y += width * 0.45; half *= 0.9; band += 1`) |
| 4 | `@@ -790,15 +797,7 @@ pub fn append` | first call site: drop `camera_rotation[0]`, pass `transform` | HEAD passes `transform, camera_rotation[0]` again | **(c)** `54ca14f778` rewrote it; kept |
| 5 | `@@ -872,7 +871,6 @@ pub fn append` | second call site: drop `camera_rotation[0]` | same | **(c)**; kept |
| 6 | `@@ -969,7 +967,6 @@ pub fn append` | third call site: drop `camera_rotation[0]` | same | **(c)**; kept |
| 7 | `@@ -1017,15 +1014,7 @@ pub fn append` | fourth call site: drop `camera_rotation[0]`, pass `transform` | same | **(c)**; kept |
| 8 | `@@ -3743,13 +3732,144 @@ mod tests` | the light-15 assertion change and the new `fire_bands_follow_the_vanilla_flame_feature_numbers` | HEAD's test evolved: 11-band 4.0 x 0.6 fixture (`234673dd`, repaired by `7c366236`) | test hunk, excluded from the non-test patch; present on both sides |

No non-test hunk is class (a) or (b); hunks 1 and 4-7 are **(c)** and stay at HEAD.

## The a/-side text of the pair (control → restore)

Everything the control's a/-side carries is either the real pre-image (`a3f2a486`'s own removed
lines, reproduced verbatim in `e017c320`) or a named kept line:

- `let width = dimensions.width * scale * 1.4;` … `y += width * 0.45;` and the quad/`z` lines are
  `a3f2a486`'s removed body text, byte for byte (including `width * 0.45`, not the `width` step an
  earlier recipe mis-stated: 0.45 * unit is what makes `1.45 * unit = 1.827`).
- `let rotation = Mat4::from_rotation_y((180. - camera_yaw).to_radians());` — HEAD's line
  (`54ca14f778`), kept; a rename of `facing` only.
- `textures[band % 2]` and the `(left, right)` mirroring — HEAD's lines, kept; for bands 0 and 1
  they emit the same UVs as the pre-image, so they add nothing to the delta.
- `band` instead of `row` — counter rename only (HEAD's name).
- The signature `transform: super::Transform, camera_yaw: f32` and the four call sites — HEAD's,
  kept (hunks 1 and 4-7 above are class (c)).

None of these lines is inside the control↔restore diff, so the pair's delta is the band geometry
alone; the control still builds (client pin above) and only its `fire_bands_follow_the_vanilla_flame_feature_numbers`
unit test is red.

## What the scene proves — and what it does not

`native-entity-fire` (`tools/harness/entity_fire_scene.json`) shoots one stationary pig burning and
then plain at a fixed pad and camera. `check_entity_fire` reads the frame's own
`entity_extraction.entity_layer_geometry` (CM-4) and asserts, in order:

- `plain_frame_has_no_fire_layer` — the unlit frame has no `EntityFire` slice;
- `fire_layer_present` — the burning frame has one (green on both halves here; it is
  MODEL-ENTITY-FIRE's assertion, not this row's);
- `fire_overlay_is_one_layer` — one slice, not two;
- `fire_band_count` — 12 indices (2 bands). Both loops give the pig `n = ceil(y_top / 0.45)` =
  2 bands, so this cannot separate the halves either;
- `fire_band_span` — **2.2..2.5 blocks, the row's discriminator.** Derived from the model, not
  fitted: the control stacks `(1 + 0.45) * unit`, the fix `(1.4 + 0.45) * unit`, with
  `unit = bbWidth * scale * 1.4 = 1.26` for the scene pig, so the analytic spans are **1.827** and
  **2.331**; the registered two-sided band sits around the vanilla 2.331. The earlier receipts
  `20261125T122429` / `20261125T123333` measured exactly those numbers on the old pins.

Not witnessed: the sprite parity, the u mirroring and the camera-facing rotation. Both halves
submit a single fire layer and the plane looks the same in both trees, so those stay covered by
`fire_bands_follow_the_vanilla_flame_feature_numbers` (and its 11-band fixture, which exercises
both mirroring branches).
