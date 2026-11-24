# HC-3 — hunk map for the committed reversal pair

Promotion receipt: `promotions/HC-3.json`.
Client pair: reversal `client:0941d0948f` → re-land `client:45ef47e3cc` (the row's `fix_commit`).

The pair is the same one HC-2 uses: `5ef71d56` ("fix(client): restore inventory shortcuts and beacon
interaction") is the small commit whose `native_item_key` dispatch restores physical gameplay
`F`/middle-click (this row) and the inventory 1-9/F/Q hovered-slot actions (HC-2). One reversal commit
removes the dispatch call site; one re-land restores it verbatim.

## Fix in several commits

The row's path was closed by more than one commit, in this order:

| commit | part of the path it closes | disposition |
|---|---|---|
| `client:d5303246` (2026-10-29) | makes `pick` a rebindable native action (`src/ui/options.rs`, `src/session/actions.rs`) instead of a hard-coded key | NOT reversed — reason (b)/(a): the kept `controls_tests` call the binding surface, and reversing it removes the whole configurable-binding feature, which is other rows' behaviour |
| `client:83994480` (2026-11-09) | the world-input path that turns a physical middle-click into that binding ("integrate ClientNext slices on RC5", 9,174 insertions) | NOT reversed — reason (a): the input slice is not separable from the other slices the same commit integrates, and its reversal would turn the control red on other rows' assertions |
| `client:5ef71d56` (2026-10-29) | the `native_item_key` call site that makes the physical press reach `InputIntent::Key` at all | REVERSED — the pair's `0941d0948f`; without it the earlier two have no production effect, because the press falls through to the binding-action path |

## Original commit

`client:5ef71d56` predates the client rename (`f0a6abf6`), so `pin-release --client` cannot build its
parent and the control is a reversal on the current tip. Its non-test hunks do not reverse-apply
(`git apply -R --check --recount` fails in `src/lib.rs` and `src/ui/state.rs`,
`receipts/XClientControl/negctl/hc23-reverse-apply-failures.log`), so the pair hand-ports them.

## Hunks of `client:5ef71d56`

| file | hunk (`@@` header) | content | disposition |
|---|---|---|---|
| `src/lib.rs` | `@@ -1129,6 +1178,12 @@ impl ApplicationHandler for App` | the `native_item_key(...)` early dispatch inside the key handler: a physical gameplay `F` (swap offhand) / middle-click (`pick`) press reaches `InputIntent::Key` instead of falling through to the binding-action path that has no term for them | REVERSED — `0941d0948f` removes the call site on the tip (`App::dispatch_binding`), so a middle-click `pick` press resolves to an action (`pick`) that the fallthrough match has no arm for and is dropped; this is the row's symptom |
| `src/lib.rs` | `@@ -44,6 +44,55 @@ use winit::{` | the new `native_item_key` function, including its `pick` branch (`!inventory_open && matches!(action, "drop" \| "swap_offhand" \| "pick")`) | NOT reversed — reason (b): `controls_tests.rs:46-83` and `ui/inventory_input_tests.rs:453-553` call it, so deleting it stops the control from compiling; with the call site gone it has no production effect |
| `src/lib.rs` | `@@ -1205,7 +1260,6 @@ impl ApplicationHandler for App` | `KeyCode::KeyQ => "drop"` removed from the gameplay key map | NOT reversed — reason (c): the tip no longer has that match (bindings table + `dispatch_binding`); the HEAD equivalent of the removal is the dispatch hunk itself, which is reversed |
| `src/lib.rs` | `@@ -1214,23 +1268,6 @@ impl ApplicationHandler for App` | the old key-handling block this commit replaced | NOT reversed — reason (c), same rewrite |
| `src/ui/state.rs` | `@@ -2637,6 +2662,11 @@ impl UiState` | the swap-while-carrying guard | NOT reversed — reason (a): HC-2-SWAP-WHILE-CARRYING's behaviour |
| `src/ui/state.rs` | `@@ -17,6 +17,7 @@`, `@@ -437,6 +438,8 @@`, `@@ -593,6 +596,8 @@`, `@@ -1130,6 +1135,8 @@` | `Instant`/`Duration` imports and the `last_slot_click`/`collect_on_release` fields | NOT reversed — reason (b): the kept double-click path reads them |
| `src/ui/state.rs` | `@@ -1981,6 +1988,8 @@`, `@@ -2007,6 +2016,15 @@`, `@@ -2139,7 +2157,13 @@`, `@@ -2172,6 +2196,7 @@` | the double-click collect-on-release path | NOT reversed — reason (a): a separate symptom covered by `ui::inventory_input_tests` |
| `src/inventory.rs`, `src/inventory/policies.rs`, `src/protocol/content.rs`, `src/ui.rs`, `src/ui/screens.rs` | see `HC-2.hunks.md` | beacon interaction, content plumbing, module wiring | NOT reversed — reason (a)/(b): the beacon half and test wiring; the full list with every `@@` header is in `HC-2.hunks.md` |
| test files (`src/inventory/beacon_tests.rs`, `bundle_gesture_tests.rs`, `ui/inventory_input_tests.rs`, `ui_hotfix_tests.rs`) | `@@ -0,0 +1,…` | tests | OMITTED — test hunks |

## a/-side adaptation

None. The removed call site is exactly the hunk's `+` block, restored verbatim by the re-land
(`git diff 0941d0948f 45ef47e3cc` is that one block, patch-id `317bd348e8…`).

## Proving ref

`harness:entities:native-v2-input-pick` → `survival_picked_block_hotbar`
(`tools/harness/backends/native_player_input.py::pick`, the survival leg): the player stands aimed at
a placed `minecraft:diamond_block`, a physical middle-click arrives as one sub-tick press, and the
server's stack has to land in the hotbar as exactly one stack of 7. Without the dispatch call site
the press resolves to the `pick` action, which the gameplay fallthrough has no arm for, so nothing
happens and the hotbar stays empty. The scene runs the survival leg before the creative one and
stops at the first failing check, so the control records `survival_picked_block_hotbar` false and
never reaches `creative_picked_block_hotbar`; both legs are the same physical middle-click gesture
and the restore proves both. The control's receipt, with the observed hotbar slot and the
server-side state, is recorded in `promotions/HC-3.json`.
