# MODEL-DYNAMIC-CONTEXT — hunk map for the tip reversal pair

Promotion receipt: `promotions/MODEL-DYNAMIC-CONTEXT.json`.
Client pair: reversal `client:698fe4a947` → re-land `client:4f8d72692d` (the row's `fix_commit`).
Both pins clean (`698fe4a94796`, `4f8d72692d29`); the pair's net tree is client `main` `530556e8`.

## Original commit

`client:9096a86b` — "feat(entities): 26.3 cushion/sulfur_cube rigs and shelf block-item clock
context", 2026-11-11, 16 files. It carries two unrelated behaviours: the cushion/sulfur-cube rigs
(another row's path, and `MODEL-OPUS-*` rows own it) and the **block-item world context** this row
describes. The pair forward-ports only the second.

## Hunks

| file | original hunk | content | disposition |
|---|---|---|---|
| `src/entities/render_special/items.rs` | `@@ -81,10 +83,30 @@ pub(crate) fn append_observed_item(` | the `world_context` construction (position, facing yaw, sun angle, moon phase, per-block-slot `block_clock_needles`) and `..world_context.unwrap_or_default()` | REVERSED — `698fe4a947` deletes the block and restores the pre-fix `..Default::default()`; `4f8d72692d` restores both verbatim. This is the row's symptom: without it the observed item's `ItemContext` never sees the world and the shelf clock/compass face is frozen |
| `src/entities/render_special/items.rs` | `@@ -62,6 +62,7 @@ struct ObservedItemFrame` (`pub block_world: …`) | the frame field the construction reads | NOT reversed — reason (b): the struct literal in `render_blocks.rs` must keep constructing it, or the control stops compiling. `698fe4a947` binds it as `block_world: _` (the documented `_`-binding adaptation); `4f8d72692d` restores the name |
| `src/entities/render_special/items.rs` | `@@ -29,11 +29,11 @@` (`observed_stack` → `ensure_observed_stack` + `cache.observed_items[&key]`) | borrow-checker accommodation for the construction (the stack borrow has to end before `dynamic_item(&mut cache.dynamic_models, …)`) | NOT reversed — behaviour-neutral refactor the kept locals use; reversing it without reverting the construction is not possible, and it changes no drawing |
| `src/entities/render_blocks.rs` | `@@ -130,6 +130,7 @@` (`clock_environment` field), `@@ -158,7 +159,7 @@` (destructure), `@@ -304,7 +305,12 @@` (`block_world: clock_environment.map(...)`) | the block scene frame's world-context input | NOT reversed — (b): the field is required by every `StaticBlockSceneFrame` literal (including `entities/animation_tests.rs:2150` and `crowd_tests.rs:1318`, which build `Some(...)`), so removing it stops the control from compiling |
| `src/entities.rs` | `@@ -1591,6 +1591,7 @@` and `@@ -1609,6 +1610,7 @@` (now around `remove_block_entity`/`unload_chunk`) | `special_render_cache.remove_block` / `unload_chunk` | NOT reversed — (b): `block_clock_needles` and the `block_needle_tests` module stay; the invalidation is inert while the construction is absent |
| `src/entities/blocks.rs` | `@@ -56,6 +56,7 @@` | `special_render_cache.remove_block(position)` | NOT reversed — (b), same reason |
| `src/entities/render_special.rs` | `@@ -55,14 +55,46 @@` and `@@ -76,31 +108,9 @@` (`block_clock_needles`, `remove_block`, `unload_chunk`, the `dynamic_item` extraction) and `@@ -227,3 +237,21 @@` (`mod block_needle_tests`) | the needle cache and the dynamic-model predicate | NOT reversed — (b)/(c): the extraction is behaviour-neutral and is the only caller side of the kept `observed_stack` refactor; the cache and its test module are the API the kept invalidation calls and tests use |
| `src/session/observation.rs` | `@@ -874,6 +874,7 @@ impl Session` (`clock_environment: Some((&item_context, &item_time_at))`) | the session supplies the world context | NOT reversed — (b): the struct literal needs the field |
| `src/entities/presentation.rs` | `@@ -268,6 +268,41 @@`, `@@ -795,6 +830,11 @@`, `@@ -870,6 +910,15 @@`, `@@ -982,12 +1031,24 @@` | cushion/sulfur-cube material, facing snap, scale and the inner emissive layer | OMITTED — (a) another row's behaviour (cushion/sulfur rigs); reversing it would red those rows' assertions, not this one |
| `src/entities/resources.rs` | `@@ -542,11 +542,15 @@` and `@@ -640,6 +644,23 @@` | the fixture texture list and a cushion/sulfur render test | OMITTED — test hunks |

## a/-side adaptation

`698fe4a947` keeps the `ObservedItemFrame` destructure and binds its `block_world` field as
`block_world: _`; every other line of the reversal is the fix's own pre-image (`..Default::default()`
is the line `9096a86b` replaced). No stub, no flag, no early return: the reversed tree still computes
and passes the block world context, and only the application to the item state — the drawing — is
gone.

## Proving ref

`harness:scene-shot:native-shelf-clock` → `clock_face_changed`
(`tools/harness/backends/scene_shot_checks.py:100`): the shelf's clock face is read in a fixed
`18x24` ROI (`435,457`-`453,481`) in the day and the night frame, and at least 20 blue-pattern pixels
must change. The scene's shelf is `minecraft:oak_shelf[facing=north]{Items:[{Slot:1b,id:"minecraft:clock"}]}`
— a block entity, so the assertion lands on this row's path. With the construction removed the face
is frozen and the control reds it.

The second registered ref, `harness:scene-shot:native-dynamic-context` →
`dropped_clock_face_changed`, covers a **dropped** clock item, which is drawn by the entity path
(`Cache::item_context`); the pair does not touch it, so it stays green on both sides and is not this
row's control. The row's remaining surfaces stay unproven: item frames cannot hold an item through
`summon` on this server, and a mob/other-player hand clock needs a second client with a held item.
