# SETTLEMENT-MAP-SITE-CLICK-LOSES-PROJECT — fix and re-break control

Fix commit: `f8f27afda8` (this branch). Control patch:
`docs/bugs/promotions/SETTLEMENT-MAP-SITE-CLICK-LOSES-PROJECT.patch`
(`patch:5099c724385fda24…`), which reverses the fix's UI production hunks on
the committed tree (`git apply -R --check`).

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

Reversed, `selected_project_screen` renders the settlement-wide status again
(aggregate) while every routing symbol stays, so the control tree builds. On the
reversed tree the five DV5 tests fail by assertion ("exactly the selected native
facts, not an aggregate", artifact `.analysis/validation/20261123T191740-test-_gm7ipne`);
the candidate's 104 `settlements::ui` tests pass
(`.analysis/validation/20261123T191304-test-okwmmwow`). The graphical control
(the aggregate dialog) belongs to the integrated sweep; the
`harness:settlement-map-qa:selected_card` step is still to be authored.
