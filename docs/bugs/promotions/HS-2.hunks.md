# HS-2: patch-mode fix identity

Original fix: `6948e64e1` (`fix(core): refresh hopper viewers after vehicle transfers`) — when a hopper
pulls an item out of a container vehicle above it, the vehicle synced its own viewers but the hopper's open
menu was never told, so it stayed stale. The commit made `crate::vehicles::pull_into_hopper` take its own
branch and call `changed(state, pos)`.

The commit no longer reverses on HEAD (`redstone/containers.rs` gained the composter branch and moved the
call into a different `if` shape), so the row is a scoped patch:
`fix_commit = "patch:b40603f4f3cf1584ad694b5fe3124c2cb4e76f6785fff6a14434bf450efd758f"` in
`docs/bugs/promotions/HS-2.patch`. Its a/-side is the real pre-image of `6948e64e1^`: the original `else`
branch `crate::vehicles::pull_into_hopper(state, above, pos) || suction(state, pos, above)`. The
composter/`take_composter` context around it is HEAD's later code, kept so the reversed tree still compiles.
Re-audit:
`python3 -m tools.harness ledger promote HS-2 --fix-patch docs/bugs/promotions/HS-2.patch --dry-run`.

## Carried

- `crates/mc-core/src/redstone/containers.rs` `@@ -1055,11 +1055,15 @@` — the `pull_into_hopper` branch that
  calls `changed(state, pos)` before reporting the move (`crates/mc-core/src/redstone/containers.rs:1061-1065`).

## Not carried

- `crates/mc-core/src/vehicles_tests.rs` — (b) the kept test that is the row's ref.
- The composter branch and `take_composter` (later commits) — (c) rewritten after the fix; the context is
  kept so the reversed tree still compiles. It is not the row's behaviour.

## Proving refs (file:line of the failing assertion)

- `cargo:mc-core:vehicles::tests::hopper_pulling_from_a_container_vehicle_refreshes_its_open_menu` — with the
  branch reversed the hopper viewer never sees the pulled apple while the transfer itself still succeeds, so
  the ref fails at its own `assert!(refreshed, "the hopper viewer sees the pulled apple")`
  (`crates/mc-core/src/vehicles_tests.rs:1650`). The neighbouring
  `assert_eq!(block_entities[..].slots[0].item_id, apple)` stays green in the control: it proves the
  transfer, not the stale-menu symptom.
- **The real-client ref, delivered 2026-11-25.** The ingress that did not exist
  (`summon` refused `Items` for a container vehicle: `command_summon::supports` had no vehicle container
  field, observed as a cart that never appeared in
  `20261124T120607-native-v2-ledger-hopper-vehicle-kua__3jb`) now exists: commit `89f9b15d5` accepts vanilla
  `Items` cargo on the kinds whose `vehicles::container_size` is non-zero and validates the list shape, with
  `cargo:mc-core:command_summon::tests::container_vehicle_summon_carries_vanilla_items_cargo`
  (`crates/mc-core/src/command_summon_tests.rs:1323`) as the ingress ref. The gate
  `harness:entities:native-v2-ledger-hopper-vehicle` (`tools/harness/backends/native_ledger_hopper_vehicle.py`)
  opens the hopper's own menu, summons a chest minecart carrying one diamond in the block above it, and
  asserts the open `HopperMenu` receives the pulled diamond
  (`open_hopper_menu_receives_the_pulled_item`) before closing and reopening to show the same diamond
  (`transfer_reached_the_hopper`), so a red is the stale menu, not the ingress. The ingress change is
  instrumentation for this row, **not** its fix: the fix under test is only the `changed(state, pos)` hunk
  above. Passed receipts: gate `20261125T020116-native-v2-ledger-hopper-vehicle-jkhhpt_v`, promotion
  `20261125T023318-ledger-promote-2wfn6c8g`.
- **The control cannot run the graphical ref**: the control trees have no reverted-pin build, so
  `harness:entities:native-v2-ledger-hopper-vehicle` is `unrunnable` there and the red comes from the cargo
  ref at `vehicles/../vehicles_tests.rs:1650` (the `#[path = "../vehicles_tests.rs"]` include makes the panic
  path un-normalised; `proving_assertions` repeats it verbatim). POL-1 practice.
