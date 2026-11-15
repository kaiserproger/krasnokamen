# JAVA-BATTLE-TAKE-END-CAMERA: forward-ported hunk map

Base: `5315128eeb4cfcd19a6158db6498307ac674cad4`. The adjacent patch is identical to `../fix-patches/JAVA-BATTLE-TAKE-END-CAMERA.patch`; digest uses `ledger._split_non_test`. Tests/docs stay in the control. No invented pre-image or a/-side behavior adaptation.

run-battle.py's final +900 hunk is retained at tools/battle/run-battle.py:903–905: wait before pause/stop. The watch_ready signature/ready-marker/error-label hunks (+522/+533/+542) are retained at HEAD :525–552. The argument declaration/validation (+83/+130) remains as kept CLI API (b): retained subprocess tests invoke --completion-timeout; deleting it only produces an argument error. Dry-run explanation (+739) stays (a), the separate informational CLI behavior, not shutdown ordering. Proving ref: the holds_shutdown test, tools/battle/test_run_battle.py:619 (observed stop before capture release), not the ready marker or process exit itself. Early completion.done is legal; the row does not claim all captures happen after completion.ready.

## Every original non-test hunk

The dispositions above apply to each listed hunk, including mixed hunks at sub-hunk granularity.

| Commit | Original path/hunk |
| --- | --- |
| `5fd8de1de` | `tools/battle/run-battle.py` `@@ -83,6 +83,7 @@ def arguments():` |
| `5fd8de1de` | `tools/battle/run-battle.py` `@@ -129,6 +130,8 @@ def arguments():` |
| `5fd8de1de` | `tools/battle/run-battle.py` `@@ -519,7 +522,7 @@ def read_pin(pin):` |
| `5fd8de1de` | `tools/battle/run-battle.py` `@@ -530,6 +533,8 @@ def watch_ready(path, process, timeout=180):` |
| `5fd8de1de` | `tools/battle/run-battle.py` `@@ -537,14 +542,14 @@ def watch_ready(path, process, timeout=180):` |
| `5fd8de1de` | `tools/battle/run-battle.py` `@@ -734,6 +739,7 @@ def run(a):` |
| `5fd8de1de` | `tools/battle/run-battle.py` `@@ -894,6 +900,9 @@ def run(a):` |

## Proof

Registered proving refs are run with retained tests on HEAD, with this patch reverse-applied and then restored. Receipts: `receipts/HarnessFix4/JAVA-BATTLE-TAKE-END-CAMERA.red.log` and `.green.log`. Only the symptom assertion named above carries the red; other failures are not evidence. The row stays open until Main runs ledger promote and independent review.
