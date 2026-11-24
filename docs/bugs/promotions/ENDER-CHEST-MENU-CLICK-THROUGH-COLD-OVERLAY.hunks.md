# ENDER-CHEST-MENU-CLICK-THROUGH-COLD-OVERLAY: patch-mode fix identity

Original fix: `c7bd44dd06` (`fix(residency): keep cold menu, mining, brush, copper and egg writes off the
overlay`) — the `menu_source_cold` helper plus the `container_click` guard that closes a cold block menu
before any handler is selected (cursor custody kept). `b460f313cc` later extended the same check to
`container_slot_state`/`container_button` and this lane rewrote the helper's doc comment. The whole commit
no longer reverses on HEAD, so the row moves to a scoped patch: `fix_commit =
"patch:0db22dafacbc9521…"` names the HEAD equivalent of the three menu guards plus the slot-of guard, in
`docs/bugs/promotions/ENDER-CHEST-MENU-CLICK-THROUGH-COLD-OVERLAY.patch` (a diff from the pre-fix form to
the candidate; `git apply -R` restores the pre-fix code). Re-audit:
`python3 -m tools.harness ledger promote ENDER-CHEST-MENU-CLICK-THROUGH-COLD-OVERLAY --fix-patch docs/bugs/promotions/ENDER-CHEST-MENU-CLICK-THROUGH-COLD-OVERLAY.patch --dry-run`.
Rule (docs/bugs/README.md, step 1): every original non-test hunk is carried unless it is another row's or
owner's behaviour, API the kept tests call, or code a later commit rewrote.

## Carried

