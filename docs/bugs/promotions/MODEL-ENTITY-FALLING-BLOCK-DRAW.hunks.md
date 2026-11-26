# MODEL-ENTITY-FALLING-BLOCK-DRAW — hunk map for the committed reversal pair

Promotion: `docs/bugs/promotions/MODEL-ENTITY-FALLING-BLOCK-DRAW.json` (written by `ledger promote`).
Scene: `native-entity-falling-block` (`tools/harness/entity_falling_block_scene.json`), proving
assertion `falling_block_layer_present`.

## Why this pair

The real fix `client:e8fe40c2` **created** `src/entities/render_special/world_fx.rs` (no pre-image
blob exists at its parent), so the row has no faithful a/-side. Its control is therefore the
committed reversal on the current head, exactly as MODEL-ENTITY-TNT-FALLING-LIGHTNING's: the
`append` guard and dispatches are deleted, the `world_fx` module stays as a kept helper (dead_code
is allowed crate-wide), and the next commit re-lands the same lines.

| half | client commit | content |
|---|---|---|
| control | `client:d2c556ed` | guard narrowed back to painting/item frames (leash clauses stay: MODEL-ENTITY-LEASH is promoted), the `lightning_bolt` dispatch and the `tnt \| falling_block` block draw deleted |
| restore (= `fix_commit`) | `client:9f8b91c8` | re-applies exactly that deletion in reverse; `git diff d2c556ed 9f8b91c8` is `client:9f8b91c8`'s non-test patch (patch id `b924ddca…`) |

`ledger promote`'s client rule compares the two pinned heads' production patch with
`fix_commit`'s, so the recorded `fix_commit` is the restore commit and the delta is the whole
three-subject patch. That is honest: the three subjects share one guard line and one dispatch arm,
and a hand edit that deletes only `falling_block` would be an invented pre-image (the README's
"a hand re-break is not a control"). The same pair carried the TNT row (promoted
`2026-11-25T18:36:34Z`); this row's scene summons one falling block and no TNT.

## Hunks carried

`src/entities/render_special.rs` (the only file):

| hunk | change | role |
|---|---|---|
| `@@ -297,14 +297,13 @@` | the top guard's `\| "minecraft:tnt" \| "minecraft:falling_block" \| "minecraft:lightning_bolt"` arms removed; guard is `painting \| item_frame \| glow_item_frame` plus the leash clauses | **this row**: the `minecraft:falling_block` arm, without which the entity returns before any draw |
| `@@ -314,9 +313,6 @@` | the `minecraft:lightning_bolt` → `world_fx::append_bolt` early return deleted | class (a): MODEL-ENTITY-LIGHTNING-DRAW's subject, unavoidable on the same guard/patch |
| `@@ -364,27 +360,8 @@` | the `matches!("minecraft:tnt" \| "minecraft:falling_block")` → `world_fx::append_block` branch deleted, `append_block` kept in `world_fx` | **this row**: the block draw (the arm is shared with the TNT row — class (a) half) |

No test hunks: the row's unit coverage (`world_fx_tests::tnt_and_received_falling_block_emit_the_pack_model`)
was published by the same bulk commit and is not part of the non-test patch.

## The scene proves the dispatch, not the model

The assertion reads the captured frame's own `entity_extraction.entity_layer_geometry` (CM-4, the
cloud-free entity pass): one `ItemCutout`/`EntityCutout` layer with 36 indices (six quads) spanning
1.000 on every axis, which is the received `BlockState`'s cube at scale 1 — a falling block has no
fuse, so no swell and no white-flash phase to alternate the material, unlike the TNT row. The
fixture checks come first: the `empty` frame carries no 36-index block layer, and
`entity_extraction.eligible >= 1` plus the `falling-block-entities` operator dump listing
`minecraft:falling_block` prove the summoned entity reached the client, so a refused summon cannot
read as the deleted dispatch.
