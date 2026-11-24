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
| `fad1cef1e` `scene_shot.py` `@@ -1243,13 +1243,16 @@`: the wide-only texture line → `for model in ("slim", "wide")` | the same hunk above | carried; (c) `1cadb5c0a` moved the function, the text is identical. **The `slim` half is HARNESS-PLAYER-HEAD-SLIM-FALLBACK's behaviour** (its own promoted port is `patch:a9527f08…`); it stays in this patch only because it shares the hunk and the scene fixture with the hat row |
| `f645ca39a` `@@ -1222,13 +1241,47 @@`: `probe.check("placed_head_hat_layer_pixels", hat > 100)` in `check_player_head` | `scene_shot_checks.py` `@@ -376,6 +376,7 @@` | carried. `check_placed_head` (a later function for `native-placed-head`) keeps its own identical line |
| `f645ca39a` `@@ -1537,9 +1578,15 @@`: `old_packs` + the `if ctx.scenario_id == "native-player-head":` resource-pack block + the `finally` tuple entry | `scene_shot_scenario.py` `@@ -266,6 +266,7 @@`, `@@ -272,6 +272,11 @@`, `@@ -288,6 +288,7 @@`, where a later commit widened the condition to `("native-player-head", "native-placed-head")` | carried; (c) the widened tuple is HEAD's text, its a/-side is the pre-image with no pack path. Reversing it also removes the pack for `native-placed-head` (MODEL-PLACED-HEAD-PROFILE's scene): that is another row's behaviour at HEAD — (a) — and harmless here because HAT registers no `native-placed-head` ref |
| `f645ca39a` `player_head_scene.json` `@@ -61,10 +61,10 @@`, `@@ -85,10 +85,10 @@`: `jeb_` → `HarnessHat` in all four profile commands | identical at HEAD | carried |
| `fad1cef1e` `@@ -1279,43 +1279,52 @@`: `worn_hat` + `worn_head_renders` | — | (a) HARNESS-WORN-HEAD-BACK-FALSE-POSITIVE, carried by its own patch |
| the bundled commits' other scene/clock/audio/test hunks | — | (a) other rows' behaviour and their tests |

a/-side adaptations: none. Every a/-side line is either the exact pre-image of the named commit (the
wide-only skin block, the absent pack wiring, `jeb_`) or HEAD's own text with the pack wiring removed.

## Proving ref and line

`python:tools.harness.test_scene_shot:PlayerHeadTests.test_placed_head_roi_covers_fixture_and_requires_hat_pixels`
at `tools/harness/test_scene_shot.py:572` — `assertRaisesRegex(AssertionError, "placed_head_hat_layer_pixels")`
no longer raises once the assertion this fix added is reversed. That is this row's own assertion.
Red observed 2026-11-24; green at HEAD.

The same test's `:673` (`slim default skin has no authored hat fixture`) also goes red under the
reversal, but that is **HARNESS-PLAYER-HEAD-SLIM-FALLBACK's assertion**, not this row's evidence: it
fires because the patch carries `fad1cef1e`'s slim+wide loop (SLIM's promoted hunk), not because of the
hat wiring. It is recorded here as SLIM's, and the promotion JSON's red list for the slim test is not
read as HAT's proof.

## Graphical control (a real run, cross-row red)

`ledger graphical-control` ran the scene on the pinned reversed tree (`20261124T125905-ledger-graphical-control-bq8iqdad`,
tree `8ba13456…` = this promotion's `control_tree_sha256`, pin
`pins/302dd7221721-8ba13456-server-…`): status `failed`, `scene-shot assertion failed:
worn_head_renders`. That predicate belongs to HARNESS-WORN-HEAD-BACK-FALSE-POSITIVE — the reversal
removes this row's own `placed_head_hat_layer_pixels` check and the pack wiring, so the first
hat-dependent assertion left in `check_player_head` is WORN-BACK's. **The graphical red is therefore
cross-row and is not claimed as this row's assertion.** The row's symptom is nevertheless in the same
receipts: with the pack gone the frames report `placed_head_hat_layer_pixels` **0** and
`worn_head_hat_layer_pixels` **0**, against **9822** and **6647** on the candidate
(`20261124T123409-native-player-head-m2siodqi/scene-shot/scenario.json`). The row's own graphical proof
would need a control tree whose pack wiring is reversed but whose delivered skin is authored; none
exists, so the promotion rests on the python `:572` red plus these numbers.
