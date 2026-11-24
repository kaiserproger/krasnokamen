# ISSUE-1-17-NO-PERIMETER-WALL-GATE-OR-WELL — HEAD re-break control patch

Patch: `docs/bugs/promotions/ISSUE-1-17-NO-PERIMETER-WALL-GATE-OR-WELL.patch`
(`patch:c94fd89091a83c5f…`), the row's `fix_commit`, rebuilt 2026-11-24
(D123CivGatesG) after `f98e94e4c3` moved the fortification arm. It reverses on
the clean committed tree at `997809f4e6` (`git apply -R --check`, plain
context) and is what a promotion passes as `--fix-patch`.

## What the fix is, in two commits

- `e8927d570` (origin, 2026-11-13; HEAD form kept whole in
  `promotions/ISSUE-1-17-NO-PERIMETER-WALL-GATE-OR-WELL.origin.patch`):
  `plan_pass` consults `fortification_site` — the first funded piece of the
  perimeter wall, its gates or the well from `FORTIFY_RESIDENTS` (10)
  residents, from `layout::fortification`. Before the fix the site pass went
  straight to `street_site`, so at most one wall section was ever asked for and
  no gate or well was placed.
- `f98e94e4c3` (2026-11-24, F12): the arm moved **before** the gatherer
  keep-out. With a demanded gatherer staffed (a certified natural
  LumberCamp/Mine, free in a real world) `plan_pass` returned `false` at
  `if gathering` before reaching the funded wall; the 2026-11-24 perimeter gate
  (`20261124T125551`) stood still for 1580 s with 768 cobblestone in storage.
  The keep-out now precedes `street_site` only.

## The carried hunk

At HEAD (line 1868..) the pass reads

```rust
    let Some(record) = state.settlements.get(id) else { return false; };
    if let Some(site) = meadow_site(...) { return begin_cells(...); }
    // The wall, its gates and the well come before street work: ...   <- f98e94e4c3
    if let Some(site) = fortification_site(state, record, residents, &images) {
        return begin_cells(state, site.project, site.bounds, site.cells);
    }
    if gathering { return false; }
    let Some(site) = street_site(state, record, &images) else { return false; };
    begin_cells(...)
```

The control deletes the fix's comment and its `fortification_site` arm and
restores the pre-fix comment, i.e. the true pre-image of `e8927d570^`
(line 1588-1594 of `git show e8927d570^:crates/mc-core/src/settlements/construction.rs`):
`if gathering { return; }`, `// Free street work fills every pass that starts
no building or upgrade.`, then `street_site` and `begin_cells`.

**a/-side provenance.** The single added line in reversed direction is that
pre-fix comment, verbatim from the pre-image blob. Every removed line is the
fix's own text. No adapted or invented a/-side line exists, and no behaviour is
re-break: the reversed pass admits no fortification and is exactly the code
before `e8927d570`. The pre-image ended the pass inside the `None` arm while
HEAD returns `false` through `let … else`; those lines are carried unchanged
from HEAD, so no adaptation is needed.

## Omissions

- `const FORTIFY_RESIDENTS` and the whole `fortification_site` function —
  **(b)**: the kept tests call the function directly
  (`settlements::construction::construction_parallel_tick_tests.rs:565/572/581`)
  and `fewer_than_ten_residents_or_no_stock_start_no_fortification` pins the
  constant's threshold; deleting either stops the control from compiling.
- `layout::fortification`, the gate/well blueprints and the autonomous-kind
  list — already present in the base tree this patch reverses on; they are the
  data the kept tests exercise and are not this row's admission path.
- The test additions of both commits (`construction_fortification_tests.rs`,
  `construction_planner_tests.rs`, `layout_tests.rs`, the harness gate) — the
  promote keeps test hunks, as the contract requires.

## Reverse-apply and control evidence

`git apply -R --check docs/bugs/promotions/ISSUE-1-17-NO-PERIMETER-WALL-GATE-OR-WELL.patch`
succeeds on the clean candidate `997809f4e6` (checked by D123CivGatesG).

Control run of the same reversal on the pre-F12 tree (recorded before the
ordering commit existed): `python3 -m tools.harness run test -p mc-core --lib
settlements::construction::fortification_tests::ten_residents_wall_the_town`
→ `.analysis/validation/20261123T195751-test-81nfdwu7` (`status: failed`,
dirty tree at `ff1ad2bfc8`), red at the row's own assertion
`crates/mc-core/src/settlements/construction_fortification_tests.rs:174`
(`assert_eq!(order[0], WELL, "the well comes first")`: empty admission order,
`index out of bounds: the len is 0 but the index is 0`). The promote of this
rebuilt patch runs the same reversal on the candidate and records its own
control; that receipt is authoritative.

The graphical `harness:buildings-qa:perimeter.enclosed` red needs its own
reverted-tree pin and is recorded `unrunnable` when no build token allows it
(wave-2 rule 2); the candidate's green graphical receipt is the perimeter gate
run this promotion passes as `--client-receipt`.
