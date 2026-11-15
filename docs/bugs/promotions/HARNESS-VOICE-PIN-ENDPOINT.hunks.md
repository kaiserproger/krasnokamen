# Forward-ported fix hunk map

Candidate base: `5315128eeb4cfcd19a6158db6498307ac674cad4`. The adjacent `.patch` is identical to `../fix-patches/HARNESS-VOICE-PIN-ENDPOINT.patch`. Digest is SHA-256 of `ledger._split_non_test(patch, source_of)[0]`, not a patch-id. No test hunks are reversed. No a/-side API adaptation is used.

Original commit: `ff1b494a01622e58b15c85da2897e368f402b65c`. Its non-test hunks are: audio.py +278 (private audio API and docstring), screens_voice_qa.py +131 (endpoint API) and +176 (live loop); runtime.py +955 (volume/device), +997 (private sink routing), +1028 (Gradle cache); screen_audio_graph.sh +31 (startup ordering and fail-closed cleanup).

screens_voice_qa.py +176 is retained verbatim: wrap the live leg with java_endpoint. The +131 API hunk is omitted (b): retained conflicting-selector tests call java_endpoint directly. That ref remains green and is not negative-control evidence. All audio.py, runtime.py and screen_audio_graph.sh hunks are omitted (a): separate sink/cache/startup rows.

Proving ref: `python:tools.harness.test_screens_voice_qa:ScreensVoiceHelperTests.test_run_scopes_pinned_endpoints_to_each_leg`; symptom assertion `tools/harness/test_screens_voice_qa.py:90`. Red/green output: `receipts/HarnessFix4/HARNESS-VOICE-PIN-ENDPOINT.red.log` and `.green.log`. The red is the child launch/selection assertion, not an import or setup error.
