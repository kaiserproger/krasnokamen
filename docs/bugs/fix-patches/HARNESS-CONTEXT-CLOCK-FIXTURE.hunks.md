# HARNESS-CONTEXT-CLOCK-FIXTURE — patch map

Fix: `fad1cef1e` "fix(harness): exercise actual head skins and visible clock faces" (2026-11-15) — the
`dynamic_context_scene.json` half. The dropped clock hung under the HUD/chat area, so the oracle's
search box saw HUD pixels; the fix lifts the player above the dropped item looking down 37° and adds
the held-day/held-night captures, and puts a clock in the right shelf's `Slot:0b`.

Port: two of the commit's four json hunks were rewritten by later commits
(`635d650e1` inserted `dropped_clock_capture` steps before the held block, `f645ca39a` changed the
time/framing region), so the committed patch (`patch:b23cdceb…`) carries the HEAD equivalent of the
two that still express this row's fixture: the right-shelf `Slot:0b` clock line and the whole
held/`tp` block at HEAD's positions. Both a/-sides are the commit's own pre-image (`Slot:1b` only;
no held shots).

| original hunk | HEAD equivalent | note |
|---|---|---|
| `dynamic_context_scene.json` `@@ -49,7 +49,7 @@` (right shelf gains `Slot:0b` clock) | same line at HEAD `@@ -49,7 +49,7 @@` | verbatim, the hunk still reverses |
| `dynamic_context_scene.json` `@@ -91,13 +91,40 @@` (the held block: `tp [1000.5,162,1001.5,10,37]`, `held-day`, `time set 18000`, `held-night`) | HEAD `@@ -99,5 +99,32 @@` | rewritten position: `635d650e1` inserted `dropped_clock_capture` steps before it; the content is byte-identical |
| the commit's `time set 6000/13000/18000` and framing hunks | not in this patch | (c) later commits own the current time/framing region; its units are other rows' (DROPPED-CLOCK-*) |
| the commit's `check_clock_face` rewrite | not in this patch | (a) the oracle was rewritten again by `635d650e1` and belongs to HARNESS-DROPPED-CLOCK-HUD-ORACLE |

Proving ref: `python:tools.harness.test_scene_shot:DroppedClockTests.test_fixture_lifts_and_lights_dropped_clock_away_from_hud`
at `tools/harness/test_scene_shot.py:971`. Control observed 2026-11-24 in a copy with the patch
reversed: the right shelf no longer holds the `Slot:0b` clock (the same assertion also covers the tp
pose and held shots); green at HEAD.

Missing proof for promotion: a passed `harness:scene-shot:native-dynamic-context` receipt on the
candidate plus its control on the reversed tree. Note the control's scene is unrunnable there (the
held `_load_shots` names vanish with the block), so the red is the python ref; record it as such.
