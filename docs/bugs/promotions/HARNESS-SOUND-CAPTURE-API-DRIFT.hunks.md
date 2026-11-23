# HARNESS-SOUND-CAPTURE-API-DRIFT promotion hunk map

Original fix: `f645ca39ad243e17008dcc5d2b3661e6d2d473f7`.
The forward patch `docs/bugs/fix-patches/HARNESS-SOUND-CAPTURE-API-DRIFT.patch`
retains both original `tools/harness/audio.py` hunks at old lines 45 and 50:
the named sink/description constructor and per-sink monitor artifact.
They remain the equivalent current behavior. No a-side adaptation is used.

Excluded original non-test hunks, reason (a), unrelated behavior:
`scene_shot.py` lines 723–1600 (gait, head, dynamic context and launch);
`dynamic_context_scene.json` line 55 and `player_head_scene.json` lines 64–91
(other scenarios); `java_clients.py` lines 23–387, `java_launcher.py` lines
172–217, `runtime.py` lines 71–1038 and `vanilla_world_roundtrip.py` line 130
(other launch/roundtrip behavior). Documentation and test hunks are excluded
by the ledger control convention.

Proving ref: `python:tools.harness.test_private_audio:SoundCaptureApiTest.test_backend_named_sinks_have_distinct_monitors_and_artifacts`.
Its `test_private_audio.py` assertion on `TypeError` for unsupported `sink`
proves the original constructor failure. Audio subprocesses are mocked; this
does not claim an actual audio graph or screen/voice client acceptance.
