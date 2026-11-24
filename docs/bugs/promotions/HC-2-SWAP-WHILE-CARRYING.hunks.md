# HC-2-SWAP-WHILE-CARRYING — hunk map

Row: `HC-2-SWAP-WHILE-CARRYING` (client, major). Proving refs:
`harness:entities:native-v2-input-menus` (assertion `swap_while_carrying_keeps_slot`) and
`client:krasnokamen-client:ui::inventory_input_tests::inventory_swapping_while_carrying_does_not_change_slots`.

## Real fix and its pre-image

The behaviour was added by client `5ef71d566a26c78d51e1c1970058c8cf66ee6c91`
("fix(client): restore inventory shortcuts and beacon interaction", 2026-10-29), a bulk commit that
predates the client rename `f0a6abf6` (2026-11-12). Its pre-image is therefore not buildable with
today's `pin-release --client`, and the route is the committed-reversal pair (docs/bugs/README.md:

- reversal `fba8b0d4a64b9efbe3184b27eb817f50b4b9121d` — deletes exactly the guard below, keeps every
  test of the fix (so the row's unit ref is red on this tree: `negctl/swap-unit-control.log`).
- re-land `104e654e34b34ef93ea820eca0c13d1b0998fa95` — restores it and is the row's `fix_commit`
  (`negctl/swap-unit-restore.log` is green).

`git diff fba8b0d4a6 104e654e34` is the guard hunk only; patch-id equals the re-land commit's own
non-test patch (the ledger checks this again with `_check_client_control_delta`).

## The guard (the row's path, HEAD position)

```diff
--- a/src/ui/state.rs
+++ b/src/ui/state.rs
@@ -2946,6 +2954,14 @@ impl UiState {
                 "q" | "Q" => (u8::from(m.control), ClickMode::Throw),
                 _ => return Ok(()),
             };
+            if mode == ClickMode::Swap
+                && inventory
+                    .cursor
+                    .as_ref()
+                    .is_some_and(|stack| !stack.empty())
+            {
+                return Ok(());
+            }
             self.target(&id)?;
             match action {
```

Original hunk in 5ef71d56: `@@ -2637,6 +2662,11 @@ impl UiState` (same lines, pre-rustfmt layout). Vanilla Minecraft's
container-number and F keys keep the hovered slot untouched while a stack rides the cursor; without
the guard the client sends a Swap click and the server exchanges cursor and hovered slot.

## Every other non-test hunk of 5ef71d56 and why it is not in this pair

| file | hunk | what it does | reason |
|---|---|---|---|
| `src/ui/state.rs` | `@@ -17,6 +17,7` | imports `Duration, Instant` for double-click timing | (a) other half of the fix (collect-on-double-click, another row's behaviour); the guard needs no timing |
| `src/ui/state.rs` | `@@ -437,6 +438,8` | `last_slot_click`, `collect_on_release` fields | (a) another behaviour of the same bulk commit (collect-on-release) |
| `src/ui/state.rs` | `@@ -593,6 +596,8` | initializes those fields | (a) another behaviour of the same bulk commit |
| `src/ui/state.rs` | `@@ -1130,6 +1135,8` | clears them in `clear_session` | (a) another behaviour of the same bulk commit |
| `src/ui/state.rs` | `@@ -1981,6 +1988,8` | pointer-down bookkeeping for double click | (a) another behaviour of the same bulk commit |
| `src/ui/state.rs` | `@@ -2007,6 +2016,15` | double-click detection (250 ms, same slot/button) | (a) another behaviour of the same bulk commit |
| `src/ui/state.rs` | `@@ -2139,7 +2157,13` | `PickupAll` on collect release | (a) another behaviour of the same bulk commit |
| `src/ui/state.rs` | `@@ -2172,6 +2196,7` | resets `collect_on_release` on pointer up | (a) another behaviour of the same bulk commit |
| `src/ui/screens.rs` | `@@ -422,7 +422,7` | screen label/window title line changed | (a) another row's UI text in the same bulk commit |
| `src/ui/screens.rs` | `@@ -568,8 +568,6` | removes two hover lines | (a) another row's UI cleanup in the same bulk commit |
| `src/inventory.rs` | `@@ -14,6 +14,8`, `@@ -725,6 +727,15`, `@@ -793,7 +804,9` | cursor/pending-stack handling for the short-click path | (a) the physical-key dispatch half of HC-2 (its own pair `0941d0948f -> 45ef47e3cc`) |
| `src/inventory/policies.rs` | `@@ -1372,3 +1372,7` | inside `mod fraction_tests` — a test hunk | test hunk; excluded by `non_test_patch` |
| `src/protocol/content.rs` | `@@ -2155,6 +2155,10` | menu permission/slot metadata from content | (a) another row's beacon/inventory data, not the swap guard |
| `src/ui.rs` | `@@ -32,6 +32,8` | test-module registration (`#[cfg(test)] mod inventory_input_tests;`) | test wiring; excluded by `non_test_patch` |
| `src/lib.rs` | `@@ -44,6 +44,55`, `@@ -1129,6 +1178,12`, `@@ -1205,7 +1260,6`, `@@ -1214,23 +1268,6` | input plumbing and test-module lines | (a) physical-key dispatch half of HC-2; reversed and mapped in HC-2's own pair |

The row was split from HC-2 precisely because one pair cannot carry both halves: HC-2's pair reverses
only the physical-key dispatch hunk and leaves this guard in place
(`HC-2-SWAP-WHILE-CARRYING.toml`, `source`). Nothing else of 5ef71d56 is claimed here.

## Gate hygiene

- The scene step picks the seeded diamond stack onto the cursor with a physical left click, then
  presses `1` and `F` over a slot holding emeralds and requires both the hovered slot and the cursor
  unchanged; on the guard-less control tree the press swaps them and the assertion fails.
- The scene's earlier steps (`physical_short_click_breaks_block`, number/F/Q inventory steps) are
  unchanged, so a red at `swap_while_carrying_keeps_slot` is not shared with QA5-FAST-CLICK.
- Both receipts run in the lane's own XDG roots and settings file (`native_player_input.menus` is
  launched by `entities` with its per-run settings file).
