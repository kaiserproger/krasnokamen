# WORN-HEAD-ADJACENT-FALSE-POSITIVE — patch map

Fix: `98900fb5c` "fix(harness): reject skulls adjacent to the actual head" (2026-11-15).
The committed patch (`docs/bugs/fix-patches/WORN-HEAD-ADJACENT-FALSE-POSITIVE.patch`,
`patch:964a879c…`) is HEAD's equivalent of that commit's production hunk. The fix predates
`1cadb5c0a` "refactor(harness): split buildings-qa and scene-shot into owner modules", which moved
`check_player_head` out of `tools/harness/backends/scene_shot.py`; the ROI statement is byte-identical
in its new file, so the pre-image is the commit's own line and not a hand re-break.

| original hunk (`98900fb5c`) | HEAD equivalent | note |
|---|---|---|
| `tools/harness/backends/scene_shot.py` `@@ -1279,7 +1279,7 @@` — `worn_hat` ROI `(580, 280, 700, 400)` → `(600, 280, 680, 375)` | `tools/harness/backends/scene_shot_checks.py` `@@ -357,7 +357,7 @@` — the same single statement | (c) the function moved by the module split `1cadb5c0a`; the text is identical, so the reversal restores the commit's own broad ROI |
| `tools/harness/test_scene_shot.py` — added `PlayerHeadTests.test_worn_head_rejects_adjacent_skull_clipping_the_old_broad_roi` | present at HEAD | kept (not in the patch): it is the row's proving ref. With the patch reversed it fails at `test_scene_shot.py:662` (`assertRaisesRegex(AssertionError, "worn_head_renders")` did not raise: the broad ROI counts the neighbouring skull's 743 magenta pixels); on HEAD it passes. |
| `docs/development/real-client-gates.md` | — | docs: never reversed |

Proving ref and line: `python:tools.harness.test_scene_shot:PlayerHeadTests.test_worn_head_rejects_adjacent_skull_clipping_the_old_broad_roi`
at `tools/harness/test_scene_shot.py:662`. Control observed 2026-11-24 in a copy of this tree with the
patch reversed (`git apply -R`), red before any edit and green at HEAD.

Missing proof for promotion: a passed real-client receipt of `harness:scene-shot:native-player-head` on
the candidate plus the graphical control on the reversed tree (rule 2/3), i.e. Main's scene sweep.
