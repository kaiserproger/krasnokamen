# MODEL-ENTITY-TNT-FALLING-LIGHTNING — hunk map for the pinned pair

Promotion receipt: `promotions/MODEL-ENTITY-TNT-FALLING-LIGHTNING.json` (written by `ledger promote`).
Client pair on the current head `03c4675b`: control **`client:d2c556ed`** (the three dispatches
deleted, the guard narrowed) → restore **`client:9f8b91c8`** (exactly that diff re-applied; the row's
`fix_commit`). Clean pins `pins/d2c556edf020-client-*`, `pins/9f8b91c8*`.

## Why this is a committed reversal

The real fix `client:e8fe40c2` (`fix(models): hand-drawn rigs and missing entity visuals …`) created
`src/entities/render_special/world_fx.rs` — no pre-image blob exists at `e8fe40c2^` — and it is a
bulk 63-file commit whose other hunks belong to the rig rows. Patch mode therefore has nothing to
reverse; the README's committed-reversal route applies, and this map names every hunk of the real
commit that the reversal carries or leaves.

## Real commit `e8fe40c2` — the TNT / falling-block / bolt part

| # | file `@@` header | what it did | HEAD equivalent | class |
|---|---|---|---|---|
| 1 | `src/entities/render_special.rs` `@@ -6,6 +6,7 @@ mod kernels;` | added `mod world_fx;` | the module is `render_special/world_fx.rs` after a later split; HEAD declares it at `render_special.rs:9` | **(c)** rewritten by a later commit; the module stays (kept-helper) |
| 2 | `@@ -54,6 +55,7 @@ pub(crate) struct Cache` | added `tnt_state: Option<u32>` | HEAD's `Cache` has it | **kept-helper**: `append_block` resolves the default primed state through it, the control keeps the field and the helper, and nothing in the pair's delta touches it |
| 3 | `@@ -146,13 +148,26 @@ pub(crate) fn append(` — the top guard | widened the guard to `tnt`, `falling_block`, `lightning_bolt` and leashed entities | HEAD's guard has the same list plus the leash clauses | **split**: the `tnt`/`falling_block`/`lightning_bolt` arms are reversed by the control (the guard goes back to painting/item frames); the leash arms are **(a)** (MODEL-ENTITY-LEASH is promoted — reversing them would flip its assertion) |
| 4 | same hunk — the `lightning_bolt` dispatch | `return world_fx::append_bolt(entity, origin, tick, partial, out);` | HEAD's call is `append_bolt(entity, origin, transform, out)` (a later commit added the transform) | reversed by the control (`append_bolt` stays as a kept helper) |
| 5 | same hunk — the leash dispatch | `return world_fx::append_leash(...)` | HEAD's call | **(a)** MODEL-ENTITY-LEASH's promoted behaviour; untouched |
| 6 | `@@ -163,6 +178,10 @@ pub(crate) fn append(` — the block draw | `if matches!(tnt \| falling_block) { return world_fx::append_block(...) }` | HEAD's call passes `p::EntityDrawFrame { origin, transform, tick, partial, light, parent: None }` plus `tint_source` (later commits added tint and the environment) | reversed by the control (`append_block` stays as a kept helper) |
| 7 | `src/entities/render_special/world_fx.rs` (new file, 391 lines) | the authored module: `tnt_appearance`, `append_block`, `append_bolt`, `append_leash`, the rope helpers | HEAD's `render_special/world_fx.rs` | **kept-helper**: `append_block` and `append_bolt` become unreferenced in the control; `append_leash`'s caller is the promoted leash dispatch and stays live. Deleting the module would flip MODEL-ENTITY-LEASH, so it stays |
| 8 | `src/entities/render_special/world_fx_tests.rs` (new file) | the TNT/swell/flash/rope unit tests | HEAD's tests | test hunk, excluded; present on both sides |

## The a/-side text of the control

Three deletions, no invented behaviour:

```rust
// the guard, leash clauses untouched:
if !matches!(
    entity.type_name.as_str(),
    "minecraft:painting" | "minecraft:item_frame" | "minecraft:glow_item_frame"
) && entity.leash_value().is_none()
    && entity.leash_holder_id.is_none()
{ return Ok(Handled::Unhandled); }

// the bolt dispatch: deleted (append_bolt stays in world_fx)
// the block draw: deleted (append_block stays in world_fx)
```

