# TEST-JAVA-CAPABILITIES: forward-ported hunk map

Base: `ff72f53bde64aac80ca791226f0bb65daabb4f10`. The adjacent patch is identical to `../fix-patches/TEST-JAVA-CAPABILITIES.patch`; digest uses `ledger._split_non_test`. Tests/docs stay in the control. No invented pre-image or a/-side behavior adaptation.

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

The original adapter refs remain, but hand-authored observation dictionaries alone are not producer proof. Added `harness:scene-shot:java-matrix-system-chat`, assertion `system_chat_producer_reaches_matrix_events` at `tools/harness/backends/java_system_chat.py:23`. One real plain Java 26.3 client joins and receives a unique system-chat packet; GUI receipt independently fences arrival, then actual JavaMatrixClient.snapshot must expose that message from the loader journal. No multi-message live ordering claim.

Loader producer control: owned loader worktree `wt/HarnessFix4Producer`, base `a55980282483eb1116e974e95d6552b92131bb63`; remove only `MinecraftClientObservation.java:126` emission, rebuild the real agent jar and rerun the same registered gate. `receipts/HarnessFix4Proof/java-producer.red.log` fails the journal assertion after successful join/packet receipt. The control patch and build receipts are retained there. Core adapter patch control and restored producer/adapter evidence are `java-adapter.red.log` and `java-producer.restore-green.log`. The loader control is additional causal proof, not a new core fix hunk. Main authorized server pin `8b6fc28e58bb` for development; promotion must use Main's merged-tree pin.
