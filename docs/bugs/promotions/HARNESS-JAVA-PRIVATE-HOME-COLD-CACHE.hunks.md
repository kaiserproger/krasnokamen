# Forward-ported fix hunk map

Candidate base: `5315128eeb4cfcd19a6158db6498307ac674cad4`. The adjacent `.patch` is identical to `../fix-patches/HARNESS-JAVA-PRIVATE-HOME-COLD-CACHE.patch`. Digest is SHA-256 of `ledger._split_non_test(patch, source_of)[0]`, not a patch-id. No test hunks are reversed. No a/-side API adaptation is used.

Original commit: `ff1b494a01622e58b15c85da2897e368f402b65c`. Its non-test hunks are: audio.py +278 (private audio API and docstring), screens_voice_qa.py +131 (endpoint API) and +176 (live loop); runtime.py +955 (volume/device), +997 (private sink routing), +1028 (Gradle cache); screen_audio_graph.sh +31 (startup ordering and fail-closed cleanup).

runtime.py +1028 is retained verbatim at tools/harness/runtime.py (GRADLE_USER_HOME assignment). All other original hunks are omitted (a): audio startup, audible sink and endpoint selection belong to their separate rows.

Proving ref: `python:tools.harness.test_loader_v2:LoaderV2Tests.test_client_launch_reuses_gradle_cache_despite_private_home`; symptom assertion `tools/harness/test_loader_v2.py:290`. Red/green output: `receipts/HarnessFix4/HARNESS-JAVA-PRIVATE-HOME-COLD-CACHE.red.log` and `.green.log`. The red is the child launch/selection assertion, not an import or setup error.
