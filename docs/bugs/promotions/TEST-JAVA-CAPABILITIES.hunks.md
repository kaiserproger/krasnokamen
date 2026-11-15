# TEST-JAVA-CAPABILITIES: forward-ported hunk map

Base: `5315128eeb4cfcd19a6158db6498307ac674cad4`. The adjacent patch is identical to `../fix-patches/TEST-JAVA-CAPABILITIES.patch`; digest uses `ledger._split_non_test`. Tests/docs stay in the control. No invented pre-image or a/-side behavior adaptation.

The cumulative system-chat projection portion of java_matrix_client.py +81 is retained at tools/harness/java_matrix_client.py:84–94,102. XP fields in the same hunk remain (a), unrelated observation behavior. Later original +123 component-reading and +241 respawn/swap-hands hunks remain (a), not cumulative chat. Both _items.py hunks remain (a): scalar console-feedback parsing and floor loading/teleport sequencing, not the client chat journal. The row is explicitly narrowed to chat; TEST-JAVA-PREDICTION-ACK-LANES preserves all previously uncovered work as open. Proving ref test_events_use_applied_system_chat_not_gui_history at tools/harness/test_java_matrix_client.py:86 asserts the ordered cumulative journal rather than GUI history.

## Every original non-test hunk

The dispositions above apply to each listed hunk, including mixed hunks at sub-hunk granularity.

| Commit | Original path/hunk |
| --- | --- |
| `e99902e71` | `tools/harness/java_matrix_client.py` `@@ -81,11 +81,25 @@ class JavaMatrixClient:` |
| `e99902e71` | `tools/harness/java_matrix_client.py` `@@ -109,6 +123,35 @@ class JavaMatrixClient:` |
| `e99902e71` | `tools/harness/java_matrix_client.py` `@@ -198,6 +241,12 @@ class JavaMatrixClient:` |
| `e99902e71` | `tools/harness/matrix_domains/_items.py` `@@ -107,9 +107,11 @@ def number(ctx, target, path):` |
| `e99902e71` | `tools/harness/matrix_domains/_items.py` `@@ -173,14 +175,22 @@ def oracle(ctx):` |

## Proof

Registered proving refs are run with retained tests on HEAD, with this patch reverse-applied and then restored. Receipts: `receipts/HarnessFix4/TEST-JAVA-CAPABILITIES.red.log` and `.green.log`. Only the symptom assertion named above carries the red; other failures are not evidence. The row stays open until Main runs ledger promote and independent review.
