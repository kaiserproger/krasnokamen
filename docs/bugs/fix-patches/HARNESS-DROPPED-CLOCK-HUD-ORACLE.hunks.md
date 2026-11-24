# HARNESS-DROPPED-CLOCK-HUD-ORACLE — patch map

Fix: `f645ca39a` "fix(harness): repair stage-three launch and scene oracles" (2026-11-15), the dropped
clock oracle. The old check searched `(335, 640, 395, 700)` for dark "needle" pixels — a region that
overlaps the hotbar/chat — so a frame with no visible clock face could still pass by counting HUD
glyphs. The fix samples the gold rim's enclosed face and compares its blue pattern between the day and
night phases.

Three later commits rewrote the added oracle, so no original post-fix line exists at HEAD:
`fad1cef1e` replaced the blue fraction with an 8×8 normalized grid and moved the box,
`635d650e1` split out `_clock_face_samples`, and `1cadb5c0a` moved everything to
`tools/harness/backends/scene_shot_checks.py`. The committed patch
(`docs/bugs/fix-patches/HARNESS-DROPPED-CLOCK-HUD-ORACLE.patch`, `patch:91fa7d71…`) therefore carries
the HEAD equivalent of the replacement: the call site's current form.

| original hunk (`f645ca39a`, `scene_shot.py`) | HEAD equivalent (`scene_shot_checks.py`) | note |
|---|---|---|
| `@@ -1222,13 +1241,47 @@`: new `check_dropped_clock` (gold rim, blue fraction ≥ 0.15) | `@@ -501,19 +501,7 @@`: `check_clock_face(probe, day, night, "dropped_clock", (300, 260, 460, 420))` | carried: (c) the oracle's body was rewritten by `fad1cef1e`/`635d650e1` and moved by `1cadb5c0a`; the HEAD call is the forward port of the same decision, and its a/-side is the commit's verbatim pre-image block below |
| `@@ -1269,19 +1322,7 @@`: the removed dark-needle block (`drop_box = (335, 640, 395, 700)`, `axis_diff > 30.0`) | the a/-side of the hunk above, copied verbatim from `f645ca39a^` | carried: this is the real pre-image the fix replaced |
| `dynamic_context_scene.json` `@@ -52,7 +52,10 @@` (lift the clock, light it) | — | (a) HARNESS-CONTEXT-CLOCK-FIXTURE, already promoted as `patch:b23cdceb…` from `fad1cef1e` |
| `player_head_scene.json`, `audio.py`, `java_*.py`, `runtime.py` and the test/docs files | — | (a) other rows of the bundled commit and its tests |

a/-side adaptation: none. The restored block is the pre-fix text verbatim; its `axis_angle`/`axis_diff`
helpers still exist in `check_dynamic_context` for the compass check.

Proving ref and line: `python:tools.harness.test_scene_shot:DroppedClockTests.test_only_clock_face_change_passes_not_hud_or_static_clock`
red at `tools/harness/test_scene_shot.py:944`. With the pre-image oracle restored, the HUD-only day
frame is accepted (`dropped_clock_face_day` does **not** fire, which is the row's symptom), the run
continues and `assertRaisesRegex` reports `"dropped_clock_face_day" does not match
"held_clock_face_day"` — the held-clock assertion added by the later `fad1cef1e` hunk. The failing
expectation, the test and the file are this row's own; no other row's ref is the proving one. Green at
HEAD. Observed 2026-11-24.
