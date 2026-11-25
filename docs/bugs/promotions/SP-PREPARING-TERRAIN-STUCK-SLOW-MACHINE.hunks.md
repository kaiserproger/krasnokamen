# SP-PREPARING-TERRAIN-STUCK-SLOW-MACHINE — hunk map

The fix is the same pair already promoted for
`NATIVE-INPLAY-BLOCKED-BY-MISSING-CHUNK`, so the hunk map is that file:

- `docs/bugs/promotions/NATIVE-INPLAY-BLOCKED-BY-MISSING-CHUNK.hunks.md`

Pair: **A (control)** `client:30183eef7bd61858524ca29a7f53dcc89b39ef5b` (the exact pre-`803f0e00`
body of `Session::play_world_presentation_ready`, byte-identical to
`git show 803f0e00^:src/session/observation.rs`) → **B (fix_commit)**
`client:fdc19309818760c2086494acabf2ed9108702f3b` (the re-land).

`git diff A B` = `non_test_patch(B)`, patch-id `ff511a9d83b149fcd2ba255f29f8bf786397ab70`.
Non-test hunks: one, in `src/session/observation.rs` (`@@ -505,20 +505,10 @@` at A, the function
body). Test hunks of `803f0e00`/`108c8b2a` are excluded by `non_test_patch`. No a/-side adaptation.

## This row's scene change (core, not part of the client fix)

`tools/harness/backends/singleplayer_regressions.py` (scenario `native-sp-slow-machine`):

- the play wait is now bounded at `min(300 s, remaining)` and keeps the observation on timeout
  instead of erroring, so a client that never releases the gate produces an assertion, not a
  timeout;
- new assertion `loading_overlay_gate_released`: `loading_readiness.world_ready` true. The overlay
  text is rendered from the same value (`Session::sync_ui_state` takes
  `play_world_presentation_ready`), and the observation's `overlay` field is a stub, so this is the
  discriminating read for the row's symptom; the older `terrain_overlay_cleared` only reads
  `screen.open` and cannot see a play-phase overlay.

Proving assertion: `loading_overlay_gate_released` (red on A, green on B).
