# SETTLEMENT-BUILDER-PROJECT-STALLS-AFTER-PLACED-BLOCK-BROKEN — control hunks

The fix under test is `95dc2a9bd` (fix(settlements): requeue broken builder project cells).
The control is the tree without it, built from the real pre-image text in that commit's history:
`if !paused {` at the `recover` entry is the historical line, and every other hunk is a pure
deletion of text the fix added.

| hunk | site | pre-fix state |
|---|---|---|
| 1 | `settlement_builds::recover` entry | `if !paused {` |
| 2 | `recover`, before the liquid revalidation | no repair scan (`let mut repair` … `if committed && repair.is_empty()`) |
| 3 | `recover`, before `resource_plan_hash` | no repair-stage tail consumer |
| 4 | `reopen_emptied` skip list | `free_soil_work` only, no `harvestable_crop` |
| 5 | `finish_project`, before the committed check | no `executor::recover` guard |
| 6 | `finish_project`, before the blueprint lookup | no `executor::reopen_emptied` guard |

`git apply -R --check` passes on `90e855ae4f`; the reversed tree compiles
(`cargo check -p mc-core --lib`) and its refs are red at the row's own stall
(`construction did not finish`, broken cell still air, `20261123T200456-test-5yvracl_`).
