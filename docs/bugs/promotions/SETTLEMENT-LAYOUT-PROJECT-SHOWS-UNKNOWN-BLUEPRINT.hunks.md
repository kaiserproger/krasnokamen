# SETTLEMENT-LAYOUT-PROJECT-SHOWS-UNKNOWN-BLUEPRINT — HEAD-adapted name patch

## Origin

The fix is `49151c3a6` ("fix(settlements): plan no wall stretch without a builder stand; name
layout projects", 2026-11-23). Its `ui_text.rs` hunk adds the three built-in layout kinds:

```
+        "perimeter" => ("Wall section", "Секция стены"),
+        "perimeter_gate" => ("Gate", "Ворота"),
+        "well" => ("Well", "Колодец"),
```

before `"road"`, and the same commit adds the regression test
`every_layout_infrastructure_project_has_a_display_name` inside `mod tests`.

## Why this patch is the HEAD equivalent, not the commit's diff

A later commit, `a895cc853` ("settlements: name the autonomous furnace project", 2026-11-23),
inserted `"furnace" => ("Furnace", "Печь"),` exactly where the three arms end. The origin
commit's hunk (well followed by `"road"`) therefore no longer reverse-applies at HEAD. This
patch carries the same three lines at HEAD's position, between `"spanish_wall"` and the
newly inserted `"furnace"` arm; the a/-side is HEAD's real text (it exists: the arms are in
`ui_text.rs:797-799`), only the hunk context differs. `patch:7115b0d6…` is its digest.

## What is left out

- The commit's test hunk (class (b), a kept test): the row's regression test must survive the
  reversal, otherwise the control cannot run the test that proves the symptom.
- Everything else in `49151c3a6` (the `layout::stand_columns` planner change) belongs to
  SETTLEMENT-WALL-STRETCH-WITHOUT-STAND (class (a): another row's behaviour, promoted there).

## Proving refs

- `cargo:mc-core:settlements::ui_text::tests::every_layout_infrastructure_project_has_a_display_name`
  — red on the reversed tree at `assert_ne!(blueprint(Lang::En, id), "Unknown blueprint")` for
  `perimeter` (the reversed map falls through to the `_ => ("Unknown blueprint", …)` arm). The row
  names that panic in `proving_assertions` as `crates/mc-core/src/settlements/ui_text.rs:1021`: the
  reversed patch removes the three naming arms **above** the test, so the control tree's line 1021
  (column 13) is HEAD's `assert_ne!` at line 1024 — the promote's class check reads the candidate
  file, which is why the row carries the control-tree location explicitly (promote artifact
  `20261124T171527-ledger-promote-x_zmjg1r`, panic text `assertion left != right failed: perimeter`,
  `left: "Unknown blueprint"`).
- `harness:buildings-qa:perimeter.project_named` — the settlement's own fortification projects
  (wall section, gates, well) are the ones this fix names; the gate reads the live Construction
  page's project labels through the native client and fails when any active project is
  `Unknown blueprint` or when no layout-infrastructure label is observed during the wall build. A
  real-client receipt of `buildings-qa` `perimeter` covers it; the *graphical* control needs its
  own reverted-tree pin with its own `.analysis/ledger-builds` registration, so the promote records
  it `unrunnable` (wave-2 rule 2) rather than pretending a red.
