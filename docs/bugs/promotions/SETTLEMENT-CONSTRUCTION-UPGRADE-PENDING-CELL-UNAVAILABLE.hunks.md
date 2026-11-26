# SETTLEMENT-CONSTRUCTION-UPGRADE-PENDING-CELL-UNAVAILABLE — control hunks

The fix under test spans three packs of one lane (`983178bde2` + `2d5735657` + the box-filter pack),
all committed before the promoted head. The recorded pre-image is the lane base, `597352d5a7`,
whose `reopen_emptied` scans only the record's own cells — no inheritance at all.

`docs/bugs/promotions/SETTLEMENT-CONSTRUCTION-UPGRADE-PENDING-CELL-UNAVAILABLE.patch`
(`patch:35c376cb`) is `git diff 597352d5a7 <head> -- settlement_builds.rs blueprints.rs`. The net
`blueprints.rs` diff against that base is empty (pack 1 added `Blueprint::image`/`Blueprint::inherited`,
pack 2 deleted them), so the patch is one file: `settlement_builds.rs`, where the `reopen_emptied` loop that consumes the
fold is added. The helpers themselves (`inherited`, `blueprint_origin`, `inside`) stay in the control
as a kept helper (class (b), the same convention as the builder row): the row's two `preparation_tests`
regressions call them, so deleting them would make the control's own test build fail and every cargo
ref `unrunnable`. Removing the loop is what removes the behaviour - the inherited cell is never queued
and the certificate never closes.

| hunk | site | pre-fix state |
|---|---|---|
| 1 (kept) | `settlement_builds.rs`: the inherited-cell loop in `reopen_emptied` | absent (`597352d5a7` text: the scan reads `record.cells` only) |

What the fold does, and why each part is guarded: the chain of committed records of the same site is
folded last-write-per-position, only cells **inside the record's own box** survive (a level-1 slope
foundation's fill below the level-2 floor and its detour or retired cells are outside it, and such a
cell in the `repair` stage makes `plugin_structures::validate` refuse the world on the next load), and
the chain is keyed on the **blueprint origin** (`bounds.min.x`, `bounds.max.y - height + 1`,
`bounds.min.z`) rather than `snapshot.origin`, because a slope foundation's site minimum is its fill
bottom and `same_site` compares the blueprint origin.

Candidate refs: the two `construction_fencing_tests` regressions (the inherited perimeter cell with the
ledger charge, and the species case) plus `harness:buildings-qa:house.L2_upgrade`. The control gate
reds at `buildings_house_L2_built` (the level-2 certificate never appears), the candidate gate
`20261125T175716-buildings-qa-s1_5wshf` passes L1-L3.

Two more regressions guard the box filter and the origin derivation directly
(`plugin_structures::settlement_builds::preparation_tests::an_emptied_level_fill_below_the_upgrade_box_never_reaches_the_repair_stage`
and `::the_inherited_chain_reads_the_blueprint_origin_not_the_site_minimum`). They are named in the
row's `repro_check` but not registered as refs: they call the new helpers, so they cannot compile on
the pre-image and a control run would report them `unrunnable` rather than red.
