# MODEL-ENTITY-FIRE — hunk map for the dispatch-deletion pair

Promotion receipt: `promotions/MODEL-ENTITY-FIRE.json` (written by `ledger promote`).
Client pair on the client head `234673dd`: control **`client:6608f21c`** (the four
`append_entity_fire` calls AND the function deleted; the match arm loses its unused binding) →
restore **`client:07d879cd`** (exactly that diff re-added), which is the row's `fix_commit`. Clean pins
`pins/6608f21c3d95-client-*` / `pins/07d879cd6732-client-*`; server `pins/38d60b3bcd9b-server-*`
(core `38d60b3bcd`).

## Provenance (corrected in review round 3)

The overlay did **not** come in with `83994480`. `git blame -C -C 83994480^` resolves the inline
`DATA_SHARED_FLAGS_ID` block that `83994480` removed to **`client:43362918`** ("Integrate Solaris
client rc4 candidate", 2026-11-03):

| commit | role in the row's path |
|---|---|
| `client:43362918` | **origin of the fix**: adds the inline `EntityFire` overlay on the ordinary-rig path (`git blame -C -C 83994480^`, the block `let flags = number(&models, entity, "DATA_SHARED_FLAGS_ID")` …) — before it, a burning entity draws no fire layer |
| `client:83994480` | MOVES that block into `fn append_entity_fire` and ADDS the experience-orb and special-renderer calls; it does not introduce the overlay |
| `client:a89b5664` | GPU crowd instancing: adds the fourth (crowd) call |
| `client:a3f2a486` | rewrites the band GEOMETRY inside the function (MODEL-ENTITY-FIRE-BANDS) |
| `client:54ca14f778` | refines the rotation source and the sprite parity inside the function (unit-covered) |

Because these are bulk commits whose other hunks belong to other rows, no existing commit equals a
dispatch-deletion control's delta and `_check_client_control_delta` (`ledger.py:1917-1931`) would
refuse a pair against any of them. The pair is therefore the committed reversal this row records: the
control deletes every fire call and the function, the restore re-lands exactly that, and the restore is
the `fix_commit`.

## Hunks the reversal carries (control → restore)

| # | commit | file `@@` header | what the hunk did | HEAD equivalent | class |
|---|---|---|---|---|---|
| 1 | `43362918` | `src/entities/presentation.rs` `@@ -460,11 +637,95 @@ pub fn append` | the inline overlay: `flags & 1` test, `facing` by camera yaw, the `width`/`half`/`y`/`z` loop on the rig path | the same body lives in `fn append_entity_fire` (extracted by `83994480`, geometry rewritten by `a3f2a486`, rotation/sprite by `54ca14f778`); the control deletes the function and the rig-path call | carried |
| 2 | `83994480` | `@@ -494,6 +527,48 @@ fn append_experience_orb` | adds `fn append_entity_fire` (pre-`a3f2a486` geometry) | `presentation.rs:786` (the function, with the 2026-11-25 geometry, rotation and sprite) | carried |
| 3 | `83994480` | `@@ -501,7 +576,7 @@ pub fn append` | adds `maps, tint_source` to the frame destructuring | HEAD's destructuring has them | **(a)** another area's behaviour (special renderers/block entities); kept, not reversed |
| 4 | `83994480` | `@@ -564,6 +639,7 @@ pub fn append` | the experience-orb fire call | HEAD's orb call at `presentation.rs:949` | carried |
| 5 | `83994480` | `@@ -573,29 +649,28 @@ pub fn append` | (i) the item-context refactor, (ii) the `handled @` binding plus the special-renderer fire call | HEAD has the item-context refactor (other area) and, at `presentation.rs:1026`, the fire call inside `if matches!(handled, Handled::Rendered)` | **split**: (i) is **(a)** and kept; (ii)'s call and the `handled @` binding are carried |
| 6 | `83994480` | `@@ -660,49 +735,8 @@ pub fn append` | removes the inline block and calls `append_entity_fire` on the rig path | HEAD's rig-path call at `presentation.rs:1176`, inside `if entity.binding…is_ok()` (`54ca14f778` dropped the `flags` test there because the function tests it again) | carried (the call is deleted; the `if entity.binding` guard is HEAD's and stays) |
| 7 | `a89b5664` | `@@ -745,6 +773,168 @@ pub fn append` | adds the crowd fire call | HEAD's crowd call at `presentation.rs:1123` | carried |
| 8 | `a3f2a486` / `54ca14f778` / `234673dd` | the function body and its test | geometry, rotation, sprite parity, the test fixture | HEAD's function and `fire_bands_follow_the_vanilla_flame_feature_numbers` | **(c)**/**test**: the body is deleted wholesale by the control, so its internal rewrite does not enter the delta |

No non-test hunk of the row's path is class (b). Hunk 3 is the only omission, and it is class (a).

## The a/-side text of the pair

The control carries two a/-side lines that are not deletions:

1. the match pattern loses its binding — `Ok(super::render_special::Handled::Rendered |
   super::render_special::Handled::NoGeometry)` — which is the REAL pre-`83994480` text
   (`83994480`'s own `-` side, reproduced at `/tmp/e8.diff:1669`), not an invented arm;
2. a blank line where the rig-path `if entity.binding…` block was (the review's "blank a/-side
   line"); it changes no behaviour.

The restore re-adds exactly these two lines plus the five call/function pieces above, so
`diff(control, restore)` is the row's path and `promote.json` records the pair's non-test patch id.

## Measured pair (2026-11-25, clean server pin `pins/38d60b3bcd9b-server-*` = core `38d60b3bcd`)

Three takes per side on `native-entity-fire`, all six on the same clean pin:

| side | receipts | result |
|---|---|---|
| control `client:6608f21c` | `20261125T172859` / `172924` / `172949` | FAIL at `fire_layer_present` on all three (fire layer `[]`) |
| restore `client:07d879cd` | `20261125T173014` / `173045` / `173119` | PASS on all three (one `EntityFire` layer, 12 indices) |

`ledger promote MODEL-ENTITY-FIRE` receipt `20261125T173150-ledger-promote-gs_g1cu7`.

**Round-3 re-take** on the clean server pin `pins/38fe67d13d1a-server-*` (core `38fe67d13`, the
tree with the exact fire assertions): control `20261125T191116` FAIL at `fire_layer_present` (no fire
layer), restore `20261125T191141` PASS with span 2.331 and the lowest fire vertex on the body's bbox
bottom (160.000 vs 160.001) — so the tightened `|span - 2.331| <= 0.01` and
`fire_bands_start_at_the_feet` are green on the restore and the control is still red on the row's own
ref. A `fixed` row cannot be re-promoted (`ledger promote` needs `status = "open"`), so this is
recorded evidence only; the promotion above stands. An earlier
dirty-pin attempt (`172108` / `172137` / `172205` red, `172234` / `172308` / `172337` green) is not
part of the promotion; it was re-taken clean because `ledger promote` refuses a dirty pin.

## What the pair proves — and what it does not

`native-entity-fire` reads the frame's own `entity_extraction.entity_layer_geometry`. The control
submits no `EntityFire` slice, the restore submits one, so `fire_layer_present` is exactly the row's
symptom: a client that does not dispatch the overlay. The band geometry is MODEL-ENTITY-FIRE-BANDS'
assertion; the sprite parity, the u mirroring and the camera-facing rotation stay unit-covered
(`fire_bands_follow_the_vanilla_flame_feature_numbers`).
