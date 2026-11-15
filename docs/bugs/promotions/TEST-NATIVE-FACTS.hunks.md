# TEST-NATIVE-FACTS: forward-ported hunk map

Base: `5315128eeb4cfcd19a6158db6498307ac674cad4`. The adjacent patch is identical to `../fix-patches/TEST-NATIVE-FACTS.patch`; digest uses `ledger._split_non_test`. Tests/docs stay in the control. No invented pre-image or a/-side behavior adaptation.

native_matrix.py +608/+620/+640 equivalent: oracle_started initialization, body-entry flag and failure_status dispatch at tools/harness/backends/native_matrix.py:808,820,854–855 are retained. In +44 the failure_status API stays (b), because kept direct tests call it; check_health and the check_health call in +640 stay (a), TEST-NATIVE-HEALTH's behavior. Other paths in the table are omitted (a): protocol lifecycle shutdown, UUID/census and item-event behavior, not startup-vs-oracle classification. No synthetic always-fail replacement. Proving ref: test_run_classifies_startup_and_oracle_exceptions at tools/harness/test_native_matrix.py:117; startup must be blocked, and summary executed/fail counts remain zero. The retained post-start branch now reaches its actual planted oracle assertion instead of failing on an incomplete fake host. Later missing-content-specific MissingCapability coverage remains green and is not control evidence.

## Every original non-test hunk

The dispositions above apply to each listed hunk, including mixed hunks at sub-hunk granularity.

| Commit | Original path/hunk |
| --- | --- |
| `11ed1c38a` | `tools/harness/backends/native_matrix.py` `@@ -44,6 +44,30 @@ def transport_blocker(scenario: dict[str, Any]) -> str | None:` |
| `11ed1c38a` | `tools/harness/backends/native_matrix.py` `@@ -584,6 +608,7 @@ def run(ctx: Any) -> dict[str, Any]:` |
| `11ed1c38a` | `tools/harness/backends/native_matrix.py` `@@ -595,6 +620,7 @@ def run(ctx: Any) -> dict[str, Any]:` |
| `11ed1c38a` | `tools/harness/backends/native_matrix.py` `@@ -614,12 +640,14 @@ def run(ctx: Any) -> dict[str, Any]:` |
| `11ed1c38a` | `tools/harness/backends/protocol_lifecycle.py` `@@ -294,9 +294,9 @@ seed = {self.seed}` |
| `11ed1c38a` | `tools/harness/backends/protocol_lifecycle.py` `@@ -312,13 +312,13 @@ seed = {self.seed}` |
| `11ed1c38a` | `tools/harness/protocol_entities.py` `@@ -385,6 +385,8 @@ class ProtocolEntities:` |
| `11ed1c38a` | `tools/harness/protocol_entities.py` `@@ -437,6 +439,12 @@ class ProtocolEntities:` |
| `11ed1c38a` | `tools/harness/protocol_entities.py` `@@ -457,6 +465,8 @@ class ProtocolEntities:` |
| `11ed1c38a` | `tools/harness/protocol_entities.py` `@@ -480,6 +490,8 @@ class ProtocolEntities:` |
| `11ed1c38a` | `tools/harness/protocol_entities.py` `@@ -559,7 +571,9 @@ class ProtocolEntities:` |
| `11ed1c38a` | `tools/harness/protocol_matrix_client.py` `@@ -135,6 +135,10 @@ class ProtocolMatrixClient(ProtocolStationVerbs, ProtocolInputVerbs):` |

## Proof

Registered proving refs are run with retained tests on HEAD, with this patch reverse-applied and then restored. Receipts: `receipts/HarnessFix4/TEST-NATIVE-FACTS.red.log` and `.green.log`. Only the symptom assertion named above carries the red; other failures are not evidence. The row stays open until Main runs ledger promote and independent review.
