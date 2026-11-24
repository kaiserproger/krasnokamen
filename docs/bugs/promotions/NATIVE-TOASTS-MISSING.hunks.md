# NATIVE-TOASTS-MISSING — hunk map for the tip reversal pair

Promotion receipt: `promotions/NATIVE-TOASTS-MISSING.json`.
Client pair: reversal `client:6c3ebb27ac` → re-land `client:e32052ade6` (the row's `fix_commit`).

## Original commit

`client:8deb8fd7` "feat(ui): vanilla advancement, recipe and unsecure-server toasts" (2026-11-11)
predates the client rename (`f0a6abf6`, 2026-11-12), so `pin-release --client` cannot build its
parent and the control must be a reversal on the current tip.

Files and non-test hunk counts: `src/inventory.rs` 1, `src/inventory/receive.rs` 1,
`src/session.rs` 2, `src/session/packets.rs` 2, `src/ui.rs` 1, `src/ui/advancements.rs` 1,
`src/ui/anim.rs` 1, `src/ui/packets.rs` 1, `src/ui/screens.rs` 1, `src/ui/state.rs` 8,
`src/ui/toasts.rs` 1 (new file), `src/ui/toasts_tests.rs` (test file, never reversed).

## Hunks of `client:8deb8fd7`, with their `@@` headers

| file | hunk (`@@` header) | disposition |
|---|---|---|
| `src/ui/state.rs` | `@@ -3502,7 +3509,7 @@ impl UiState` | REVERSED — the `render` tail: `-        Ok(())` → `+        self.paint_toasts(content, out, transform)`; the only hunk whose removal takes the toast pixels out of the retained overlay |
| `src/ui/state.rs` | `@@ -400,6 +400,8 @@ pub(crate) enum Paint` (`Paint::Toast`) | NOT reversed — (b)/(c): the node kind is registered by the painter path and read by later code (`state.rs:3767` draws nothing for it), and it is inert without the paint call |
| `src/ui/state.rs` | `@@ -521,6 +523,9 @@ pub struct UiState` (`toasts`, `insecure_chat_warned`) | NOT reversed — (b): the kept manager tests construct and read these fields |
| `src/ui/state.rs` | `@@ -650,6 +655,8 @@ impl UiState` (field defaults) | NOT reversed — (b), same as the fields |
| `src/ui/state.rs` | `@@ -3579,7 +3586,7 @@ impl UiState` | NOT reversed — (b): the node-registration arm is inert without the paint call |
| `src/ui/state.rs` | `@@ -3924,6 +3931,7 @@ impl UiState` | NOT reversed — (b): the sound drain, exercised by the kept manager tests |
| `src/ui/state.rs` | `@@ -4159,6 +4167,18 @@ fn append_stack_item(` | NOT reversed — (b): HUD ordering around the toast slot |
| `src/ui/state.rs` | `@@ -4166,7 +4186,7 @@ fn append_stack_item(` | NOT reversed — (b), same as the previous hunk |
| `src/ui/toasts.rs` | `@@ -0,0 +1,768 @@` (new module: `ToastManager`, `Toast`, `paint_toasts`, the nine-slice, the advancement/recipe/system kinds) | NOT reversed — (b): the module is API the kept `src/ui/toasts_tests.rs` calls (`ToastManager`, `Toast::system`, `progress_map_order`); deleting it stops the control from compiling. Its only effect on a frame is through the reversed call above |
| `src/ui/advancements.rs` | `@@ -527,4 +527,73 @@ mod tests` | NOT reversed — (a): the advancement stacking behaviour of NATIVE-ADVANCEMENT-TOAST-DRAW, and an inline test module |
| `src/ui/packets.rs` | `@@ -786,9 +786,19 @@ impl UiState` | NOT reversed — (a): the advancement/recipe unlock packets, NATIVE-RECIPE-TOAST-DRAW |
| `src/ui/screens.rs` | `@@ -171,7 +171,7 @@ impl UiState` | NOT reversed — (a): the screen-side toast raise, the advancement/recipe rows' path |
| `src/ui/anim.rs` | `@@ -163,7 +163,7 @@ impl UiState` | NOT reversed — (a): the animation flag the toast kinds drive; the capture behaviour it interacts with is CAPTURE-DEFERRED-HUD-TRANSIENT |
| `src/inventory.rs` | `@@ -439,6 +439,9 @@ pub struct InventoryStore`, `@@ -474,6 +477,7 @@ impl Default for InventoryStore` | NOT reversed — (a): recipe-book notification state, NATIVE-RECIPE-TOAST-DRAW |
| `src/inventory/receive.rs` | `@@ -71,8 +71,13 @@ impl InventoryStore`, `@@ -84,6 +89,7 @@ impl InventoryStore` | NOT reversed — (a), same as the previous file |
| `src/session.rs` | `@@ -1132,7 +1132,7 @@ impl Session`, `@@ -1144,6 +1144,20 @@ impl Session` | NOT reversed — (a)/(b): the Play-login raise the kept manager tests drive, and the advancement/recipe packet path |
| `src/session/packets.rs` | `@@ -287,6 +287,16 @@ impl Session`, `@@ -365,14 +375,24 @@ impl Session` | NOT reversed — (a): the advancement/recipe packet plumbing |
| `src/ui.rs` | `@@ -40,6 +40,7 @@ mod statistics;` (`mod toasts`) | NOT reversed — (b): removing the module wiring stops the kept tests from compiling |
| `src/ui/toasts_tests.rs` | `@@ -0,0 +1,218 @@` | test file, never reversed |

## a/-side adaptation

None: the reversal restores the pre-image line `Ok(())` that the original hunk deleted, and the
re-land restores the added line. `git diff 6c3ebb27ac e32052ade6` is exactly the original hunk's
`+` side.

## Proving ref

`harness:scene-shot:native-toast` → `toast_panel_appeared`
(`tools/harness/backends/scene_shot_checks.py`): the panel's own rectangle (x 621..1280, y 0..168 at
GUI scale 3) must carry dark navy pixels two seconds after Play and none eighteen seconds later; the
companion `toast_title_drawn` reads the panel's yellow title ink.

Promotion receipts (candidate core `1cc8a62888`, server pin `dbeaf821`):

- control `20261123T232512-native-toast-kzny_pv7` — FAIL `toast_panel_appeared`, early/late panel 0/0,
  `gpu_complete` and `camera_unchanged` true (an assertion failure, not a capture deferral).
- restore `20261123T232603-native-toast-hfc0xnu1` — PASSED, early panel 89445, title 3195, late 0.
- promote `20261123T233140-ledger-promote-qxmq3srr` at tree `41b0fe867f`; pair patch id `7a2815a2…`.

The earlier receipts `20261123T225248` / `20261123T230254` on core `107eb70061` are superseded: the
candidate fingerprint is HEAD plus the dirty hash, so the row commit that followed them moved the
tree and the promotion had to be re-run on `1cc8a62888`.
