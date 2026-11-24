# SETTLEMENT-BUILDER-PROJECT-STALLS-AFTER-PLACED-BLOCK-BROKEN — HEAD re-break control patch

Patch: `docs/bugs/promotions/SETTLEMENT-BUILDER-PROJECT-STALLS-AFTER-PLACED-BLOCK-BROKEN.patch`
(`patch:7df366247147e68b…`), the row's `fix_commit`. It reverses on the clean
committed tree at `b67e1b0612` (`git apply -R --check`, plain context) and is
what a promotion passes as `--fix-patch`.

## What the fix is (origin `patch:7212599e…`, kept whole in `promotions/SETTLEMENT-BUILDER-PROJECT-STALLS-AFTER-PLACED-BLOCK-BROKEN.origin.patch`, HEAD form of the breaker fix)

A player breaking an already-placed cell of an active project left a hole that
was never re-placed: the project stayed active at 250/250 and held the only
slot. The fix queues an emptied placed cell into the `repair` stage:

1. `settlement_builds::recover` scans the finished cells of `liquid`/`fill`/
   `build` stages and appends the emptied ones as a newly certified `repair`
   stage (a solid foreign block is left alone);
2. `finish_project` runs that scan for a committed structure
   (`reopen_emptied`) and keeps the project open when it queued a repair;
3. `finish_project` first calls `executor::recover` so a paused structure is
   resumed before completion is judged.

## HEAD equivalents

All three effects exist at HEAD in the current ledger API and are reversed:

- `settlement_builds.rs` hunk 1: `let committed = ...` / `if !paused && !committed`
  back to the pre-fix `if !paused`;
- `settlement_builds.rs` hunks 2-3: the repair scan and the `repair` stage push
  removed again, so no emptied cell is ever queued;
- `construction.rs` hunks 4-5: the `executor::recover` guard and the
  `executor::reopen_emptied` guard removed from `finish_project`.

## Omissions

- The doc-comment lines of the original `settlement_builds.rs` hunk (the
  paragraph about queued repairs) — **(c)**: a later commit rewrote that prose
  ("Completed fill/build damage is appended as a newly certified repair
  portion…"), so the pre-fix sentence has no HEAD equivalent.

The control keeps the later `retry_preparation` and liquid `revalidate` paths
of `recover` for a *paused* structure: they belong to other rows (owner-order
re-siting, N3 sapling cancel, liquid proofs) and reversing them would turn
their assertions red — **(a)**. With the early-out restored they are
unreachable for a committed structure, which is the pre-fix state.

## Reverse-apply evidence

`git apply -R --check docs/bugs/promotions/SETTLEMENT-BUILDER-PROJECT-STALLS-AFTER-PLACED-BLOCK-BROKEN.patch`
on the committed tree `b67e1b0612` succeeds.

Control run (reversed tree, then restored): `python3 -m tools.harness run test
-p mc-core --lib settlements::construction::fencing_tests::a_placed_cell_broken_to_air_is_replaced`
→ `.analysis/validation/20261123T200456-test-5yvracl_` (`status: failed`,
`dirty` tree at head `dd94645dda`). The failure is the row's own stall: the
helper at `crates/mc-core/src/settlements/construction_fencing_tests.rs:346`
panics "construction did not finish" with the status
`house · level 1 · 100% / Blocks: 249/249 · Committed` and materials still
outstanding — the broken cell is never re-placed and the project stays open at
249/249. (A first attempt that reversed only `reopen_emptied`, leaving the
`recover` scan, stayed green: `.analysis/validation/20261123T200022-test-qj1pdsyz` —
that is why the scan is part of this control.) The graphical
`harness:buildings-qa:house.broken_cell_replaced` red is the integrated
sweep's control run, not claimed here.
