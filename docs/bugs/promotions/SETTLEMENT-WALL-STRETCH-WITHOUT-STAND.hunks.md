# SETTLEMENT-WALL-STRETCH-WITHOUT-STAND — forward-ported fix patch

Forward patch: `docs/bugs/promotions/SETTLEMENT-WALL-STRETCH-WITHOUT-STAND.patch`
(`patch:f630747ddc130aadca7dd1f483db925ac2eb8783a896b7c40feb110d13179abd`), the
row's `fix_commit`. It applies in reverse on the candidate `wt/D123Civ`
(`git apply -R --check`).

## Why patch mode

The fix landed as `49151c3a6` together with another row's change
(`ui_text::blueprint` names, row
`SETTLEMENT-LAYOUT-PROJECT-SHOWS-UNKNOWN-BLUEPRINT`), so the commit's non-test
hunks cannot be used whole. The patch is that commit's `layout.rs` hunks,
forward-ported to HEAD after the `span_work` stand check moved inside the
column loop (per-span `HashSet` removed) — the current code, nothing else.

## Hunks → HEAD equivalents

| hunk | HEAD equivalent | disposition |
|---|---|---|
| `stand` builds its stand table through the new `stand_columns` fn (single stand table for `stand` and the planner) | `layout.rs:1265-1290` | **included** |
| `span_work`'s doc line gains the no-stand drop | `layout.rs:1500-1503` | **included** |
| `span_work` refuses a stretch that owes work on a column with no builder stand beside it | `layout.rs:1533-1546`, checked per column inside the loop (no per-span `HashSet`) | **included** |
| `ui_text.rs` layout names (another row) | — | **omitted, reason (a)**: `SETTLEMENT-LAYOUT-PROJECT-SHOWS-UNKNOWN-BLUEPRINT` owns their reversal; including them would turn that row's assertion red |
| `construction_fortification_tests.rs` (test) | present | kept (test) |

No a/-side adaptation.

## Proving ref

`cargo:mc-core:settlements::construction::fortification_tests::a_wall_stretch_without_a_builder_stand_is_not_planned`
— red at its admission assertion with the patch reversed, green on the restore.

## Residual scope

Only the `stand == None` drop is fixed and proved. The row's same-field
`worksite_unreached` fast-ab `[107,77,35]` case is not reproduced here.
