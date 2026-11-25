# SETTLEMENT-CONSTRUCTION-UPGRADE-PENDING-CELL-UNAVAILABLE — control hunks

Patch mode is used because the fix is two sessions of one lane: `settlement_builds.rs` and
`blueprints.rs` were changed by D123CivGatesH's pack (committed `2f9d545215`) and then changed again by
D123CivGatesJ's pack (`2d5735657`), which replaced the blueprint-image inheritance with the executed
build chain and deleted `Blueprint::image`/`Blueprint::inherited`; the net `blueprints.rs` diff against
the lane base is empty. The recorded pre-image is therefore the lane base itself, `597352d5a7`, whose
`reopen_emptied` scans only the record's own cells.

`docs/bugs/promotions/SETTLEMENT-CONSTRUCTION-UPGRADE-PENDING-CELL-UNAVAILABLE.patch`
(`patch:27b05b91`) is `git diff 597352d5a7 <head> -- settlement_builds.rs blueprints.rs`: one file,
`settlement_builds.rs`, in which the whole `inherited()` function (the chain fold) and the
`reopen_emptied` loop over it are added. Reversing it restores `597352d5a7`'s text, so the control has
no inherited-cell scan at all.

| hunk | site | pre-fix state |
|---|---|---|
| 1 (kept) | `settlement_builds.rs`: `fn inherited(..)` + the `reopen_emptied` loop | absent (`597352d5a7` text: the scan reads `record.cells` only) |

The two membership cargo refs are red on that pre-image: `20261125T115102` (the lane's own red run on
the base) failed `an_upgrade_repairs_an_inherited_perimeter_cell_emptied_mid_build` at
`construction_fencing_tests.rs:385` (`construction did not finish`). On the pack-2 pre-image
(`2f9d545215`, blueprint-image inheritance) that test stays green, because the image already repairs an
oak inherited cell; the species test
`an_upgrade_repairs_an_inherited_cell_in_the_species_the_house_was_built_with` is the one that reds
there (`20261125T170443`, same assertion), and the cost/`validate` assertions are the pack-2 additions.

The graphical ref `house.L2_upgrade` is red on the pre-image: gate `20261124T231255` FAILed it after the
L2 adversarial break (the L1 stage, including `broken_cell_replaced`, PASSed on that run). The promotion
run re-runs it on the reversed control tree itself.
