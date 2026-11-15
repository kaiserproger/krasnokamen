# Forward-ported fix hunk map

Candidate base: `ff72f53bde64aac80ca791226f0bb65daabb4f10`. The adjacent `.patch` is identical to `../fix-patches/HARNESS-VOICE-PIN-ENDPOINT.patch`. Digest is SHA-256 of `ledger._split_non_test(patch, source_of)[0]`, not a patch-id. No test hunks are reversed. No a/-side API adaptation is used.

Original commit: `ff1b494a01622e58b15c85da2897e368f402b65c`. Its non-test hunks are: audio.py +278 (private audio API and docstring), screens_voice_qa.py +131 (endpoint API) and +176 (live loop); runtime.py +955 (volume/device), +997 (private sink routing), +1028 (Gradle cache); screen_audio_graph.sh +31 (startup ordering and fail-closed cleanup).

screens_voice_qa.py +176 is retained verbatim: wrap the live leg with java_endpoint. The +131 API hunk is omitted (b): retained conflicting-selector tests call java_endpoint directly. That ref remains green and is not negative-control evidence. All audio.py, runtime.py and screen_audio_graph.sh hunks are omitted (a): separate sink/cache/startup rows.

Proving ref: `python:tools.harness.test_screens_voice_qa:ScreensVoiceHelperTests.test_voice_leg_launches_with_svc_pinned_version`; symptom assertion `tools/harness/test_screens_voice_qa.py:103`. It selects only voice, runs the real leg scoping path, and asserts the child Minecraft endpoint; a preceding screens failure cannot mask it. Red/green output: `receipts/HarnessFix4Proof/voice.red.log` and `voice.green.log`. The red says voice lacks `-PminecraftVersion=26.1.2`, not an import/setup error.
