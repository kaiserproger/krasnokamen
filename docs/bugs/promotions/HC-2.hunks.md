# HC-2 — hunk map for the tip reversal pair

Promotion receipt: `promotions/HC-2.json`.
Client pair: reversal `client:0941d0948f` → re-land `client:45ef47e3cc` (the row's `fix_commit`).

## Original commit

`client:5ef71d56` "fix(client): restore inventory shortcuts and beacon interaction" (2026-10-29)
predates the client rename (`f0a6abf6`, 2026-11-12), so `pin-release --client` cannot build its
parent and the control is a reversal on the current tip. Its non-test hunks do not reverse-apply
(`git apply -R --check --recount` fails in `src/lib.rs` and `src/ui/state.rs`,
`receipts/XClientControl/negctl/hc23-reverse-apply-failures.log`), so the pair hand-ports them.

## Hunks of `client:5ef71d56`

| file | hunk (`@@` header) | content | disposition |
|---|---|---|---|
| `src/lib.rs` | `@@ -1129,6 +1178,12 @@ impl ApplicationHandler for App` | the `native_item_key(...)` early dispatch inside the key handler: physical 1-9/F/Q reach `UiIntent::Key` while the inventory is open | REVERSED — the pair's `0941d0948f` removes the call site on the tip (`App::dispatch_binding`), so the key falls through to the binding action path again; this is the row's symptom |
| `src/lib.rs` | `@@ -44,6 +44,55 @@ use winit::{` | the new `native_item_key` function | NOT reversed — reason (b): `controls_tests.rs:46-83` and `ui/inventory_input_tests.rs:453-553` call it, so deleting it stops the control from compiling; with the call site gone it has no production effect |
| `src/lib.rs` | `@@ -1205,7 +1260,6 @@ impl ApplicationHandler for App` | `KeyCode::KeyQ => "drop"` removed from the gameplay key map | NOT reversed — reason (c): the tip no longer has that match (bindings table + `dispatch_binding`); the HEAD equivalent of the removal is the dispatch hunk itself, which is reversed |
| `src/lib.rs` | `@@ -1214,23 +1268,6 @@ impl ApplicationHandler for App` | the old key-handling block this commit replaced | NOT reversed — reason (c), same rewrite |
| `src/ui/state.rs` | `@@ -2637,6 +2662,11 @@ impl UiState` | the swap-while-carrying guard (`if mode == ClickMode::Swap && cursor is not empty { return Ok(()) }`) | NOT reversed — reason (a): that is the behaviour of the split row **HC-2-SWAP-WHILE-CARRYING**, which keeps `inventory_swapping_while_carrying_does_not_change_slots`; its control is that row's own |
| `src/ui/state.rs` | `@@ -17,6 +17,7 @@ use serde_json::{json, Value};`, `@@ -437,6 +438,8 @@ pub struct UiState`, `@@ -593,6 +596,8 @@ impl UiState`, `@@ -1130,6 +1135,8 @@ impl UiState` | `Instant`/`Duration` imports, `last_slot_click`/`collect_on_release` fields and their resets | NOT reversed — (b): the double-click collect path below reads them, and the kept inventory tests drive that path |
| `src/ui/state.rs` | `@@ -1981,6 +1988,8 @@ impl UiState`, `@@ -2007,6 +2016,15 @@ impl UiState`, `@@ -2139,7 +2157,13 @@ impl UiState`, `@@ -2172,6 +2196,7 @@ impl UiState` | the double-click collect-on-release path | NOT reversed — reason (a): a separate symptom (double-click collects into the cursor), covered by `ui::inventory_input_tests` and not by this row |
| `src/inventory.rs` | `@@ -14,6 +14,8 @@ mod persistent;`, `@@ -725,6 +727,15 @@ impl InventoryStore`, `@@ -793,7 +804,9 @@ impl InventoryStore` | beacon interaction / persistent state | NOT reversed — reason (a): the beacon half of the commit, a different surface |
| `src/inventory/policies.rs` | `@@ -1372,3 +1372,7 @@ mod fraction_tests` | test-module arm | OMITTED — test hunk |
| `src/inventory/beacon_tests.rs`, `src/inventory/bundle_gesture_tests.rs`, `src/ui/inventory_input_tests.rs`, `src/ui_hotfix_tests.rs` | `@@ -0,0 +1,186 @@`, `@@ -0,0 +1,187 @@`, `@@ -0,0 +1,313 @@`, `@@ -490,12 +490,12 @@` | tests | OMITTED — test hunks |
| `src/protocol/content.rs` | `@@ -2155,6 +2155,10 @@ impl Content` | beacon/content plumbing | NOT reversed — reason (a), the beacon half |
| `src/ui.rs` | `@@ -32,6 +32,8 @@ mod sample;` | module wiring for the new inventory tests | NOT reversed — (b), test-module wiring |
| `src/ui/screens.rs` | `@@ -422,7 +422,7 @@ impl UiState`, `@@ -568,8 +568,6 @@ impl UiState` | beacon screen behaviour | NOT reversed — reason (a), the beacon half |

## a/-side adaptation

None. The removed call site is exactly the hunk's `+` block, restored verbatim by the re-land
(`git diff 0941d0948f 45ef47e3cc` is that one block).

## Proving ref

`harness:entities:native-v2-input-menus` → `survival_number_1_moves_hovered`
(`tools/harness/backends/native_player_input.py`): with the inventory open, a physical `1` press
must move the hovered stack. The control's receipt, with the observed server-side cursor and slot
state, is recorded in `promotions/HC-2.json`.

## Gate hygiene (2026-11-24, why the restore was blocked and what changed)

The pair was already committed on 2026-11-24 but could not be promoted: the registered scene failed
`survival_number_2_moves_hovered` on every run of every client pin, while the same pins passed the
whole scene the day before. This was not the fix and not the client. The native client reads
`client-settings.json` back on every start from its data root, and those runs inherited a file whose
`hotbar_2` had been rebound to the middle mouse button
(`receipts/D123/D123Client/negctl/contaminated-client-settings.json`, and the same rebind in
`~/.local/share/krasnokamen-client/client-settings.json` since 2026-11-15). `Bindings::action` is a
reverse map from one input to one action, so a physical `Digit2` press resolved to no action:
an instrumented build logs `window code=Digit2 pressed=true …` and then no `action=` line, while
`Digit1` logs `action=hotbar_1` and reaches `ui key=1 node=slot/9`. `tools/harness/backends/native_client.py`
now gives every native run its own settings file (`launch_environment`), so a saved rebind cannot
change a gate result again; the harness unit ref
`python:tools.harness.test_harness:NativeLaunchEnvironmentTests.test_a_launch_never_inherits_the_data_roots_managed_settings`
is on the row. With that isolation the scene passes the survival and creative number steps on both
the control's client and the re-land (`runs in promotions/HC-2.json`).

The scene's creative half also carries an unrelated open defect,
`CLIENT-CREATIVE-PLACEMENT-DROPPED`: the placement click is release-driven and is silently dropped
on some runs (trace: the pickup's `prediction_click_slot slot=10` has no `slot=11` successor and the
cursor still holds the stack). It does not touch this row's proving assertion: the scene stops at
the first failure and the survival number steps run before it, so the control's red stays
`survival_number_1_moves_hovered`.
