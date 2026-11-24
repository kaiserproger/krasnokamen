# MODEL-OPUS-15 — hunk map for the tip reversal pair

Promotion receipt: `promotions/MODEL-OPUS-15.json`.
Client pair: reversal `client:c290f71b95` → re-land `client:b11ca37604` (the row's `fix_commit`).

This pair supersedes the earlier `e292b8f68e` → `10a9c5dee5` pair, whose map claimed the slot-4
omission guard was reversed when it was not (review ReviewD123Client4 B2). The earlier pair is kept
in the client history and still carries MODEL-ITEM-BARS.

## Original commit

`client:e8fe40c2` "fix(models): hand-drawn rigs and missing entity visuals from public references"
(2026-11-05) is a bulk commit (234 files: rigs, renderers, item bars, ~120 model JSON files). It
cannot be reversed wholesale on the current tip, so the pair forward-ports only the worn-back-layer
hunks and leaves every other hunk to its own row.

## `src/entities/equipment.rs` — every non-test hunk

| # | original hunk | content | disposition in the pair |
|---|---|---|---|
| 1 | `@@ -22,6 +22,8 @@ struct Asset` | `#[serde(default)] use_player_texture: bool` on `LayerDefinition` | NOT reversed — reason (b): the retained fixture and constructors name it (`equipment_tests.rs:196` JSON, `equipment.rs:685`, `:1530`, `:1850-1866`, `:1958`, `:2006`), so deleting the field deletes the plumbing the kept refs exercise; with the body stubbed nothing reads it and it cannot change a frame |
| 2 | `@@ -31,6 +33,7 @@ struct Layer` | `use_player_texture: bool` on `Layer` | NOT reversed — same (b) proof as #1 |
| 3 | `@@ -78,6 +81,76 @@ impl Cache` | `wings_asset()` + the whole `append_player_back` body (rig choice, wing/cape motion, player-texture layers) | REVERSED in the pair's `@@ -102,11 +102,87 @@` — the body becomes `Ok(())` and the parameters `_e/_profile/_frame/_out` (a/-side adaptation below) |
| 4 | `@@ -91,22 +164,21 @@ impl Cache` | the player-slot-4 `Worn::Omitted` early return in `resolve` (so the wings are not also drawn as a chestplate layer) | REVERSED in the pair's `@@ -122,6 +198,13 @@` — this is the hunk the earlier pair left in place; `git blame -C -C -L 201,207 -- src/entities/equipment.rs` (run on `b11ca37604`) names `e8fe40c20` for the comment, the `wings_asset` condition and the braces, `9a645aa99` for the re-wrapped first two condition lines and `a89b5664d` for the `return` |
| 5 | `@@ -244,6 +316,7 @@ fn load_asset(...)` | `use_player_texture: definition.use_player_texture` | NOT reversed — reason (b), same as #1 |
| 6 | `@@ -284,6 +357,33 @@ fn armor_rig(...)` | `animal_overlay()` (horse/wolf/llama/ghast/nautilus gear) | OMITTED — (a): those overlays are MODEL-OPUS-3/11's behaviour; reversing them would red their controls |
| 7 | `@@ -296,6 +396,7 @@ mod tests` | first inline `#[cfg(test)]` arm | OMITTED — test hunk |
| 8 | `@@ -430,6 +531,7 @@ mod tests` | inline test arm | OMITTED — test hunk |
| 9 | `@@ -438,6 +540,8 @@ mod tests` | inline test arm | OMITTED — test hunk |
| 10 | `@@ -820,6 +924,7 @@ mod tests` | inline test arm | OMITTED — test hunk |
| 11 | `@@ -918,11 +1023,13 @@ mod tests` | inline test arm | OMITTED — test hunk |
| 12 | `@@ -932,6 +1039,7 @@ mod tests` | inline test arm | OMITTED — test hunk |
| 13 | `@@ -1023,6 +1131,7 @@ mod tests` | inline test arm | OMITTED — test hunk |
| 14 | `@@ -1061,6 +1170,7 @@ mod tests` | inline test arm | OMITTED — test hunk |

`src/entities/equipment_tests.rs` `@@ -0,0 +1,289 @@` is a test file and is never reversed.

## Every other file of `e8fe40c2`

| file | hunk (`@@` header) | disposition |
|---|---|---|
| `docs/ENTITY_MODELS.md` | `@@ -40,6 +40,14 @@`, `@@ -94,8 +102,18 @@`, `@@ -125,7 +143,8 @@`, `@@ -230,11 +249,9 @@`, `@@ -259,7 +276,14 @@`, `@@ -308,10 +332,9 @@` | OMITTED — docs |
| `src/entities.rs` | `@@ -4,6 +4,7 @@` (`mod creeper;`) | OMITTED — (a), the creeper row |
| `src/entities/animation_tests.rs` | `@@ -231,6 +231,156 @@` | OMITTED — test hunk |
| `src/entities/creeper.rs` | `@@ -0,0 +1,75 @@` | OMITTED — (a), the creeper row |
| `src/entities/presentation.rs` | `@@ -418,6 +418,7 @@`, `@@ -651,6 +652,7 @@`, `@@ -822,6 +824,10 @@`, `@@ -947,18 +953,28 @@`, `@@ -1047,9 +1063,20 @@`, `@@ -1116,6 +1143,12 @@`, `@@ -1156,12 +1189,8 @@`, `@@ -1288,22 +1317,29 @@` | OMITTED — (a): shared rig/pose evaluation for the other entity rows; also (b) the kept evaluation tests call it |
| `src/entities/presentation.rs` | `@@ -2341,7 +2377,6 @@ mod tests`, `@@ -2810,8 +2845,11 @@ mod tests` | OMITTED — test hunks |
| `src/entities/profiles.rs` | `@@ -168,6 +168,35 @@` (`head_skin()`) | OMITTED — (a), MODEL-OPUS-N3 / MODEL-PLAYER-HEAD-PROFILE |
| `src/entities/render_blocks.rs` | `@@ -270,6 +270,7 @@`, `@@ -430,17 +431,20 @@` | OMITTED — (a), the block-entity rows |
| `src/entities/render_blocks/village_tests.rs` | `@@ -723,6 +723,56 @@` | OMITTED — test hunk |
| `src/entities/render_living.rs` | `@@ -4,4 +4,4 @@` | OMITTED — (a), the living-renderer rows |
| `src/entities/render_living/state.rs` | `@@ -278,6 +278,10 @@`, `@@ -308,6 +312,10 @@`, `@@ -316,6 +324,37 @@`, `@@ -465,6 +504,9 @@`, `@@ -813,6 +855,14 @@` | OMITTED — (a) |
| `src/entities/render_special.rs` | `@@ -6,6 +6,7 @@`, `@@ -54,6 +55,7 @@`, `@@ -146,13 +148,26 @@`, `@@ -163,6 +178,10 @@` | OMITTED — (a), the special-renderer rows |
| `src/entities/render_special/kernels.rs` | `@@ -92,69 +92,3 @@` | OMITTED — (a) |
| `src/entities/render_special/timeline.rs` | `@@ -206,7 +206,7 @@`, `@@ -217,6 +217,9 @@` | OMITTED — (a) |
| `src/entities/render_special/world_fx.rs` | `@@ -0,0 +1,391 @@` | OMITTED — (a) |
| `src/entities/render_special/world_fx_tests.rs` | `@@ -0,0 +1,323 @@` | OMITTED — test file |
| `src/entities/solaris_models/tests.rs` | `@@ -470,3 +470,125 @@` | OMITTED — test hunk |
| `src/entities/source_state.rs` | `@@ -263,6 +263,35 @@`, `@@ -401,6 +430,11 @@` | OMITTED — (a), the weather/state rows |
| `src/ui/item_bar_tests.rs` | `@@ -0,0 +1,441 @@` | OMITTED — test file (MODEL-ITEM-BARS) |
| `src/ui/item_bars.rs` | `@@ -0,0 +1,137 @@` | NOT this row — reversed and re-landed in the earlier pair for MODEL-ITEM-BARS (its own map) |
| `src/ui/state.rs` | `@@ -20,6 +20,9 @@`, `@@ -4144,7 +4147,7 @@` | NOT this row — the `mod item_bars;` line and its call site (reason (b), API MODEL-ITEM-BARS's kept tests call); unchanged by either pair |
| `src/ui/state.rs` | `@@ -4671,3 +4674,7 @@` | OMITTED — test-module registration |
| `tools/entity_models.py` | `@@ -326,6 +326,9 @@` | OMITTED — (a), the rig asset tooling |
| `assets/solaris/models/**` (~120 files) | one `@@ -0,0 +1,N @@` or a modified-block `@@` each | OMITTED — (a): the rig asset set belongs to the model rows, not the back-layer draw path |

## a/-side adaptation

`e292b8f68e` and `c290f71b95` keep `append_player_back`'s signature, so the reversed body takes
`_e`, `_profile`, `_frame`, `_out` and returns `Ok(())`; `wings_asset` keeps its definition (the
retained test module calls it indirectly through the fixture). The re-land restores the real names
and body. No other text outside the pre-image is introduced: `git diff c290f71b95 b11ca37604` is
exactly the two hunks above.

## Proving ref

`harness:scene-shot:native-elytra` → `worn_elytra_drawn`
(`tools/harness/backends/scene_shot_checks.py`): the elytra frame must change the avatar's upper-back
ROI by more than 4000 px against the same player with an empty chest slot, with idle motion bounded
(< 2500 px). That is the row's symptom (no worn back layer). `chestplate_back_roi` and
`idle_back_motion_bounded` only bound the comparison and stay green on both sides.

Control and restore receipts (candidate core `c2cab51e74`, server pin `c2cab51e7404`; client pins
`c290f71b95` sha `dfc4a2920da5` and `b11ca37604` sha `a6b29408adac`):

- control `20261124T003244-native-elytra-ps_wnt_t` — FAIL `worn_elytra_drawn`;
  `elytra_changed_pixels = {chestplate: 12790, elytra: 336, idle: 835}`. With the guard also
  reversed the wings draw as a chestplate layer on both frames, so the back ROI barely changes.
- restore `20261124T003313-native-elytra-cbm2r4i1` — PASSED;
  `elytra_changed_pixels = {chestplate: 12444, elytra: 9448, idle: 79}`.
- promote `20261124T003824-ledger-promote-j54ozaac`; pair patch id `ea74507b…`; the client unit ref
  ran green in `ledger-1-client-player_back_uses_wings_a.log`.
