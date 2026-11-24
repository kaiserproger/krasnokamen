# Control map — SETTLEMENT-LAYOUT-PROJECT-SHOWS-UNKNOWN-BLUEPRINT

Patch: `docs/bugs/promotions/SETTLEMENT-LAYOUT-PROJECT-SHOWS-UNKNOWN-BLUEPRINT.patch`
(production hunk only; `patch:7115b0d628472efc2bc6467230c260f7fbc86ec0bdc14ce5463350d2bbdd5a01`,
the file's own sha256 because the file holds no test hunk).
Regenerated 2026-11-24 at HEAD `f5fdd9f0bd` because the earlier copy of `patch:cf80f14a`
no longer reverse-applies (its `@@ -794` context predates the furnace name line `a895cc853b`).

## Original commit

`49151c3a6` — fix(settlements): plan no wall stretch without a builder stand; name layout
projects. It closes two rows; only the naming part belongs to this row.

## Hunks

| original hunk | file | HEAD equivalent | status |
|---|---|---|---|
| `ui_text.rs @@ -794,6 +794,9` — adds `perimeter` (Wall section), `perimeter_gate` (Gate), `well` (Well) to `blueprint()` | `crates/mc-core/src/settlements/ui_text.rs` | the same three added lines; only the neighbouring `"furnace"` line came later (`a895cc853b`), so the hunk is re-anchored at 794 with unchanged context | **carried** |
| `ui_text.rs` test hunk — `every_layout_infrastructure_project_has_a_display_name` | same file, inline `#[cfg(test)]` module | unchanged at HEAD | stays (test hunk; the splitter keeps Rust hunks inside an inline `#[cfg(test)]` module) |
| `layout.rs` — `stand_columns`/`span_work` wall-stretch refactor | `crates/mc-core/src/settlements/layout.rs` | unchanged at HEAD | omitted **(a)**: another row's behaviour — `SETTLEMENT-WALL-STRETCH-WITHOUT-STAND`, promoted with its own control `patch:f630747d`; reverting it here would turn the control red on that row's assertion |
| `construction_fortification_tests.rs` | test file | unchanged | stays (test file) |
| `docs/settlements/construction.md`, `docs/bugs/**` | docs | unchanged | excluded by the splitter (docs stay) |

## a/-side adaptation

None. Every deleted line of the patch is the current file's own text (the pre-image of
`49151c3a6` for this hunk), so the reversed tree is byte-identical to the pre-fix code for
this behaviour.

## Proving ref

`cargo:mc-core:settlements::ui_text::tests::every_layout_infrastructure_project_has_a_display_name`
fails on the reversed tree at `assert_ne!(blueprint(Lang::En, "perimeter"), "Unknown blueprint")`
— the row's own symptom (the status card names the project). Recorded control:
`.analysis/validation/20261123T155230-test-a40xgdpk` (the earlier patch's red run; re-run by
`ledger promote` on this patch).
