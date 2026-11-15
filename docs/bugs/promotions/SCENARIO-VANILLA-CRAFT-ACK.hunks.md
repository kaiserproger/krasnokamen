# SCENARIO-VANILLA-CRAFT-ACK: forward-ported hunk map

Base: `5315128eeb4cfcd19a6158db6498307ac674cad4`. The adjacent patch is identical to `../fix-patches/SCENARIO-VANILLA-CRAFT-ACK.patch`; digest uses `ledger._split_non_test`. Tests/docs stay in the control. No invented pre-image or a/-side behavior adaptation.

Both original random.py and quirk_parity/player.py hunks are retained (HEAD tools/harness/matrix_domains/random.py:440–451; tools/harness/quirk_parity/player.py:58–60). Both wire.py behavioral hunks are also retained at tools/harness/wire.py:270–274,574–575; they DO still have HEAD equivalents, so the earlier claim of omission due to rewrite is not used. Only click_body's keyword parameter remains (b): retained API callers pass force_cursor_sync. This is no behavior adaptation; the reversed encoder uses its original empty prediction bytes. Proving ref test_seed19_adapter_consumes_received_packet_transitions, tools/harness/test_matrix_random.py:65: a received input without its cursor correction must not satisfy the craft fence. The socket fixture is driven by click and packet transitions, never a description-string match. Missing correction still times out in its independent branch.

## Every original non-test hunk

The dispositions above apply to each listed hunk, including mixed hunks at sub-hunk granularity.

| Commit | Original path/hunk |
| --- | --- |
| `09c0d0d7d` | `tools/harness/matrix_domains/random.py` `@@ -407,12 +407,18 @@ def _action(ctx, step):` |
| `09c0d0d7d` | `tools/harness/quirk_parity/player.py` `@@ -55,6 +55,9 @@ def run_player_script(ctx, row, jar, vanilla_data):` |
| `09c0d0d7d` | `tools/harness/wire.py` `@@ -262,12 +262,16 @@ def pack_block_position(position: tuple[int, int, int]) -> bytes:` |
| `09c0d0d7d` | `tools/harness/wire.py` `@@ -557,7 +561,8 @@ class PlayObserver:` |

## Proof

Registered proving refs are run with retained tests on HEAD, with this patch reverse-applied and then restored. Receipts: `receipts/HarnessFix4/SCENARIO-VANILLA-CRAFT-ACK.red.log` and `.green.log`. Only the symptom assertion named above carries the red; other failures are not evidence. The row stays open until Main runs ledger promote and independent review.
