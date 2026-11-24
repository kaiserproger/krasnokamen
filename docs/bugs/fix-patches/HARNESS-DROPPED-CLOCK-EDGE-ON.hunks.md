# HARNESS-DROPPED-CLOCK-EDGE-ON — patch map

Fix: `635d650e1` "fix(harness): sample dropped clock until its face is observable" (2026-11-15). The
dropped item's yaw decides whether its face is visible at all: a single capture could land on an
edge-on pose, so the day/night oracle saw zero (or 35) sampled cells and the gate blamed the render.
The fix replaces the two plain `shot` steps with `dropped_clock_capture`, which samples up to 32
frames (4 ticks apart) until the gold-rimmed face is fully observable, then copies the selected frame
and its same-frame sidecar.

The committed patch (`docs/bugs/fix-patches/HARNESS-DROPPED-CLOCK-EDGE-ON.patch`,
`patch:f720ec08…`) is HEAD's equivalent. `1cadb5c0a` split the runtime: the step validation and MCP
call plan live in `scene_shot_steps.py`, the executor branch in `scene_shot_execute.py`, and
`_capture_dropped_clock` in `scene_shot_checks.py` (its `capture`/`_load_shots` calls are spelled
`_qa.` there). An earlier port (`patch:a8aa87b9…`) removed the function but not
`scene_shot_execute.py`'s import of it, so its reversed tree died at `ImportError` and the promotion
refused (`20261124T123706-ledger-promote-b3alh9et`: unrunnable, not red). The import hunk below is what
makes the reversal land on an assertion; `patch:f720ec08` is the promoted digest.

| original hunk (`635d650e1`, `scene_shot.py`) | HEAD equivalent | note |
|---|---|---|
| `dynamic_context_scene.json` `@@ -79,7 +79,7 @@` and `@@ -88,7 +88,7 @@`: `"shot": "ctx-day"`/`"ctx-night"` → `"dropped_clock_capture"` | identical at HEAD | carried; the a/-side is the commit's own pre-image |
| `@@ -287,6 +289,8 @@ scene_mcp_calls`: `dropped_clock_capture` → `minecraft_wait_ticks 4` | `scene_shot_steps.py` `@@ -54,6 +54,8 @@` | carried |
| `@@ -424,6 +426,8 @@ validate_steps`: `native` and `("ctx-day", "ctx-night")` | `scene_shot_steps.py` `@@ -193,6 +193,8 @@` | carried |
| `@@ -633,6 +637,8 @@ execute`: the `dropped_clock_capture` branch | `scene_shot_execute.py` `@@ -53,6 +53,8 @@` | carried |
| `@@ -1302,31 +1308,65 @@`: new `_capture_dropped_clock` | `scene_shot_checks.py`: the function at HEAD, `_qa.capture_native`/`_qa._load_shots` | carried; the a/-side is its removal (the pre-image had no such function) |
| (the same function's import) | `scene_shot_execute.py` `@@ -16,6 +16,7 @@`: `from .scene_shot_checks import _capture_dropped_clock` | carried; the pre-image imported nothing from the checks module, and leaving it behind makes the reversed tree fail at import instead of at an assertion (the first port `patch:a8aa87b9` did exactly that and the promotion refused) |
| the same hunk: the `check_clock_face` refactor into `_clock_face_samples` | — | (a) the oracle body is HARNESS-DROPPED-CLOCK-HUD-ORACLE's and the held-clock face math is HARNESS-CONTEXT-CLOCK-FIXTURE's |
| `test_scene_shot.py` additions | — | the row's kept tests, never reversed |

a/-side adaptations: none.

Proving ref and line: `python:tools.harness.test_scene_shot:DroppedClockTests.test_clock_scene_samples_past_edge_on_without_changing_oracle`
red at `tools/harness/test_scene_shot.py:825` (the test's probe raises `AssertionError:
dropped_clock_face_day` from `check_clock_face` once the edge-on frame is captured as the only shot).
The row's other two refs — `test_clock_capture_is_bounded_and_does_not_accept_hud_only` and
`test_clock_capture_selects_static_face_but_animation_oracle_rejects_it` — are **unrunnable** in the
control, not red (`promotions/HARNESS-DROPPED-CLOCK-EDGE-ON.json` records both that way): with the
`dropped_clock_capture` step reverted their assertions are never reached. Green at HEAD.
Observed 2026-11-24.
