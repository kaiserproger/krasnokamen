# HARNESS-GAIT-ARTIFACT-REUSE promotion hunk map

Original fix: `f645ca39ad243e17008dcc5d2b3661e6d2d473f7`.
The forward patch `docs/bugs/fix-patches/HARNESS-GAIT-ARTIFACT-REUSE.patch`
retains both original `tools/harness/backends/scene_shot.py` hunks at old
lines 723 and 732: unique capture names and retained image/sidecar paths.
The executor moved to `scene_shot_execute.py`; these are their current
behavioral equivalents. No a-side adaptation is used.

Excluded original non-test hunks, reason (a), unrelated behavior: `audio.py`
lines 45 and 50 (sound sinks); `scene_shot.py` lines 1213–1600 (head rendering,
dynamic context and launch); `dynamic_context_scene.json` line 55 and
`player_head_scene.json` lines 64–91 (other scenarios); `java_clients.py`
lines 23–387, `java_launcher.py` lines 172–217, `runtime.py` lines 71–1038
and `vanilla_world_roundtrip.py` line 130 (other launch/roundtrip behavior).
Documentation and test hunks are excluded by the ledger control convention.

Proving ref: `python:tools.harness.test_scene_shot:WalkCaptureArtifactsTests.test_each_retry_owns_an_artifact_and_misses_are_bounded`.
Its `test_scene_shot.py` assertion on the caught `FileExistsError` proves the
original exclusive-artifact collision, not a launch or setup failure.
