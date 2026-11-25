# AUDIT-SCULK: patch-mode fix identity

Original fix: `cc3084d7e` (`fix(core): restore scaffolding boats bubble columns and reactive
blocks`) — it added the copper bulb as a reactive block: `redstone::reactive("copper_bulb")`, the
bulb's own tick that toggles `lit` on a rising `powered` edge and mirrors `powered`, and the bulb's
analog output. That is the downward half of the row's chain: the DomWorld latch feeds a copper bulb
one block *below* its dust, and without the bulb's tick the bulb never latches. The row was found by
the gate audit after this guard existed, so the fix is a guard older than the finding **and what that
proves is narrower than the row title**: the control shows only "without the copper bulb's latch the two
bulbs stay dark"; it cannot show that `cc3084d7e` caused the DomWorld expected-lit failures.
`BUGS.md:546` files the row under "Test / harness … excluded from product counts" as *"74 expected-lit
failures, 86 passes; root cause unisolated… not yet a proven product defect"*, and `cc3084d7e`
(2026-10-29) was already in the tree while those failures were recorded — the failing audit ran on a
tree whose `redstone/mod.rs` already carried the bulb arm (pre-image trees such as `wt/PerfABOld` do not
have it, so the reversal is still a real pre-image, just not the audit's cause). The row therefore closes
as **missing coverage for the downward latch**, not as the audit's root cause; N1 of REVIEW-wave2-3.
The commit's three hunks still reverse on HEAD unchanged and the row uses the real pre-image as a scoped
patch:
`fix_commit = "patch:b3845d0732f3086bf094d6a04db368476e1478a763b85e3bbb22211af93ecdb8"` —
the value is printed by `ledger promote --dry-run`; the file is
`docs/bugs/promotions/AUDIT-SCULK.patch` (`git show cc3084d7e --format='' -- …/redstone/mod.rs
…/redstone/signal.rs`). Re-audit:
`python3 -m tools.harness ledger promote AUDIT-SCULK --fix-patch docs/bugs/promotions/AUDIT-SCULK.patch --dry-run`.

## Carried

- `redstone/mod.rs` `reactive()` — `|| path.ends_with("copper_bulb")`.
- `redstone/mod.rs` `tick_cell()` — the `_ if path.ends_with("copper_bulb")` arm: the `lit` toggle
  on a rising edge and the `powered` mirror.
- `redstone/signal.rs` `analog()` — `_ if path.ends_with("copper_bulb") => Some(lit ? 15 : 0)`.

## Not carried

- Everything else in `cc3084d7e` (scaffolding, boats, bubble columns, mount and vehicle motion,
  player movement) — (a) other rows and other lanes.
- `crates/mc-core/src/redstone/copper_bulb_tests.rs` — (b) kept so the control compiles; those refs
  belong to the commit's own unit coverage, not to this row, and are not cited as proving refs.

## Proving refs (file:line of the failing assertion)

- `cargo:mc-core:sculk::tests::note_frequency_reaches_downward_latches_after_sensor_replacement` —
  with the patch reversed the two dust-fed copper bulbs keep `lit = false`, so the ref fails at its
  own `assert_eq!(prop(&s, frequency_bulb, "lit"), "true")`
  (`crates/mc-core/src/sculk_tests.rs:361`, the `assert_eq!` at `:362` is its second argument line) while the sensor phase and comparator assertions above
  it stay green: the chain carries the vibration, only the downward latch is missing.
- `harness:entities:native-v2-ledger-sculk-downward` — a real client reads the dust power, then both
  downward bulbs' `lit` in the same chain (`tools/harness/backends/native_ledger_sculk_downward.py`).
  Passed receipt: see the promotion JSON.
