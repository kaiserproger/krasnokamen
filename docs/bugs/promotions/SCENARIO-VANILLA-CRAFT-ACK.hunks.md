# SCENARIO-VANILLA-CRAFT-ACK: forward-ported hunk map

Base: `ff72f53bde64aac80ca791226f0bb65daabb4f10`. The adjacent patch is identical to `../fix-patches/SCENARIO-VANILLA-CRAFT-ACK.patch`; digest uses `ledger._split_non_test`. Tests/docs stay in the control. No invented pre-image or a/-side behavior adaptation.

Both original random.py and quirk_parity/player.py hunks are retained (HEAD tools/harness/matrix_domains/random.py:440–451; tools/harness/quirk_parity/player.py:58–60). Both wire.py behavioral hunks remain at tools/harness/wire.py:270–274,574–575. Only click_body's keyword parameter remains (b): retained API callers pass force_cursor_sync; the reversed encoder uses its original empty prediction bytes. Proving refs: `test_seed19_adapter_consumes_received_packet_transitions`, tools/harness/test_matrix_random.py:65, proves the early-result fence; `test_vanilla_craft_cursor_correction_uses_transmitted_prediction`, :178, proves the original stale cursor when the vanilla correction is omitted. The latter runs real run_player_script setup, ProtocolMatrixClient.click_slot, PlayObserver.click/send and click_body. Its socket peer decodes actual emitted carried-stack bytes and corrects only a mismatching prediction. It neither mocks click_slot nor injects packets from a wait callback.

## Every original non-test hunk

The dispositions above apply to each listed hunk, including mixed hunks at sub-hunk granularity.

| Commit | Original path/hunk |
| --- | --- |
| `09c0d0d7d` | `tools/harness/matrix_domains/random.py` `@@ -407,12 +407,18 @@ def _action(ctx, step):` |
| `09c0d0d7d` | `tools/harness/quirk_parity/player.py` `@@ -55,6 +55,9 @@ def run_player_script(ctx, row, jar, vanilla_data):` |
| `09c0d0d7d` | `tools/harness/wire.py` `@@ -262,12 +262,16 @@ def pack_block_position(position: tuple[int, int, int]) -> bytes:` |
| `09c0d0d7d` | `tools/harness/wire.py` `@@ -557,7 +561,8 @@ class PlayObserver:` |

## Proof

Combined patch: `receipts/HarnessFix4Proof/craft.red.log` and `craft.green.log`. Independent player setup and wire controls: `craft-player-flag.red.log`/`.green.log` and `craft-wire.red.log`/`.green.log`. The sync-specific red retains the old log cursor after input receipt, then fails the named craft assertion; restoration passes. No live vanilla differential rerun is claimed. Status stays open; only Main promotes.