- `tick/input/inventory.rs @@ -225,6 +225,10 @@ pub(in crate::tick) fn container_click` — the
  `if crate::tick::containers::menu_source_cold(state, entity) { close_player_menu(...); return; }` block;
  HEAD `crates/mc-core/src/tick/input/inventory.rs:228-231`. Reversing it lets a click reach the handler
  with a cold menu coordinate (the row's symptom).
- `tick/input/inventory.rs @@ -483,6 +483,10 @@ fn container_slot_state` and `@@ -502,6 +502,10 @@
  fn container_button` — the same guard in the other two menu inputs (`b460f313cc`, HEAD
  `crates/mc-core/src/tick/input/inventory.rs:489-492` and `:508-511`). **The owner of the three guard
  hunks is this row** (ANVIL's patch is stations-only and does not carry them; a whole-commit reverse is
  not possible). Reversing them lets a cold slot-state/button reach the crafter/station handlers.
- `tick/input/inventory.rs` slot-of early return in `container_slot_state` — **`b748b9c053`** (this lane),
  HEAD `crates/mc-core/src/tick/input/inventory.rs:486-488`. It keeps the input off the crafter path when
  `state.connections` still maps a connection whose entity row is gone. **Not the `expect` panic**: with
  the guard alone reversed nothing reaches `.expect("native menu owner")` — the fixture carries no cursor
  and the drop set is empty — but reversing the whole patch removes the cold guard too, so
  `close_player_menu` runs (`crates/mc-core/src/tick/players.rs:155-173`) and the observed red
  (20261123T231647-ledger-promote-d6x5u_d1) lands on the `MenuClosed`/custody assertions.
  Mechanism: `expire_native_openers` (`crates/mc-core/src/tick/dimension.rs:301`) runs after
  `apply_inputs` and only touches `until=Some(..)` entries (`crates/mc-core/src/tick/custody.rs:286-297`),
  while `menu_at` filters `until.is_none()` (`crates/mc-core/src/tick/containers.rs:246-250`), so a ghost
  menu persists in `open_containers`; the close path is what the guard keeps from running.

## Original non-test hunks (complete)

```
## b748b9c053 non-test hunks (kept test files: crates/mc-core/src/ender_chest_tests.rs)
- crates/mc-core/src/tick/input/inventory.rs: slot-of early return in container_slot_state
## b460f313cc non-test hunks (kept test files: crates/mc-core/src/tick/input/inventory.rs only via the guards below, tick/stations_tests.rs)
- crates/mc-core/src/tick/input/inventory.rs: -483,6 +483,10 (container_slot_state cold guard); -502,6 +506,10 (container_button cold guard)
## c7bd44dd06 non-test hunks (kept test files: brushing_tests.rs, copper_tests.rs, ender_chest_tests.rs, interact_parity_tests.rs, tick/mining_tests.rs, tick/stations_tests.rs)
- crates/mc-core/src/brushing.rs: -284,6 +284,14; -296,3 +304,7
- crates/mc-core/src/copper.rs: -220,6 +220,14; -298,6 +306,9
- crates/mc-core/src/tick.rs: -75,6 +75,9
- crates/mc-core/src/tick/containers.rs: -4,6 +4,20
- crates/mc-core/src/tick/input/inventory.rs: -225,6 +225,10
- crates/mc-core/src/tick/mining.rs: -38,8 +38,46; -95,6 +133,20
- crates/mc-core/src/tick/stations.rs: -139,6 +139,11
- crates/mc-core/src/vanilla_interact.rs: -125,8 +125,11
```

## Not carried

- `tick/containers.rs @@ -4,6 +4,20 @@` (`menu_source_cold`) — (b) the three carried guards
  (`crates/mc-core/src/tick/input/inventory.rs:228`, `:489`, `:508`) are its only callers, so reversing the
  helper stops the patch's own hunks from compiling; `damage_anvil` does not call it. No other row carries
  the helper (`ANVIL-DAMAGE-WRITES-AT-COLD-MENU-COORDINATE` is stations-only), which is why it stays here,
  un-owned by any control but needed to compile. The helper's doc comment was rewritten by this lane (c)
  to name exactly the gated inputs.
- (nothing left over: the `container_slot_state`/`container_button` guards and the slot-of guard are
  **carried** by this row — see Carried.)
- `brushing.rs`, `copper.rs`, `tick/mining.rs`, `tick/stations.rs`, `tick.rs`, `vanilla_interact.rs` —
  (a) BRUSH, COPPER, MINING, ANVIL and DRAGON-EGG rows; (c) where those rows later rewrote the code.
- Test files — (b) kept.

## Proving ref (file:line of the failing assertion)

- `cargo:mc-core:ender_chest::tests::ender_chest_click_with_a_cold_menu_coordinate_closes_and_keeps_cursor_custody`
  — `crates/mc-core/src/ender_chest_tests.rs:418` `"a cold menu source closes the menu"` fails once the
  click passes the cold coordinate (the menu stays open), and `:442` (`"the cursor item returns to the
  inventory"`) fails after it.
- `cargo:mc-core:ender_chest::tests::ender_chest_slot_state_with_a_cold_menu_closes_and_keeps_cursor_custody`
  and `…::ender_chest_button_with_a_cold_menu_closes_and_keeps_cursor_custody`
  (`crates/mc-core/src/ender_chest_tests.rs:256`, `:305`): with the guards reversed the cold
  slot-state/button input leaves the menu open and no `MenuClosed` is pushed.
- `cargo:mc-core:ender_chest::tests::ender_chest_slot_state_for_a_despawned_player_leaves_the_cold_menu_alone`
  (`crates/mc-core/src/ender_chest_tests.rs:324`): **not evidence on its own.** The promotion control
  (`20261123T231647-ledger-promote-d6x5u_d1`) observed it green with the slot-of guard reversed, because
  that reverse leaves the cold guard in place: `menu_source_cold` is false, `close_player_menu` never runs,
  and the menu survives as the test expects. With the whole patch reversed it reds through
  `close_player_menu` (`crates/mc-core/src/tick/players.rs:155-173`, `MenuClosed` + custody assertions).
  One-hunk control (slot-of only, cold guard intact), run 2026-11-24 on `b8122ab6e8`: the test **reds at
  `crates/mc-core/src/ender_chest_tests.rs:347`** (`a ghost owner's slot-state input is ignored` — the cold
  guard then closes the menu through `close_player_menu`), and the same run restored green. The guard is
  therefore controlled; the whole-patch control above is green for that test only because it removes the
  cold guard as well.
