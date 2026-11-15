# Forward-ported fix hunk map

Candidate base: `5315128eeb4cfcd19a6158db6498307ac674cad4`. The adjacent `.patch` is identical to `../fix-patches/HARNESS-AUDIBLE-SINK-IGNORED.patch`. Digest is SHA-256 of `ledger._split_non_test(patch, source_of)[0]`, not a patch-id. No test hunks are reversed. No a/-side API adaptation is used.

Original commit: `ff1b494a01622e58b15c85da2897e368f402b65c`. Its non-test hunks are: audio.py +278 (private audio API and docstring), screens_voice_qa.py +131 (endpoint API) and +176 (live loop); runtime.py +955 (volume/device), +997 (private sink routing), +1028 (Gradle cache); screen_audio_graph.sh +31 (startup ordering and fail-closed cleanup).

runtime.py +955 and +997 are retained verbatim: soundDevice/master volume and the private sink branch. audio.py +278 is omitted (b): retained tests call require_java_private_audio; its direct API ref is not negative-control evidence. Both screens_voice_qa.py hunks, runtime.py +1028 and screen_audio_graph.sh +31 are omitted (a): endpoint/cache/startup rows.

Proving ref: `python:tools.harness.test_runtime_profiles:ProfileTaskTests.test_audible_client_uses_only_verified_named_private_sink`; symptom assertion `tools/harness/test_runtime_profiles.py:380`. Red/green output: `receipts/HarnessFix4/HARNESS-AUDIBLE-SINK-IGNORED.red.log` and `.green.log`. The red is the child launch/selection assertion, not an import or setup error.

The refused-before-spawn ref errors in the control after the removed guard lets its mocked child proceed; that ERROR is not evidence. Only the null-versus-pulse assertion above carries this control. All three refs pass restored.
