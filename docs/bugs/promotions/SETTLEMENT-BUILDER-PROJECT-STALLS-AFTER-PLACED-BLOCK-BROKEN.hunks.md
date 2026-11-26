# SETTLEMENT-BUILDER-PROJECT-STALLS-AFTER-PLACED-BLOCK-BROKEN — control hunks

The fix under test is `95dc2a9bd` (fix(settlements): requeue broken builder project cells). Its control
is the tree without it: `if !paused {` at the `recover` entry is the historical line, and every other
hunk is a pure deletion of text the fix added.

`patch:ad2a7bf7` (five hunks) is HEAD-adapted, not the historical pre-image: `recover`'s head now
carries the upgrade lane's committed-cell scan, and `finish_project`'s tail was rewritten twice (the
shared `certificate_bounds` helper replaced the refusal warnings, and the stale terminal-index comment
is gone). The adaptation re-anchors the same removals on the current text; the delta against the
previous `patch:470330e2` is context only.

| hunk | site | pre-fix state |
|---|---|---|
| 1 | `settlement_builds::recover` entry | `if !paused {` (no `committed` arm) |
| 2 | `recover`, before the liquid revalidation | no repair scan (`let mut repair` … `if committed && repair.is_empty()`) |
| 3 | `recover`, before `resource_plan_hash` | no repair-stage tail consumer |
| 4 | `finish_project`, before the committed check | no `executor::recover` guard |
| 5 | `finish_project`, before the certificate geometry | no `executor::reopen_emptied` guard |

## Hunks of the origin patch that this control leaves out, and their class

`docs/bugs/promotions/SETTLEMENT-BUILDER-PROJECT-STALLS-AFTER-PLACED-BLOCK-BROKEN.origin.patch` is
the original seven-hunk patch. The differences:

- **(a) another row's fix — excluded.** The origin patch's `reopen_emptied` skip list adds
  `|| harvestable_crop(target, cell)` to the record-cell loop. That line belongs to
  `SETTLEMENT-HARVEST-REOPENS-FARM-REPAIR` (`dde9a9e72`, its `repro_check`), not to this row, so the
  adapted control keeps it. It cannot change these reds (no crops in the scenarios) and reversing it
  would promote one row by re-breaking another.
- **(b) helper kept.** The `reopen_emptied` function body and `REPAIR` (the origin patch creates both)
  stay in the control. Reversing them would delete the upgrade lane's inherited-cell loop and the
  repair-stage bookkeeping the file needs; the behaviour under test is removed by hunk 5 (its caller),
  and the rest is dead code in the control, not a second re-break.
- **(c) comments and bookkeeping without behaviour.** The `recover` doc comment that names
  `reopen_emptied`, and the `record.image`/`blueprint_hash`/`remaining` rewrites inside the origin
  patch's tail. They change no cell, no stage and no charge, so they cannot carry a red; keeping them
  keeps the control compiling with the rest of the branch.
- **(c) the repair-truncation block.** `recover`'s re-derivation of an existing `repair` stage
  (`truncate` before re-pushing) is bookkeeping for a case the control never reaches once hunks 1-3
  remove the scan; it stays.

`git apply -R --check` is clean on the promoted tree (`tree_head` 757cdd77b8). The reversed tree's three cargo refs red at
`complete_authored`'s `construction did not finish` (`construction_fencing_tests.rs:387`, the line the
row names in `proving_assertions`) and its house gate reds `house.broken_cell_replaced` — the L1
stage's injected break is never re-placed, the flow records
`_verdict(sub, "broken_cell_replaced", "FAIL", …)` before `probe.check` raises, and the L1 certificate
never appears. The candidate gate `20261125T175716-buildings-qa-s1_5wshf` has all four refs PASS.
