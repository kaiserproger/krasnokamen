# SETTLEMENT-BLUEPRINT-LIST-SHOWS-RAW-SPANISH-IDS — hunk map

Row: `SETTLEMENT-BLUEPRINT-LIST-SHOWS-RAW-SPANISH-IDS` (client, major) — "Construction lists Spanish
blueprint IDs instead of localized names". Refs:
`cargo:mc-core:settlements::ui::tests::every_registered_blueprint_has_explicit_russian_and_english_names`,
`cargo:mc-core:settlements::ui::tests::unknown_blueprint_names_never_expose_raw_identifiers`,
`harness:settlement-map-qa:blueprints`.

## Real fix and its pre-image

Core `0c20eb8b2a2f7fedc064b9337c74aa889781ea10` (2026-11-20, "fix(settlements-ui): localize every
registered blueprint") replaced the hand-written `ui::blueprint_label` match with
`ui_text::blueprint`, whose table covers the whole registry (including the five Spanish variants) and
supplies a localized generic name for unknown ids. Later commits rewrote `ui.rs`, so the committed
hunk no longer reverse-applies and the row is promoted in patch mode:

- patch `docs/bugs/fix-patches/SETTLEMENT-BLUEPRINT-LIST-SHOWS-RAW-SPANISH-IDS.patch`, sha256
  `06a9a6d513a2af87da34363f8ffd52f53b42a7dc857af6a1c9b39b459f04d907` (`fix_commit`).
- the a/-side is the real pre-image: `0c20eb8b2^:crates/mc-core/src/settlements/ui.rs`'s
  `fn blueprint_label<'a>(ctx: &Ctx, blueprint: &'a str) -> &'a str` with its 52-arm match, verbatim.
- the b/-side is HEAD's three-line delegation at `crates/mc-core/src/settlements/ui.rs:2348`.
- `git apply -R --check` passes on the candidate; reversing restores the old match and leaves
  `ui_text::blueprint` in place but unreachable.

## Omitted hunks of 0c20eb8b2

| file | hunk | reason |
|---|---|---|
| `crates/mc-core/src/settlements/ui_tests.rs` | the two name regressions and the migrated raw-id expectation | test hunks (kept on both sides; they are the row's refs) |
| `crates/mc-core/src/settlements/ui_text.rs` | the new `blueprint` table | created by the fix, so it has no pre-image line; the control keeps it (only the call site is reversed). A later commit (`a895cc853b`, the furnace name) also edits it and stays untouched: the table's furnace entry is SETTLEMENT-PROJECT-BLUEPRINT-WITHOUT-NAME's row, and the control makes the whole table unreachable, so that row's symptom shows there too. It is not this row's assertion. |
| `docs/settlements/menus.md` | one line | documentation |

## Gate

`harness:settlement-map-qa:blueprints` (native, `--scenario blueprints`, quickwins setup founds the
settlement) publishes `blueprints`: every Construction list page is read and the names must carry no
raw id (`snake_case` or `minecraft:`), no Unknown/Неизвестный, only the client language's letters, and
at least 26 distinct blueprints (`settlement_quickwins_stage2.blueprint_names`).

- candidate (`afee473c6`, server pin `afee473c6f8e-739df6a9-server`, receipt
  `.analysis/validation/20261124T213234-settlement-map-qa-fsofbifx`): PASSED, `blueprints: true` —
  26 distinct names, no raw id, every name in the client language (`en_us` for the native client).
- control: the promotion's own run on the same tree with the old label match (`ui_text::blueprint`
  unreachable), where the list shows `spanish_house L1` and the other raw ids.
