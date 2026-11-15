# TEST-END-RESTART-OBSERVER: forward-ported hunk map

Base: `5315128eeb4cfcd19a6158db6498307ac674cad4`. The adjacent patch is identical to `../fix-patches/TEST-END-RESTART-OBSERVER.patch`; digest uses `ledger._split_non_test`. Tests/docs stay in the control. No invented pre-image or a/-side behavior adaptation.

The original new protocol_end_restart.py hunk supplies both the kept API and the observer movement fix. Retain the observer-side teleport/comment/report subrange at tools/harness/backends/protocol_end_restart.py:46–52. Remaining run/_dragon API and its identity/log/eight-client validators stay (b): kept tests call this API; deleting the file is an import failure. protocol_matrix capability registration and protocol_lifecycle support stay (b) for that runnable API. Other original paths in the table are omitted (a): native health, Java journal, bow/input/ACK and regression registry behavior. Proving ref test_restart_moves_observers_beside_the_dragon_and_never_the_dragon: tools/harness/test_end_restart.py:74–78,103–106. The observer now actually cannot see the post-restart dragon until its own teleport; reversal fails the recovered-dragon assertion, not an import. No a/-side adaptation.

## Every original non-test hunk

The dispositions above apply to each listed hunk, including mixed hunks at sub-hunk granularity.

| Commit | Original path/hunk |
| --- | --- |
| `ef67d139e` | `tools/harness/backends/native_matrix.py` `@@ -11,10 +11,12 @@ import hashlib` |
| `ef67d139e` | `tools/harness/backends/native_matrix.py` `@@ -44,6 +46,34 @@ def transport_blocker(scenario: dict[str, Any]) -> str | None:` |
| `ef67d139e` | `tools/harness/backends/native_matrix.py` `@@ -115,6 +145,84 @@ def _write_summary(matrix: dict[str,Any]) -> None:` |
| `ef67d139e` | `tools/harness/backends/native_matrix.py` `@@ -137,6 +245,7 @@ class Lane:` |
| `ef67d139e` | `tools/harness/backends/native_matrix.py` `@@ -162,10 +271,17 @@ class Lane:` |
| `ef67d139e` | `tools/harness/backends/native_matrix.py` `@@ -227,8 +343,17 @@ class Lane:` |
| `ef67d139e` | `tools/harness/backends/native_matrix.py` `@@ -477,17 +602,21 @@ def storage(lane: Lane, scenario: dict[str, Any]) -> None:` |
| `ef67d139e` | `tools/harness/backends/native_matrix.py` `@@ -520,6 +649,13 @@ def run(ctx: Any) -> dict[str, Any]:` |
| `ef67d139e` | `tools/harness/backends/native_matrix.py` `@@ -595,6 +731,11 @@ def run(ctx: Any) -> dict[str, Any]:` |
| `ef67d139e` | `tools/harness/backends/native_matrix.py` `@@ -615,15 +756,21 @@ def run(ctx: Any) -> dict[str, Any]:` |
| `ef67d139e` | `tools/harness/backends/protocol_end_restart.py` `@@ -0,0 +1,77 @@` |
| `ef67d139e` | `tools/harness/backends/protocol_lifecycle.py` `@@ -369,9 +369,9 @@ seed = {self.seed}` |
| `ef67d139e` | `tools/harness/backends/protocol_lifecycle.py` `@@ -387,7 +387,7 @@ seed = {self.seed}` |
| `ef67d139e` | `tools/harness/backends/protocol_lifecycle.py` `@@ -397,7 +397,8 @@ seed = {self.seed}` |
| `ef67d139e` | `tools/harness/backends/protocol_matrix.py` `@@ -20,7 +20,8 @@ from .protocol_lifecycle import Blocked, ProtocolServer` |
| `ef67d139e` | `tools/harness/backends/protocol_matrix.py` `@@ -29,6 +30,10 @@ def stone_count(client):` |
| `ef67d139e` | `tools/harness/backends/protocol_matrix.py` `@@ -195,6 +200,8 @@ def run(ctx, *, random_mode=False):` |
| `ef67d139e` | `tools/harness/java_matrix_client.py` `@@ -81,11 +81,21 @@ class JavaMatrixClient:` |
| `ef67d139e` | `tools/harness/matrix_domains/_items.py` `@@ -54,7 +54,7 @@ def acknowledge(ctx):` |
| `ef67d139e` | `tools/harness/protocol_input_verbs.py` `@@ -222,9 +222,10 @@ class ProtocolInputVerbs:` |
| `ef67d139e` | `tools/harness/protocol_input_verbs.py` `@@ -234,6 +235,12 @@ class ProtocolInputVerbs:` |
| `ef67d139e` | `tools/harness/protocol_matrix_client.py` `@@ -268,7 +268,7 @@ class ProtocolMatrixClient(ProtocolStationVerbs, ProtocolInputVerbs):` |
| `ef67d139e` | `tools/harness/regressions.py` `@@ -69,7 +69,7 @@ def _client_command(profile, scenario):` |
| `ef67d139e` | `tools/harness/regressions.py` `@@ -106,6 +106,7 @@ _PROFILE_VERDICTS = {` |
| `ef67d139e` | `tools/harness/regressions.py` `@@ -126,6 +127,9 @@ def _profile_command(profile, verdict):` |
| `ef67d139e` | `tools/harness/regressions.py` `@@ -175,8 +179,10 @@ class VerdictNotMet(RuntimeError):` |
| `ef67d139e` | `tools/harness/wire.py` `@@ -520,7 +520,7 @@ class PlayObserver:` |
| `ef67d139e` | `tools/harness/wire.py` `@@ -565,13 +565,26 @@ class PlayObserver:` |

## Proof

Registered proving refs are run with retained tests on HEAD, with this patch reverse-applied and then restored. Receipts: `receipts/HarnessFix4/TEST-END-RESTART-OBSERVER.red.log` and `.green.log`. Only the symptom assertion named above carries the red; other failures are not evidence. The row stays open until Main runs ledger promote and independent review.
