# SETTLEMENT-MAP-SITE-CLICK-LOSES-PROJECT — fix and re-break control

Fix commit: `f8f27afda8` (this branch). There is no stored control patch: the
hand-written one that was here had an invented a/-side (`selected_project_screen`
did not exist before the fix), so it was deleted. The honest control is the one
`ledger promote` ran for the promotion (`20261124T030635-ledger-promote-hto7rv_j`):
it reverses the fix's real UI hunks on the candidate tree, and the real-client gate
then reads `selected_card_site_only=false` from the aggregate screen.

## Provenance

The behaviour was verified by the owner on the isolated candidates
`b4d49f18d804-02f507b4` (green: distinct Storage 1/139 and House 4/249 cards,
back to the exact map) and `…-23cb7ab2` (red: both projects in one aggregate
dialog). That production change was uncommitted work in `wt/QwDV5RouteV2`;
Main authorized reading it and this commit ports it onto `wt/D123Civ`
(`ui.rs` dispatch arm + `selected_project_screen`, `ui_map.rs` routes,
`ui_menus.rs` `Target::Project`, docs, and five DV5 regressions in
`ui_project_selection_tests.rs`).

## Reverse and red

The only control is the one `ledger promote` ran for the promotion
(`20261124T030635-ledger-promote-hto7rv_j`): it reverses the fix's real UI hunks on the
candidate tree — `selected_project_screen` renders the settlement-wide status again
(aggregate) while every routing symbol stays — and the real-client gate then reads the
row's own fact `selected_card_site_only=false` from the aggregate screen. The candidate's
104 `settlements::ui` tests pass (`.analysis/validation/20261123T191304-test-okwmmwow`).

There is no cargo control for this row: the reversed non-test hunks remove
`ui::menus::Target::Project`, which the kept DV5 tests call, so the reversed tree cannot
build (E0599, promote run `20261124T022007`, `ledger-0-list.log`) and the real-client ref
carries the control alone. The `harness:settlement-map-qa:selected_card` step is written
(native windowed, civil fixture plus a second ordered House), its candidate receipt is
`20261124T021057-settlement-map-qa-56yktcf4`, and the row is promoted on it.
