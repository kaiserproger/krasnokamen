# SPAININTEG-F5 released-carry port — review (committee-sol)

Reviewer: committee-sol (OpenAI), read-only. Tree `wt/SpainF5`
`cefaf35e24d3d5b44cd4e9760df8cbe044b4e849`, commits `3ebf01d11` (fix),
`fb5dcc7b8` (test), `cefaf35e2` (docs). Port of `wt/SpainInteg` `18ea1a5c3` and
`33554901a`. Date 2026-11-28 ~17:55 host.

## Verdict

`changes` — keep the atomic handoff, but pin source identity through selection
and prove recovery of a non-zero remaining duty.

VOTE: amend.

## What passed

- **(a) Conservation and refusal without debit.** `settlement_builds_carry.rs:6–49`
  deducts only `min(physical stock, duty)` from the shared account balance for an
  available, unoccupied resident; stock is not copied into the bank. The handoff
  is bounded by the duty, the trip remainder and the receiver's capacity. Before
  `inventory::commit` only local images change, and the duty drops only after a
  successful commit (`settlement_builds_carry.rs:147–202`,
  `settlements/inventory.rs:297–351`). No double credit found on this path.
- **(c) The two declared restart outcomes** (`native_construction_carry_drill_tests.rs:25–104`,
  `native_persistence_drill_tests.rs:146–185, 266–293`; both green in receipt
  `20261128T102418-…/test.log:347–351`). Explicitly not a power-loss test and
  not a proof of replaying a partially reduced duty.

## Must-fix

1. **A route coordinate does not certify the source.** `pickup_point` picks the
   nearest donor but returns only a cell; `donor()` then re-picks the first
   matching debtor in that cell, so two debtors in one cell can route to one body
   and debit another. Bank versus resident has the same ambiguity: routing
   prefers the bank, but `fetch` tries the debtor **before** the bank endpoint, so
   a shared cell silently swaps in the resident even when the bank has the item
   (`settlement_builds_carry.rs:52–119`;
   `settlement_builds.rs:1728–1740, 2099–2127`; `villager/building.rs:358–363,
   650–665`). The 2.5-block threshold is correct for an already chosen body but
   does not protect source identity.
2. **The bank preference is not checked against a real debit.** `can_craft`
   probes one unit with no reservations while the real fetch charges the shared
   wood account; a full bank with 4 planks and 2 sticks can probe a fence through
   but refuse the spend, after which the builder keeps walking to the bank while
   the debtor holds a finished fence (`construction_crafting.rs:422–460`;
   `settlement_builds.rs:1760–1835, 2112–2114`).
3. **Two tests pass on a default.** The bank-priority test asserts only the
   coordinate of the single bank and passes if the donor branch always returns
   `None` (`construction_carry_release_tests.rs:435–444`). The "smaller duty"
   test and the published WAL drill both exhaust the duty fully, so an
   implementation that erases the whole duty after any successful handoff passes
   (`construction_carry_release_tests.rs:323–375`;
   `native_construction_carry_drill_tests.rs:53–62, 92–104`). For `tick`, add the
   boundary where the body is inside the radius but the cell centre is not
   (`construction_carry_release_tests.rs:409–425`).
4. **Controls are hand-cut, not Git pre-images.** Control B leaves
   `let _ = (wanted, position)` instead of restoring the pre-port `plan` check and
   its red lands on `fetch().unwrap()` before the WAL write and restart, so it
   does not test replay. Control D reverts both the `exact` selection and the
   `tick` check but only `target.exact` reds. Receipts `100808`, `101055`,
   `102145`.

## Scope note

The row keeps `repro = "client"`: a real-client run is required before promotion;
the existing receipts are Cargo only. Reviewer ran no tests.
