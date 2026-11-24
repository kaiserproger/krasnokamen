# HARNESS-WORN-HEAD-BACK-FALSE-POSITIVE — patch map

Fix: `fad1cef1e` "fix(harness): exercise actual head skins and visible clock faces" (2026-11-15), the
`worn_hat` half. The gate counted only changed pixels in the worn-head region, so unrelated avatar/pose
changes satisfied it while the authored magenta skull sat on the back; the fix requires authored hat
pixels inside the actual head ROI before the worn pose may pass.

The committed patch (`docs/bugs/fix-patches/HARNESS-WORN-HEAD-BACK-FALSE-POSITIVE.patch`,
`patch:281b5b44…`) is HEAD's equivalent of that half. `1cadb5c0a` moved `check_player_head` out of
`tools/harness/backends/scene_shot.py` into `tools/harness/backends/scene_shot_checks.py`; the two
added statements are byte-identical there apart from the ROI, which `98900fb5c`
(HARNESS-WORN-HEAD-ADJACENT-FALSE-POSITIVE, promoted as `patch:964a879c…`) tightened from
`(580, 280, 700, 400)` to `(600, 280, 680, 375)`. HEAD's own value is carried, so the reversal removes
the guard, not the later tightening.

| original hunk (`fad1cef1e`, `scene_shot.py`) | HEAD equivalent (`scene_shot_checks.py`) | note |
|---|---|---|
| `@@ -1276,43 +1279,52 @@` mixed hunk: adds `worn_hat = _count(shots["head-worn"][1], (580, 280, 700, 400), magenta)` + its report line | `@@ -357,6 +357,8 @@`: the same two statements with the tightened ROI `(600, 280, 680, 375)` | carried; the ROI value is another row's promoted change, the statements are this row's own |
| the same mixed hunk: `probe.check("worn_head_renders", worn > 500)` → `worn > 500 and worn_hat > 100` | `@@ -377,7 +377,7 @@`: identical | carried; the a/-side is the commit's own pre-image line |
| the same mixed hunk: the `player_head_skin_pack` rewrite, the `head_box`/ROI lines and the `check_dropped_clock` → `check_clock_face` rewrite | — | (a) other rows' behaviour: HAT-UNOBSERVABLE (skin pack, hat assertion), PLAYER-HEAD-ROI (`head_box`), DROPPED-CLOCK-HUD-ORACLE (clock oracle) |
| `@@ -1243,13 +1243,16 @@` skin pack slim+wide, `@@ -1332,9 +1332,9 @@` time-of-day, `@@ -1351,7 +1351,6 @@`/`@@ -1359,7 +1359,15 @@` held/shelf clocks | — | (a) the skin pack is HAT-UNOBSERVABLE's; the time and shelf/held regions are the dropped-clock fixture rows' |
| `tools/harness/dynamic_context_scene.json`, `test_scene_shot.py`, `real-client-gates.md` | — | fixture/tests/docs of the bundled commit, never reversed |

a/-side adaptation: none. The two added statements are deleted and the condition returns to the
commit's own `worn > 500`.

Proving ref and line: `python:tools.harness.test_scene_shot:PlayerHeadTests.test_worn_head_rejects_back_attachment_despite_unrelated_head_roi_changes`
red at `tools/harness/test_scene_shot.py:644` (`assertRaisesRegex(AssertionError, "worn_head_renders")`
reports `"worn_head_renders" does not match "worn_head_back_unchanged"`: the reversed gate passed the
worn pose and a pre-existing guard rejected it later). Green at HEAD. Observed 2026-11-24.

Honest limit: at HEAD the older `worn_head_back_unchanged` check (predating this fix) also rejects the
test frame, so the control proves that this row's guard fires *first* on the reported frame, not that
the frame would be accepted by an otherwise unguarded gate. The test and the assertion are this row's
own; no other row's ref is the proving one.
