# ISSUE-1-17-NO-PERIMETER-WALL-GATE-OR-WELL — HEAD re-break control patch

Patch: `docs/bugs/promotions/ISSUE-1-17-NO-PERIMETER-WALL-GATE-OR-WELL.patch`
(`patch:37a0f28ae2527ee6…`), the row's `fix_commit`. It reverses on the clean
committed tree at `b67e1b0612` (`git apply -R --check`, plain context) and is
what a promotion passes as `--fix-patch`.

## What the fix is (origin `patch:fba7b70b…`, HEAD form of `Stage2Edge`)

`fortification_site` is consulted before street work: from `FORTIFY_RESIDENTS`
(10) residents the planner admits the next funded piece of the perimeter wall,
its gates or the well, from `layout::fortification`. Before the fix the site
pass went straight to `street_site`, so at most one wall section was ever
asked for and no gate or well was placed.

## HEAD equivalent: the admission hunk

At HEAD the pass reads
`fortification_site(state, record, residents, &images).or_else(|| street_site(...))`.
The control removes that arm and calls `street_site` directly, exactly the
pre-fix admission: walls, gatehouses and the well are never admitted.

## Omissions

- `const FORTIFY_RESIDENTS` and the whole `fortification_site` function (HEAD:
  `crates/mc-core/src/settlements/construction.rs`, with its later
  `jointly_fundable` guard) — **(b)**: the kept test
  `settlements::construction::construction_parallel_tick_tests.rs:565/572/581`
  calls `fortification_site` directly and `fewer_than_ten_residents_or_no_stock_start_no_fortification`
  pins the constant's threshold; deleting either stops the control from
  compiling.

## Reverse-apply evidence

`git apply -R --check docs/bugs/promotions/ISSUE-1-17-NO-PERIMETER-WALL-GATE-OR-WELL.patch`
on the committed tree `b67e1b0612` succeeds.

Control run (reversed tree, then restored): `python3 -m tools.harness run test
-p mc-core --lib settlements::construction::fortification_tests::ten_residents_wall_the_town`
→ `.analysis/validation/20261123T195751-test-81nfdwu7` (`status: failed`,
`dirty` tree at head `ff1ad2bfc8`). The failure is at the row's own assertion
`crates/mc-core/src/settlements/construction_fortification_tests.rs:174`
(`assert_eq!(order[0], WELL, "the well comes first")`): the admission order is
empty because no wall, gate or well is ever admitted (`index out of bounds: the
len is 0 but the index is 0`). The graphical
`harness:buildings-qa:perimeter.enclosed` red is the integrated sweep's control
run, not claimed here.
