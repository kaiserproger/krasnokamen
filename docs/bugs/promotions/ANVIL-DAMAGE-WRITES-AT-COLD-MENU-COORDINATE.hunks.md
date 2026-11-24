# ANVIL-DAMAGE-WRITES-AT-COLD-MENU-COORDINATE: patch-mode fix identity

Original fix: `c7bd44dd06` (`fix(residency): keep cold menu, mining, brush, copper and egg writes off the
overlay`) added the menu-source residency check and the `damage_anvil` guard; `b460f313cc` later corrected
the `stations.rs` comment ("discards the damage draw", not "waits"). The `damage_anvil` guard is this
row's path; the same two commits' `container_click`/`container_slot_state`/`container_button` guards are
carried by ENDER-CHEST-MENU-CLICK-THROUGH-COLD-OVERLAY (that row's patch also keeps `menu_source_cold`
compiling; `damage_anvil` does not call the helper), and the copper write guard belongs to
COPPER-HONEYCOMB-WAXES-ACROSS-COLD-CHUNK-EDGE.

`fix_commit = "patch:948c1d9485e5c681…"` names the HEAD equivalents of this row's non-test hunk in
`docs/bugs/promotions/ANVIL-DAMAGE-WRITES-AT-COLD-MENU-COORDINATE.patch` (a diff from the pre-fix form
`c7bd44dd06^:crates/mc-core/src/tick/stations.rs` to the candidate; `git apply -R` restores the pre-fix
code). Re-audit:
`python3 -m tools.harness ledger promote ANVIL-DAMAGE-WRITES-AT-COLD-MENU-COORDINATE --fix-patch docs/bugs/promotions/ANVIL-DAMAGE-WRITES-AT-COLD-MENU-COORDINATE.patch --dry-run`.
Rule (docs/bugs/README.md, step 1): every original non-test hunk is carried unless it is another row's or
owner's behaviour, API the kept tests call, or code a later commit rewrote.

## Carried

- `stations.rs @@ -139,6 +139,12 @@ pub(super) fn damage_anvil` — the comment plus
  `if !state.world.has_chunk(x.div_euclid(16), z.div_euclid(16)) { return; }` before the first
  `block_at` read. HEAD `crates/mc-core/src/tick/stations.rs:142-147`; the same hunk carries
  `b460f313cc`'s comment wording, which is the only later rewrite of this path (c for the comment text
  alone; the behaviour lines are unchanged).

## Not carried

- `c7bd44dd06` `tick/containers.rs @@ -4,6 +4,20 @@` `menu_source_cold` — (a) the click-input rows'
  behaviour (ENDER-CHEST-MENU-CLICK-THROUGH-COLD-OVERLAY, COPPER-HONEYCOMB-WAXES-ACROSS-COLD-CHUNK-EDGE);
  reversing it together with the three `inventory.rs` guards turns their tests red first.
- `c7bd44dd06` `tick/input/inventory.rs @@ -225,6 +225,10 @@` and `b460f313cc`
  `tick/input/inventory.rs @@ -483,6 +483,10 @@`/`@@ -498,6 +502,10 @@` — (a) the same click-input rows;
  no registered ref of this row covers them (the delta review's N4), so carrying them would add unproven
  reversals to this control.
- `c7bd44dd06` `tick.rs @@ -75,6 +75,9 @@` (module wiring) and `vanilla_interact.rs @@ -125,8 +125,11 @@`
  (dragon-egg guard) — (a) other rows; the egg's own row stays open on its client gate.
- `c7bd44dd06`/`b460f313cc` test hunks (`tick/stations_tests.rs` creation and its inline move into
  `tick.rs`, `interact_parity_tests.rs`) — (b) kept so the control compiles and runs the ref.

## Proving ref (file:line of the failing assertion)

- `cargo:mc-core:tick::cold_menu_residency_tests::anvil_damage_waits_for_a_resident_menu_coordinate` —
  `crates/mc-core/src/tick.rs:1107-1110` `"damage wrote a cold menu coordinate into the overlay"`: with
  the guard reversed, `damage_anvil` damages the overlay image and pushes a block change, so
  `changes.blocks.is_empty()` fails on the first loop iteration.
