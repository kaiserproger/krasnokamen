# MODEL-ENTITY-LEASH — hunk map for the pinned pair (pure deletion, round 3)

Promotion receipt: `promotions/MODEL-ENTITY-LEASH.json` (written by `ledger promote`).
Client pair: control **`client:fd4d1f5b`** (the guard's leash clauses AND the dispatch deleted, nothing
inserted) → restore **`client:a8c18847`** (exactly that diff re-applied; the row's `fix_commit`). Clean
pins `pins/fd4d1f5b6c94-client-*` / `pins/a8c18847ff06-client-*`.

Review round 3 (`receipts/D123/D123ClientModel/REVIEW-CODE-3-Opus.md` section 3) rejected the first
rebuild `client:df888507`: it replaced the dispatch with an inserted `return Ok(Handled::Unhandled)`,
which README:306-307 forbids as a hand re-break. This pair is the **pure deletion**: the two leash
clauses leave the top guard (the same e8fe40c2 hunk-3 shape) and the leash branch of
`render_special::append` is deleted, so a leashed entity falls out of `append` through the pre-image
guard and no line is invented. A CONTROL comment is the only inserted text.

## Why this is a committed reversal and not a `git apply -R`

The rope producer was **created** by `client:e8fe40c2` in
`src/entities/render_special/world_fx.rs` (that path does not exist at `e8fe40c2^`), so there is no
pre-image blob to reverse: `ledger promote` patch mode would have nothing to apply. The faithful
control is the deletion, exactly the shape the README allows for a client row whose fix predates the
rename; the restore re-lands the deletion's reverse and becomes the `fix_commit`.

## Real commit `e8fe40c2` (`fix(models): hand-drawn rigs and missing entity visuals …`) — leash part

| # | file `@@` header | what it did | HEAD equivalent | class |
|---|---|---|---|---|
| 1 | `src/entities/render_special.rs` `@@ -6,6 +6,7 @@ mod kernels;` | added `mod world_fx;` | the module moved to `src/entities/render_special/world_fx.rs` in a later refactor; HEAD declares it at `render_special.rs:9` | **(c)** rewritten by a later commit; the module stays (kept-helper) |
| 2 | `@@ -54,6 +55,7 @@ pub(crate) struct Cache` | added `tnt_state: Option<u32>` | HEAD's `Cache` has it | **(a)** the TNT row's behaviour (`append_block` resolves the default primed state through it); reversing it would flip MODEL-ENTITY-TNT-FALLING-LIGHTNING, so it stays |
| 3 | `@@ -146,13 +148,26 @@ pub(crate) fn append(` — the top guard and the leash dispatch | (i) widened the guard to leashed entities (`entity.leash_value().is_none() && entity.leash_holder_id.is_none()`), TNT/falling block and lightning; (ii) added the `lightning_bolt` dispatch; (iii) added the leash dispatch `return world_fx::append_leash(...)`; (iv) added the TNT/falling-block dispatch | HEAD has all four (`render_special.rs:300-334`); the leash dispatch takes `transform` after `895c63f3`/`54ca14f778` | **split**: (i)'s **leash clauses** and (iii)'s **dispatch** are the two pieces this control deletes (both carried — the guard clauses are NOT left out); (ii) and (iv) are **(a)** (the bolt/block rows) and the guard's `tnt`/`falling_block`/`lightning_bolt` arms stay with them |
| 4 | `src/entities/render_special/world_fx.rs` (new file, 391 lines) | the authored module: `tnt_appearance`, `append_block`, `append_bolt`, `append_leash`, `rope_point`, `leash_anchors`, `tube_segment` | HEAD's `render_special/world_fx.rs` (same functions, later arguments) | **(a)** + kept-helper: the module also draws TNT/falling blocks and lightning, so deleting it would turn those rows red; it stays in the control with `append_leash` unreferenced (its only caller is the deleted hunk) |
| 5 | `src/entities/render_special/world_fx_tests.rs` (new file) | tests | HEAD's tests | test hunk, excluded; present on both sides |

No non-test hunk besides 1 and 4's class (a) parts is omitted; hunk 3's leash pieces are **both
carried**, which is what round 3 required.

## The a/-side text of the control

One comment block only:

```rust
// CONTROL (D123 MODEL-ENTITY-LEASH): the pre-image guard admitted only the painting and
// item-frame kinds (plus tnt/falling_block/lightning, which another row owns), so a leashed
// entity fell out here with no rope. The leash clauses and the dispatch are deleted; nothing
// is inserted, and `append_leash` stays in world_fx as a kept helper (`dead_code` is allowed
// crate-wide).
```

Everything else is a deletion; the guard's parenthesised `matches!` closes one line earlier and no
branch, stub or early return is added. `append_leash` stays as the accepted kept helper, dead because
its only caller is the deleted hunk; `dead_code` is allowed crate-wide (`src/lib.rs:25`).

## The scene, the assertion and the threshold

`native-entity-visuals` (`tools/harness/entity_visuals_scene.json`) summons a sheep at
`(998.5, 160, 1004.5)` and a cow at `(1002.5, 160, 1004.5)` leashed to it, then shoots `leashed` from
`(1000.5, 161.62, 1000.5)` (yaw 0, pitch 0, fov 70, 1280x720). `check_entity_visuals` counts the
rope's own colour in the ROI `(560, 400, 760, 500)` and asserts `leash_rope_drawn: count > 300`.

The threshold is **derived from the model and registered before the run** (the comment at
`scene_shot_checks.py:160-172` and the row's `repro_check` carry this derivation; the arithmetic was
corrected in round 3):

- the rope is `tube_segment(..., 0.018, ...)` — a 0.036-block-diameter tube on the anchor-to-anchor
  line; `leash_anchors` puts the cow's end at `y = 160 + 1.4 * 0.7`, `z = 1004.5 + 0.45` and the
  sheep's at `y = 160 + 1.3 * 0.7`, `z = 1004.5 + 0.45`, i.e. a 4.03-block horizontal gap;
- `rope_point`'s sag `4 * clamp(horizontal * 0.1, 0.08, 0.75) * t * (1 - t)` caps at 0.403 blocks, so
  the rope's mid sits at `y ≈ 160.54` and the visible stretch is inside the ROI;
- the anchors are at `z = 1004.95`/`1005.45` against a camera at `z = 1000.5`, so the rope is ~4.45
  blocks away: one block is `360 / (4.45 * tan(35°)) = 115.5 px`, the tube projects to ~4.2 px, and
  the ROI's x range `560..760` is ndc `-0.125..+0.1875` (asymmetric), i.e. `-0.69..+1.04` blocks
  (`tan(hfov/2) = tan(35°) * 16/9 = 1.2448`), about 1.73 blocks ≈ 200 px of rope;
- the model's lower bound inside the ROI is therefore ~760-840 px; `> 300` demands under half of one
  visible crossing, which no rig, shadow or sky can supply.

## The measured pair

Three takes per side on `native-entity-visuals`, all six on the clean round-3 server pin
`pins/38fe67d13d1a-server-*` (core `38fe67d13`):

| side | receipts | result | `leash_rope_pixels` |
|---|---|---|---|
| control `client:fd4d1f5b` | `20261125T190755` / `190838` / `190913` | FAIL at `leash_rope_drawn` on all three | 0 / 0 / 0 |
| restore `client:a8c18847` | `20261125T190946` / `191015` / `191045` | PASS on all three | 1041 / 1041 / 1041 |

`ledger promote MODEL-ENTITY-LEASH` receipt `20261125T191401-ledger-promote-r1e_2hif`;
`promote.json` records `fix_patch_id e1cc83d3…` and the delta path
`src/entities/render_special.rs` only. The earlier pair (control `df888507`, restore `03c4675b`,
receipts `20261125T181048`/`181119`/`181154` vs `181228`/`181307`/`181341`, promote
`20261125T181432-ledger-promote-wdy6djxt`) is SUPERSEDED: its control inserted the early return
round 3 rejected.

## What the scene does not prove

The rope's colour and length only: its texel parity, the sag curve against vanilla's
`LeashFeatureRenderer` and the knot's own rig stay unit-covered (`world_fx_tests.rs`), and the ROI is
one camera and one gap.
