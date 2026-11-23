# MODEL-OPUS-15 — hunk map for the tip reversal pair

Promotion receipt: `promotions/MODEL-OPUS-15.json` (candidate core `e9a5cf9e31`, then re-promoted on
the same candidate with the reversal pair below).
Client pair: reversal `client:e292b8f68e` → re-land `client:10a9c5dee5` (the row's `fix_commit`).

## Original commit

`client:e8fe40c2` "fix(models): hand-drawn rigs and missing entity visuals from public references"
(2026-11-15) is a bulk commit: it also adds creepers, render_living, render_special/world_fx,
profiles::head_skin, source_state and item bars, plus ~120 `assets/solaris/models/**` JSON files.
It cannot be reversed wholesale on the current tip, so the pair forward-ports **only the worn-back
layer** hunks and leaves every other hunk to its own row.

## Hunks of `src/entities/equipment.rs` (the only file carrying this row's behaviour)

| # | original hunk (`@@` header) | content | disposition |
|---|---|---|---|
| 1 | `@@ -22,6 +22,8 @@ struct Asset` | `LayerDefinition.use_player_texture` (`#[serde(default)]`) | KEPT — re-landed by `10a9c5dee5`; `e292b8f68e` removes the field's consumer, and the field itself is API the kept `equipment_tests` call, so it stays (see adaptation) |
| 2 | `@@ -31,6 +33,7 @@ struct Layer` | `Layer.use_player_texture` | KEPT — same as #1; the field is constructed by `load_asset` (#5) and read by the elytra draw |
| 3 | `@@ -78,6 +81,76 @@ impl Cache` | `wings_asset()` + `append_player_back()` (the whole back-layer draw) | KEPT — the row's symptom; `e292b8f68e` deletes the body, `10a9c5dee5` restores it |
| 4 | `@@ -91,22 +164,21 @@ impl Cache` | the player-slot-4 early return in `append()` (so the back layer is not also drawn as a chestplate layer) | KEPT — part of the same behaviour; reversed and re-landed with #3 |
| 5 | `@@ -244,6 +316,7 @@ fn load_asset(...)` | `use_player_texture: definition.use_player_texture` | KEPT — plumbing for #1/#2 |
| 6 | `@@ -284,6 +357,33 @@ fn armor_rig(...)` | `animal_overlay()` (horse/wolf/llama/ghast/nautilus/saddles) | OMITTED — (a): horseback/wolf/llama/nautilus equipment overlays are other rows' behaviour (MODEL-OPUS-11/…); reversing them would turn their controls red |
| 7 | `@@ -296,6 +396,7 @@ mod tests` (first inline `#[cfg(test)]` arm) | test fixture resource | OMITTED — test hunk (inline `#[cfg(test)]`), never reversed |
| 8-14 | `@@ -430…`, `@@ -438…`, `@@ -820…`, `@@ -918…`, `@@ -932…`, `@@ -1023…`, `@@ -1061…` | inline test arms | OMITTED — test hunks |

## Other files of `e8fe40c2`

| file | hunks | disposition |
|---|---|---|
| `src/entities.rs` | `mod creeper;` | OMITTED — (a), the creeper row |
| `src/entities/creeper.rs` | new rig | OMITTED — (a) |
| `src/entities/presentation.rs` | rig evaluation/pose helpers | OMITTED — (a): shared rig code, its reversal reds the other entity rows; also (b), the kept evaluation tests call it |
| `src/entities/profiles.rs` | `head_skin()` | OMITTED — (a), MODEL-OPUS-N3/MODEL-PLAYER-HEAD-PROFILE |
| `src/entities/render_blocks.rs`, `render_living*.rs`, `render_special*.rs`, `source_state.rs`, `solaris_models/tests.rs` | block-entity/special/state visuals and their tests | OMITTED — (a) (their rows) and test hunks |
| `src/ui/item_bars.rs` | one hunk | NOT this row — reversed and re-landed for MODEL-ITEM-BARS (see its own map) |
| `src/ui/state.rs` | `mod item_bars;` + test-module registration | NOT this row — the `mod item_bars;` line is (b) API the kept bar tests call (and MODEL-ITEM-BARS's behaviour); the `#[cfg(test)]` arm is a test hunk |
| `docs/ENTITY_MODELS.md`, `assets/solaris/models/**`, `tools/entity_models.py` | assets/docs | OMITTED — (a): the rig asset set belongs to the model rows and is not the back-layer draw path |

## a/-side adaptation (the only text outside the pre-image)

`e292b8f68e` keeps `append_player_back`'s signature, so the reversed body takes `_e`, `_profile`,
`_frame`, `_out` and returns `Ok(())`; `10a9c5dee5` restores the real names and body. The renamed
parameters add no behaviour: the reversed function draws nothing. Everything else in the pair is the
original pre-image of hunks 1-5 restored text-for-text (`git diff e292b8f68e 10a9c5dee5` equals the
removed block).

## Proving ref

`harness:scene-shot:native-elytra` → `worn_elytra_drawn`
(`tools/harness/backends/scene_shot_checks.py:312`, "elytra > 4000 changed pixels in the avatar's
back region against the same player with an empty chest slot"). That assertion is the row's symptom
(the worn back layer is not drawn); the companion `chestplate_back_roi` (:311) and
`idle_back_motion_bounded` (:313) stay green on both sides and only bound the comparison.
