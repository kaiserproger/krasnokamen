# MODEL-ITEM-BARS — hunk map for the tip reversal pair

Promotion receipt: `promotions/MODEL-ITEM-BARS.json`.
Client pair: reversal `client:e292b8f68e` → re-land `client:10a9c5dee5` (the row's `fix_commit`).

## Original commit

`client:e8fe40c2` (bulk rig/visual commit, see `MODEL-OPUS-15.hunks.md`). This row's behaviour is
the GUI item-bar module and its wiring; the pair forward-ports only those hunks.

## Hunks

| file | original hunk | content | disposition |
|---|---|---|---|
| `src/ui/item_bars.rs` | `@@ -0,0 +1,137 @@` (new file) | `append()` — durability/bundle track and fill painting, plus `bundle_fullness` and the `TRACK`/palette constants | REVERSED — the file is one hunk: `e292b8f68e` deletes the paint body and keeps the signature and helpers (reason (b), see the adaptation), `10a9c5dee5` restores the body |
| `src/ui/state.rs` | `@@ -20,6 +20,9 @@` `#[path = "item_bars.rs"] mod item_bars;` | module wiring | NOT reversed — reason (b): the kept `item_bar_tests` call `item_bars::append` through it, so removing the module stops the control from compiling. It is identical on both sides of the pair |
| `src/ui/state.rs` | `@@ -4144,7 +4147,7 @@ fn append_stack_item(...)` `item_bars::append(content, stack, bounds, parent, out)` | the production call site | NOT reversed — **unchanged by the pair** (neither commit touches `state.rs`): the call site keeps calling `append`, which is exactly why the reversed body is reached and paints nothing. Reversing it would remove the production path instead of the drawing |
| `src/ui/state.rs` | `@@ -4671,3 +4674,7 @@` `#[cfg(test)] mod item_bar_tests;` | test-module registration | OMITTED — test hunk |
| every other file of `e8fe40c2` | see `MODEL-OPUS-15.hunks.md` | other rows' visuals | OMITTED — (a) another row's behaviour, or test hunks |

## a/-side adaptation

`e292b8f68e` keeps `item_bars::append`'s signature (the module wiring and call site in `state.rs` are
untouched by both commits), so the reversed function takes `_content`, `_stack`, `_bounds`,
`_transform`, `_out` and returns `Ok(())`; `10a9c5dee5` restores the real names, the body and the
`Painter` calls. The reversed function paints nothing — no behaviour besides the removal.

## Proving ref

`harness:scene-shot:native-item-bars` → `pickaxe_slot_bar_changes`
(`tools/harness/backends/scene_shot_checks.py:217`, "the damaged pickaxe slot changes against the
same slot with an undamaged pickaxe"). That assertion is the row's symptom (the bar is not drawn
under the icon); `bundle_slot_bar_changes` (:218) is the bundle half and `durability_bar_drawn` /
`fullness_bar_drawn` (:221-222) read the bar's own pixels.
