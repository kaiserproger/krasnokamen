# HARNESS-GAIT-SPIDER-FIXTURE-OCCLUDED — patch map

Fix: `b6f1ac655` "fix(harness): launch Java matrix clients through isolated runtime" (2026-11-15) —
the `gait_phase_scene.json` half. The spider was summoned into the camera lane the dying zombie had
occupied, so the zombie's death particles tinted the spider's ROI and the gait oracle read the wrong
subject. The fix moves the observer to `(1010.5, 160, 1000.5)` looking down +Z (yaw/pitch 0) and
summons the spider at `(1010.5, 160, 1008.5)`, which puts the zombie lane outside the 35° view.

Port: the fixture file is unchanged since the fix except for later steps after the spider capture, so
the committed patch (`patch:1601243a…`) is the commit's own one-hunk pre-image (no held-capture steps
involved).

| original hunk (`b6f1ac655`) | HEAD equivalent | note |
|---|---|---|
| `tools/harness/gait_phase_scene.json` `@@ -94,7 +94,16 @@` — add the `tp` step and move the spider summon from `1000.5` to `1010.5` | same lines at HEAD | verbatim, the hunk still reverses on HEAD |
| the commit's `java_launcher.py`/`runtime.py`/`java_clients.py` hunks | not in this patch | (a) they are the row's Gradle/runtime isolation half, owned by HARNESS-LOADER-SHARED-BUILD-DIR-RACE, not this fixture row |
| the commit's test and docs hunks | — | tests kept at HEAD; docs never reversed |

Proving ref: `python:tools.harness.test_scene_shot:SceneShotTests.test_gait_fixture_keeps_previous_subject_outside_spider_view`.
Control observed 2026-11-24 in a copy with the patch reversed: `AssertionError: -2.0 not greater than
5.601660305677678` (the zombie lane is inside the spider's view again); green at HEAD.

Missing proof for promotion: a passed `harness:scene-shot:native-gait-spider` receipt on the candidate
plus that scene on the reversed tree.
