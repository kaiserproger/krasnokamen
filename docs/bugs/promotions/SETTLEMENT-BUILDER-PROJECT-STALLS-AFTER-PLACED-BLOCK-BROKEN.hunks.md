# SETTLEMENT-BUILDER-PROJECT-STALLS-AFTER-PLACED-BLOCK-BROKEN — control hunks

The fix under test is `95dc2a9bd` (fix(settlements): requeue broken builder project cells). Its
control is the tree without it: `if !paused {` at the `recover` entry is the historical line, and every
other hunk is a pure deletion of text the fix added.

The recorded patch is HEAD-adapted, not the historical pre-image: `recover`'s head and
`finish_project`'s tail were both rewritten after the fix landed (the upgrade-cell lane's
`inherited()` scan in `reopen_emptied`, and this lane's second pack, which replaced the
`finish_project` refusal warnings with the shared `certificate_bounds` helper). The adaptation
re-anchors the same six removals on the current text; its delta against `patch:b15befd9` is context
only, never a different behaviour to remove.

| hunk | site | pre-fix state |
|---|---|---|
| 1 | `settlement_builds::recover` entry | `if !paused {` |
| 2 | `recover`, before the liquid revalidation | no repair scan (`let mut repair` … `if committed && repair.is_empty()`) |
| 3 | `recover`, before `resource_plan_hash` | no repair-stage tail consumer |
| 4 | `reopen_emptied` skip list (the record-cell loop) | `free_soil_work` only, no `harvestable_crop` |
| 5 | `finish_project`, before the committed check | no `executor::recover` guard |
| 6 | `finish_project`, before the certificate geometry | no `executor::reopen_emptied` guard |

`patch:470330e2` is `git apply -R --check`-clean on `2d5735657` (the head this row is re-promoted on).
The reversed tree's three cargo refs red at `complete_authored`'s `construction did not finish`
(`construction_fencing_tests.rs:385`, the line named by the row's `proving_assertions`) and its house
gate reds `house.broken_cell_replaced`: the L1 stage's injected break is never re-placed, so the L1
certificate never appears. The candidate gate on the same head has both verdicts PASS.
