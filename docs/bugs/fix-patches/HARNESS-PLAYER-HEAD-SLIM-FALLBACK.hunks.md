# HARNESS-PLAYER-HEAD-SLIM-FALLBACK — patch map

Fix: `fad1cef1e` "fix(harness): exercise actual head skins and visible clock faces" (2026-11-15).
`player_head_skin_pack` wrote the authored offline Steve texture only to
`assets/minecraft/textures/entity/player/wide/steve.png`, but the pinned entity facts resolve an
unresolved placed head through `default_skins[6]` (slim), so the shim found no authored hat and the
slim head had no contrasting hat. The fix writes the same skin into both model directories.

Port: the function moved verbatim to `tools/harness/backends/scene_shot_checks.py` in
`1cadb5c0a` (module split), so the committed patch (`patch:a9527f08…`) is the HEAD equivalent of the
commit's one hunk in that file.

| original hunk (`fad1cef1e`) | HEAD equivalent | note |
|---|---|---|
| `tools/harness/backends/scene_shot.py` `@@ -1243,13 +1243,16 @@` — the single `texture = .../wide/steve.png` + `skin.save(texture)` → a `for model in ("slim", "wide")` loop | `tools/harness/backends/scene_shot_checks.py` `@@ -252,13 +252,16 @@` — the same loop | (c) `1cadb5c0a` moved the function; the statements are identical |
| `check_player_head`/`check_clock_face` hunks of the same commit | not in this patch | (a) `check_player_head` is HARNESS-WORN-HEAD-BACK-FALSE-POSITIVE's (its repro_check predicate is `worn_head_renders`), `check_clock_face` is HARNESS-DROPPED-CLOCK-HUD-ORACLE's; later commits rewrote both |
| the commit's test and docs hunks | — | tests kept at HEAD; docs never reversed |

Proving ref: `python:tools.harness.test_scene_shot:PlayerHeadTests.test_authored_skin_has_opaque_contrasting_hat_and_base_window`
at `tools/harness/test_scene_shot.py:673`. Control observed 2026-11-24 in a copy of this tree with the
patch reversed: `AssertionError: False is not true : slim default skin has no authored hat fixture`;
green at HEAD.

Promoted 2026-11-24 on `595f66d98` (`docs/bugs/promotions/HARNESS-PLAYER-HEAD-SLIM-FALLBACK.json`):
the python ref above is the recorded red control, the passed real-client receipt of
`harness:scene-shot:native-player-head` on the candidate is recorded as `client_receipt`, and that
scene's own control is `unrunnable` (the tree had no registered pin-release server), so the graphical
half is not claimed.
