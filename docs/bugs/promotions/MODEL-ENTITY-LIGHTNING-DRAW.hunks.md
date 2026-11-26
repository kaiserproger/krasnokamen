# MODEL-ENTITY-LIGHTNING-DRAW — hunk map for the committed reversal pair

Promotion: `docs/bugs/promotions/MODEL-ENTITY-LIGHTNING-DRAW.json` (written by `ledger promote`).
Scene: `native-entity-lightning` (`tools/harness/entity_lightning_scene.json`), proving assertion
`lightning_layer_present`.

## Why this pair

The real fix `client:e8fe40c2` **created** `src/entities/render_special/world_fx.rs`, so there is no
pre-image blob and no a/-side to restore. The control is the committed reversal on the current head,
shared with MODEL-ENTITY-TNT-FALLING-LIGHTNING and MODEL-ENTITY-FALLING-BLOCK-DRAW:

| half | client commit | content |
|---|---|---|
| control | `client:d2c556ed` | guard narrowed back to painting/item frames (leash clauses stay), the `minecraft:lightning_bolt` → `append_bolt` early return deleted, the `tnt \| falling_block` block draw deleted |
| restore (= `fix_commit`) | `client:9f8b91c8` | re-applies exactly that deletion in reverse; `git diff d2c556ed 9f8b91c8` is `client:9f8b91c8`'s non-test patch (patch id `b924ddca…`) |

One guard line and one `matches!` arm are shared by the three subjects, so `fix_commit` is the
restore commit and the promote's `client_control_delta` is the whole patch; a hand edit deleting only
the bolt would invent a pre-image the fix never had.

## Hunks carried

`src/entities/render_special.rs` (the only file):

| hunk | change | role |
|---|---|---|
| `@@ -297,14 +297,13 @@` | the top guard's `\| "minecraft:lightning_bolt"` arm (with the tnt/falling_block arms) removed | **this row**: without the arm the bolt returns from `append` before any draw |
| `@@ -314,9 +313,6 @@` | the `if entity.type_name == "minecraft:lightning_bolt" { return world_fx::append_bolt(...) }` early return deleted | **this row**: the dispatch itself (`append_bolt` stays in `world_fx` as a kept helper) |
| `@@ -364,27 +360,8 @@` | the `tnt \| falling_block` → `append_block` branch deleted | class (a): the TNT and falling-block rows' subject, unavoidable on the same patch |

No test hunks: `world_fx_tests::lightning_dispatch_emits_seeded_geometry_without_light_or_pack_textures`
and `lightning_is_absent_between_sky_flashes` came with the bulk commit and stay as the client refs.

## The scene proves the dispatch, not the bolt's texels

`world_fx::append_lightning` draws 13 spine segments plus two 4-segment branches (21 segments), each
an inner (0.09) and an outer (0.28) tube, and a tube is four quads of six indices: **1008 indices**
on `EntityMaterial::Lightning`, with the spine ending at `position.y + 128` (the 128-block bolt).
The bolt is only drawn while `timeline.lightning_visible()` (client life starts at 2, then 1..3
re-flashes with a random gap up to ten ticks), so the scene takes sixteen consecutive shots after
the summon and the check requires that at least one of them carries the `Lightning` layer with those
numbers. The `empty` frame carries no `Lightning` layer, and the operator dump lists
`minecraft:lightning_bolt` — the server keeps the entity for 20 ticks (`ProjectileKind::Lightning`
in `crates/mc-core/src/projectiles/payloads.rs`), which covers the sixteen shots.
