# HARNESS-SCENE-SHOT-MISSING-SUBJECT — patch map

Fix: `259649f7c` "fix(harness): require scene capture subjects" (2026-11-15). A scene step
(`cmd: "summon minecraft:invalid"`) could fail to create its subject and the capture still ran, so a
scene shot of nothing passed. The fix adds a `require_entities` step: it lists the entities, records
the snapshot, fails closed when the snapshot omits entities, and asserts every declared type is
present — before any capture.

Port: `scene_shot.py` was split by `1cadb5c0a`; the execute branch now lives in
`tools/harness/backends/scene_shot_execute.py` and the validation/mcp branches in
`scene_shot_steps.py` (`validate_steps`, `scene_mcp_calls`). The committed patch
(`patch:…`) carries the HEAD equivalent of the **execute** branch, which is the guard the row names.

| original hunk (`259649f7c`) | HEAD equivalent | note |
|---|---|---|
| `tools/harness/backends/scene_shot.py` `@@ -672,6 +681,15 @@` — the `execute` branch that lists entities, raises `entity snapshot omitted entities`, and checks each type | `tools/harness/backends/scene_shot_execute.py` `@@ -410,9 +410,18 @@` (same block, same text) | (c) `1cadb5c0a` moved the function; the a/-side is the commit's own pre-image (absent) |
| `@@ -244,7 +244,7 @@` — `scene_mcp_calls` also yields the list call for `require_entities` | `tools/harness/backends/scene_shot_steps.py` `@@ -41,7 +41,7 @@`, now `("timed_entities", "require_entities", "assert_entities")` | (a) a later row added `assert_entities` to the same line; reversing it would also remove that row's branch |
| `@@ -357,6 +357,15 @@` — `validate_steps` accepted the step shape | `scene_shot_steps.py` `@@ -256,9 +256,18 @@` | (a) another row's `assert_entities` shares the resolver; the shape check alone does not red this row |
| the commit's test and docs hunks | — | tests kept at HEAD; docs never reversed |

Proving ref: `python:tools.harness.test_scene_shot:SceneShotTests.test_require_entities_fails_before_shot_when_summon_did_not_create_subject`.
Control observed 2026-11-24 in a copy with the patch reversed: the test fails
(`assertRaisesRegex(AssertionError, "subjects_minecraft:slime")` did not raise — the step is a no-op
and the capture runs); green at HEAD. The sibling
`test_require_entities_rejects_empty_or_invalid_type_lists` stays green either way (its ValueError
comes from the generic unknown-step refusal), which is why the execute branch is the narrowest control.

Missing proof for promotion: a passed graphical receipt (the row registers
`harness:scene-shot:issue1-01-villager-leg`, whose fixture uses `require_entities`) and its control on
the reversed tree. The scene stays green under this reversal (the summon succeeds in the fixture), so
the control's red is the python ref; record it as such.
