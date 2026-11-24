# HARNESS-PLAYER-HEAD-ROI — patch map

Fix: `f645ca39a` "fix(harness): repair stage-three launch and scene oracles" (2026-11-15), the placed
head ROI. `check_player_head` measured the placed head in `(880, 520, 1080, 680)`, an empty region of
the fixture (the head projects to `x593..687/y534..641`), so "placed head renders" could never be
observed.

The committed patch (`docs/bugs/fix-patches/HARNESS-PLAYER-HEAD-ROI.patch`, `patch:2a1b9ac7…`) is
HEAD's equivalent of that hunk. `1cadb5c0a` moved the function to
`tools/harness/backends/scene_shot_checks.py`; a later commit added the separate
`check_placed_head` for `native-placed-head`, which has its own literal and is untouched.

| original hunk (`f645ca39a`, `scene_shot.py`) | HEAD equivalent (`scene_shot_checks.py`) | note |
|---|---|---|
| `@@ -1222,13 +1241,47 @@`: `placed = _changed(…, (880, 520, 1080, 680))` → `head_box = (580, 520, 700, 655)` plus the three geometry comment lines and `placed = _changed(…, head_box)` | `@@ -351,7 +351,7 @@` in `check_player_head`: `head_box = (580, 520, 700, 655)` | carried as the ROI value change; see the adaptation note |
| the same mixed hunk: `magenta`, `hat`, `placed_head_hat_layer_pixels`/`placed_head_roi` reports and the hat assertion | — | (a) HARNESS-PLAYER-HEAD-HAT-UNOBSERVABLE's behaviour |
| the same mixed hunk: `player_head_skin_pack` and `check_dropped_clock` | — | (a) HAT-UNOBSERVABLE and DROPPED-CLOCK-HUD-ORACLE |
| `dynamic_context_scene.json`, `player_head_scene.json`, other files of the commit | — | (a) the bundled rows' fixtures |
| the three geometry comment lines | — | (c) a later commit rewrote them; the head position they document is unchanged |

a/-side adaptation: the reversed tree keeps `head_box = (880, 520, 1080, 680)` — the commit's own
pre-image ROI literal — because the `hat` count, the `placed_head_roi` report and the ROI row's own
ref all use `head_box` in the kept lines (a surviving half of a mixed hunk after an (a) split). It
changes no behaviour beyond restoring the pre-fix region.

Proving ref and line: `python:tools.harness.test_scene_shot:PlayerHeadTests.test_placed_head_roi_covers_fixture_and_requires_hat_pixels`
red at `tools/harness/test_scene_shot.py:568` through the test probe at `:542` with
`AssertionError: placed_head_renders` (the fixture head is outside the restored region). Green at
HEAD. Observed 2026-11-24.