That is the pre-`e8fe40c2` outcome for a primed TNT, a falling block and a lightning bolt: before the
fix the guard did not admit them and `append` returned `Handled::Unhandled` at the top, so `submit`
drew nothing for them. The comments left in their place are control markers, not behaviour. The
leash clauses are kept verbatim because reversing them would turn the promoted MODEL-ENTITY-LEASH
red — the class (a) split above.

## The scene and the exact assertion

`native-entity-tnt` (`tools/harness/entity_tnt_scene.json`) builds the fire scene's pad, shoots
`empty`, summons `minecraft:tnt 1000.5 160 1003.5 {fuse:1000s}`, waits 20 ticks, shoots `tnt` and
dumps the entities. `check_entity_tnt` runs the fixture assertions first (`*_gpu_complete`,
`*_image_size`, `tnt_camera_unchanged`, and `empty_frame_has_no_tnt_block` — the unlit frame's entity
pass must carry no opaque layer), then:

- `tnt_block_layer_present` — the TNT frame has a 36-index (six-quad) block layer (the row's proving
  ref). The layer is `ItemCutout` unlit and `EntityCutout` while the white-flash phase is on
  (`append_block` re-targets this draw's indices onto the overlay-capable material while `isLit`,
  which alternates every five ticks and is NOT stable across takes — the first runs showed exactly
  that), so the particle identity is the 36-index block model, not the material name;
- `tnt_block_layer_is_one` — exactly one such layer;
- `tnt_block_quads` — 6 quads;
- `tnt_block_span_is_one` — 1.000 blocks on every axis.

The span is **derived by javap on the real 26.3 jar**, not measured: `TntRenderer.extractRenderState`
sets `fuseRemainingInTicks = getFuse() - partial + 1`; `submit` applies
`1 + getSwellAmount(fuseRemaining)` only when `fuseRemaining < 10`, with
`getSwellAmount(f) = 0.3 * clamp(1 - f/10, 0, 1)^4`; a 1000-tick fuse is far above 10, so the block is
its unit cube at scale 1. The same disassembly gives `isLit(f) = f >= 0 && (int)(f/5) % 2 == 0` and
the block submission through `submitWhiteSolidBlock`, which the scene does not need: both the swell
and the flash are pinned by the existing unit tests
(`tnt_flashes_white_and_swells_only_at_fuse_end` asserts r = 9 → 1.0 and r = 8 > 1.0 plus the
`overlay_uv = [15., 10.]` swap; `tnt_uses_received_block_state_and_fuse_boundaries` asserts the
boundary r = 4 vs 4.01 and the flash phase change).

## The measured pair (2026-11-25, server pin `pins/51f5158b893b-server-*` = core `51f5158b8`)

Three takes per side on `native-entity-tnt`, all six on the same clean server pin:

| side | receipts | result | block layer |
|---|---|---|---|
| control `client:d2c556ed` | `20261125T183446` / `183515` / `183553` | FAIL at `tnt_block_layer_present` on all three | none (the entity pass carries no 36-index layer) |
| restore `client:9f8b91c8` | `20261125T183331` / `183401` / `183422` | PASS on all three | 6 quads, spans `[1.0, 1.0, 1.0]`, material `ItemCutout` / `EntityCutout` / `ItemCutout` |

The first TNT attempt on server `pins/ae98d53c2531-server-*` (core `ae98d53c2`, the check before it matched the 36-index block model) recorded three control takes `20261125T182716` / `182743` / `182805`, all FAIL at `tnt_block_layer_present`; they are kept as calibration for the material change and are not part of the promotion (the promoted control is `183446` / `183515` / `183553`).

The material alternation is the white-flash phase and is the reason the check matches the
36-index block model instead of a material name: a first pair on the pre-`51f5158b8` check
(`20261125T182556` PASS, `182619` / `182650` FAIL, server `pins/ae98d53c2531-server-*`) failed
exactly because the same draw had moved from `ItemCutout` to `EntityCutout`; that attempt is kept
as the calibration record for the material change, and its receipts prove the frame still submitted
the entity (`submitted: 1`, `entity_indices: 51108`). `ledger promote
MODEL-ENTITY-TNT-FALLING-LIGHTNING` receipt `20261125T183634-ledger-promote-5df3vsc9`;
`promote.json` records `fix_patch_id b924ddca…`.

## What the scene does not prove

The falling-block and lightning paths beyond the shared guard clause (they stay unit-covered), the
swell/flash at the fuse end (unit-covered; a scene for them would need the fuse read within ±2 ticks of
the capture, because the frame report carries only per-layer bounds), and the TNT texture's texels.
