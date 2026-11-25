# MODEL-ENTITY-LEASH — hunk map for the pinned pair

Promotion receipt: `promotions/MODEL-ENTITY-LEASH.json` (written by `ledger promote`).
Client pair on the current head `9308bb29`: control **`client:df888507`** (the leash dispatch
deleted) → restore **`client:03c4675b`** (exactly that diff re-applied; the row's `fix_commit`).
Both pins are clean (`pins/df888507b40f-client-*`, `pins/03c4675b57e6-client-*`). A branch
`leash-control` on the lane head carries the pair; the older control `62ca1943` is preserved as
`leash-control-old` because its red/green pair predates the current head.

## Why this is a committed reversal and not a `git apply -R`

The rope producer was **created** by `client:e8fe40c2` in
`src/entities/render_special/world_fx.rs` (that path does not exist at `e8fe40c2^`), so there is no
pre-image blob to reverse: `ledger promote` patch mode would have nothing to apply. The faithful
control is the deletion of the dispatch, exactly the shape the README allows for a client row whose
fix predates the rename; the restore re-lands the deletion's reverse and becomes the `fix_commit`.

## Real commit `e8fe40c2` (`fix(models): hand-drawn rigs and missing entity visuals …`) — leash part

| # | file `@@` header | what it did | HEAD equivalent | class |
|---|---|---|---|---|
| 1 | `src/entities/render_special.rs` `@@ -6,6 +6,7 @@ mod kernels;` | added `mod world_fx;` | the module moved to `src/entities/render_special/world_fx.rs` in a later refactor; HEAD declares it at `render_special.rs:9` | **(c)** rewritten by a later commit; the module stays (kept-helper) |
| 2 | `@@ -54,6 +55,7 @@ pub(crate) struct Cache` | added `tnt_state: Option<u32>` | HEAD's `Cache` has it | **(a)** TNT row's behaviour (`append_block` resolves the default primed state through it); reversing it would flip the TNT row, so it stays |
| 3 | `@@ -146,13 +148,26 @@ pub(crate) fn append(` | widened the top guard to TNT/falling block/lightning and leashed entities; added the `lightning_bolt` dispatch, the leash dispatch and the TNT/falling-block dispatch | HEAD has the same guard and dispatches (`append_bolt` takes `transform`, `append_block` takes the tint/environment arguments; `54ca14f778` and later commits added parameters) | **split**: the leash dispatch is reversed by the control; the lightning and TNT/falling-block dispatches are **(a)** (other rows) and stay |
| 4 | `src/entities/render_special/world_fx.rs` (new file, 391 lines) | the whole authored module: `tnt_appearance`, `append_block`, `append_bolt`, `append_leash`, `rope_point`, `leash_anchors`, `tube_segment` | HEAD's `render_special/world_fx.rs` (same functions, later arguments) | **(a)** + kept-helper: the module also draws TNT/falling blocks and lightning, so deleting it would turn the TNT row red; it stays in the control with `append_leash` simply unreferenced (its only caller is the carried hunk of row 3) |
| 5 | `src/entities/render_special/world_fx_tests.rs` (new file) | tests | HEAD's tests | test hunk, excluded; present on both sides |

No non-test hunk is class (b), and the only kept-helper unreferenced by the control is
`append_leash` (all of its calls are in hunk 3).

## The a/-side text of the control

The control replaces the leash dispatch with the pre-image's own early return:

```rust
// CONTROL (D123 MODEL-ENTITY-LEASH): the rope dispatch is deleted, so a leashed mob
// falls through to its own rig exactly as before the fix; `append_leash` stays in
// world_fx as a kept helper. `dead_code` is allowed crate-wide (src/lib.rs).
return Ok(Handled::Unhandled);
```

That `Ok(Handled::Unhandled)` is the function's pre-e8fe40c2 outcome for a leashed cow: before the
fix the top guard did not admit leashed entities, so `append` returned `Unhandled` at the top and the
ordinary rig pass drew the mob. Deleting the whole `if` block instead would let a cow fall into the
lighting/`item_frame` tail, which the pre-image never did — so this is the faithful reversal, not an
invented stub. `append_leash` stays as the accepted kept helper, dead because its only caller is the
reversed hunk; `dead_code` is allowed crate-wide (`src/lib.rs:25`).

## The scene, the assertion and the threshold

`native-entity-visuals` (`tools/harness/entity_visuals_scene.json`) summons a sheep at
`(998.5, 160, 1004.5)` and a cow at `(1002.5, 160, 1004.5)` leashed to it, then shoots `leashed` from
`(1000.5, 161.62, 1000.5)` (yaw 0, pitch 0, fov 70, 1280x720). `check_entity_visuals` counts the
rope's own colour in the ROI `(560, 400, 760, 500)` and asserts `leash_rope_drawn: count > 300`.

The threshold is **derived from the model and registered before the rebuilt run** (the comment at
`scene_shot_checks.py:160-167` and the row's `repro_check` carry the same derivation):

- the rope is `tube_segment(..., 0.018, ...)` — a 0.036-block-diameter tube on the anchor-to-anchor
  line; `leash_anchors` puts the cow's end at `y = 160 + 1.4 * 0.7`, `z = 1004.5 + 0.45` and the
  sheep's at `y = 160 + 1.3 * 0.7`, `z = 1004.5 + 0.45`, i.e. a 4.03-block horizontal gap;
- `rope_point`'s sag `4 * clamp(horizontal * 0.1, 0.08, 0.75) * t * (1 - t)` caps at 0.403 blocks, so
  the rope's mid sits at `y ≈ 160.54` and the whole visible stretch is `y ≈ 470..480` on screen,
  inside the ROI;
- at the rope's 4.7..4.9-block depth one block projects to `360 / (depth * tan(35°)) = 105..109 px`,
  so the tube is 3.8..4.0 px wide; the ROI's ±0.125 ndc spans `0.125 * tan(hfov/2) * depth =
  0.125 * 1.2448 * 4.8 ≈ 0.75` blocks either side, i.e. ~1.5 blocks ≈ 160 px of rope;
- the model's lower bound for a drawn rope inside the ROI is therefore ~160 × 3.8 ≈ **600 px**;
  `> 300` demands about half of one visible crossing, which no rig, shadow or sky can supply. The
  earlier pair measured 0 px on the control and 1041 px on the fix (receipts `20261125T140748` /
  `20261125T140830` on the pre-current head).

## What the scene does not prove

The rope's colour and length only: its texel parity, the sag curve against vanilla's
`LeashFeatureRenderer` and the knot's own rig stay unit-covered (`world_fx_tests.rs`), and the ROI
is one camera and one gap.
