# HARNESS-PLAYER-HEAD-HAT-UNOBSERVABLE — patch map

Fix in two commits. `f645ca39a` (2026-11-15) authored an offline Steve skin with an opaque magenta hat
layer, wired it into the `native-player-head` scene as a resource pack, named the heads `HarnessHat`
(unresolved default skin) instead of `jeb_`, and asserted `placed_head_hat_layer_pixels > 100`.
`fad1cef1e` (same day) saved the same skin for the `slim` model as well, because the pinned entity
facts select `slim/steve` for unresolved placed heads while the special-item path selects `wide/steve`.

The committed patch (`docs/bugs/fix-patches/HARNESS-PLAYER-HEAD-HAT-UNOBSERVABLE.patch`,
`patch:cd90d569…`) replaces the earlier stale `patch:d5503b10…` forward port (written against
`scene_shot.py` before the `1cadb5c0a` module split and before `fad1cef1e`; it no longer reversed).

| original hunk | HEAD equivalent | note |
|---|---|---|
| `f645ca39a` `scene_shot.py` `@@ -1222,13 +1241,47 @@`: new `player_head_skin_pack` (wide only) | `scene_shot_checks.py` `@@ -252,13 +252,16 @@`: the same function saving `slim` **and** `wide` | carried with `fad1cef1e`; the a/-side is `f645ca39a`'s wide-only block verbatim |
| `fad1cef1e` `scene_shot.py` `@@ -1243,13 +1243,16 @@`: the wide-only texture line → `for model in ("slim", "wide")` | the same hunk above | carried; (c) `1cadb5c0a` moved the function, the text is identical |
| `f645ca39a` `@@ -1222,13 +1241,47 @@`: `probe.check("placed_head_hat_layer_pixels", hat > 100)` in `check_player_head` | `scene_shot_checks.py` `@@ -376,6 +376,7 @@` | carried. `check_placed_head` (a later function for `native-placed-head`) keeps its own identical line |
| `f645ca39a` `@@ -1537,9 +1578,15 @@`: `old_packs` + the `if ctx.scenario_id == "native-player-head":` resource-pack block + the `finally` tuple entry | `scene_shot_scenario.py` `@@ -266,6 +266,7 @@`, `@@ -272,6 +272,11 @@`, `@@ -288,6 +288,7 @@`, where a later commit widened the condition to `("native-player-head", "native-placed-head")` | carried; (c) the widened tuple is HEAD's text, its a/-side is the pre-image with no pack path |
| `f645ca39a` `player_head_scene.json` `@@ -61,10 +61,10 @@`, `@@ -85,10 +85,10 @@`: `jeb_` → `HarnessHat` in all four profile commands | identical at HEAD | carried |
| `fad1cef1e` `@@ -1279,43 +1279,52 @@`: `worn_hat` + `worn_head_renders` | — | (a) HARNESS-WORN-HEAD-BACK-FALSE-POSITIVE, carried by its own patch |
| the bundled commits' other scene/clock/audio/test hunks | — | (a) other rows' behaviour and their tests |

a/-side adaptations: none. Every a/-side line is either the exact pre-image of the named commit (the
wide-only skin block, the absent pack wiring, `jeb_`) or HEAD's own text with the pack wiring removed.

Proving ref and line: `python:tools.harness.test_scene_shot:PlayerHeadTests.test_authored_skin_has_opaque_contrasting_hat_and_base_window`
red at `tools/harness/test_scene_shot.py:673` with `AssertionError: False is not true : slim default
skin has no authored hat fixture` (the restored wide-only pack). The row's second ref also goes red on
its pose-independent assertion (`.assertRaisesRegex(AssertionError, "placed_head_hat_layer_pixels")`
at `:572`) because the removed assertion no longer fires. Green at HEAD. Observed 2026-11-24.
