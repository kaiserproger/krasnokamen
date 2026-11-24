# COPPER-CHEST-OPEN-WEATHERING: patch-mode fix identity

Two commits close this row's path:

- `3d4893773` (`fix(compat): sync operator permissions and preserve open copper chests`) — `copper.rs`'s
  change-over-time path returns `None` for a copper chest whose custody map has an opener at either half.
  A later commit (`9f032e058 fix(core): publish copper golem container interactions`) rewrote the guard's
  tail from `open_containers.values().any(..)` to `open_containers.count_at(..)` plus its comment, so the
  original hunk no longer reverses.
- `bd7a05a64` (`fix(core): retain battle recovery and reliable client movement`) — death closes the menu
  immediately (`self.open_containers.remove(&id)`), which is the release half the row's second ref needs:
  the last opener dying must let the chest weather again without waiting for a close packet.

The row is a scoped patch:
`fix_commit = "patch:53d630da1960f99a8ab4a8c8066dd463056cdb6371f16061abc41254301f4f3f"` in
`docs/bugs/promotions/COPPER-CHEST-OPEN-WEATHERING.patch`. Both a/-sides are the real pre-images: before
`3d4893773` the guard block did not exist at all (the reversed tree oxidizes an open chest), and before
`bd7a05a64` no `open_containers.remove` call existed in the death path. Re-audit:
`python3 -m tools.harness ledger promote COPPER-CHEST-OPEN-WEATHERING --fix-patch docs/bugs/promotions/COPPER-CHEST-OPEN-WEATHERING.patch --dry-run`.

## Carried

- `crates/mc-core/src/copper.rs` — the whole `if chest(block.block.id.path()) { … }` guard, including the
  double-chest partner lookup and the `open_containers.count_at` opener check at its HEAD form
  (`crates/mc-core/src/copper.rs:130-143`), the HEAD equivalent of `3d4893773`'s hunk.
- `crates/mc-core/src/tick/damage.rs` — the death-path `self.open_containers.remove(&id)` and its comment
  (`crates/mc-core/src/tick/damage.rs:777-778`), `bd7a05a64`'s hunk.

## Not carried

- `9f032e058`'s rewrite itself — (c) a later commit; its HEAD form is what the carried hunk removes.
- The copper-golem and operator-permission hunks of `3d4893773` and the battle-recovery hunks of
  `bd7a05a64` — (a) other rows' behaviour; their reversal would turn the control red on someone else's
  assertion.
- Test files — (b) kept so the control compiles and runs the row's refs.

## Proving refs (file:line of the failing assertion)

- `cargo:mc-core:copper::tests::copper_weathering_waits_for_last_menu_close_in_its_dimension` — with the
  guard reversed the open chest advances its stage while a viewer holds it and the ref fails at its own
  `assert_eq!(family(at(&state, A).block.id.path()).unwrap().1, 0)`
  (`crates/mc-core/src/copper_tests.rs:483`); the control chest's `assert_eq!(…, 1)` right after shows the
  same `weather()` call still advances an unopened chest, so the red is the open-chest symptom.
- `cargo:mc-core:copper::tests::copper_weathering_resumes_after_last_opener_dies_and_respawns` — the second
  half; with the death release reversed the dead opener keeps the menu open
  (`assert!(state.players[&entity].open.is_none())`, `crates/mc-core/src/copper_tests.rs:623`) and with the
  guard reversed the stage advances while the first opener is still alive
  (`assert_eq!(…, expected_age, "weathering after opener … died and respawned")`,
  `crates/mc-core/src/copper_tests.rs:629-633`).
- `harness:entities:native-v2-ledger-copper-chest-open` — a real client opens one of two unwaxed copper
  chests on a fixture pad, the random-tick sampler carries an unopened control through two weathering
  stages, and the opened chest keeps its stage and its 63-slot menu until the menu closes
  (`tools/harness/backends/native_ledger_copper_chest.py`). Passed receipt: see the promotion JSON.
