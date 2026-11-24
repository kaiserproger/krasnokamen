# VILLAGER-LAND-STROLL-WET-WELL — forward-ported fix patch

Forward patch: `docs/bugs/promotions/VILLAGER-LAND-STROLL-WET-WELL.patch`
(`patch:7570e3e6bff1b416b89184b32253a9bb8b8fb4e8709d7193dd89601e409cfa3a`). It
applies in reverse on the candidate `wt/D123Civ` at
`f9a8d45596696880fc6a0c9d92f5cd1758002651` (`git apply -R --check`).

## Why patch mode

The row's path was closed by two commits, neither of which reverses on the
candidate:

- `7c691bb50549363b9044f7930eb2314aae9576ca` "fix(villagers): keep short land
  strolls out of water wells" — `brain.rs` was reformatted by
  `b4f9aa2d6` (formatting only), so the original hunk no longer applies.
- `b4f9aa2d6cbfa1830567bced64963a06b4a64b57` "fix(settlements): prefer dry work
  routes around village wells" — the allocator hunk still applies but the commit
  bundles unrelated founding/registry work that belongs to other rows.

## Commit hunks → HEAD equivalents

| commit | original hunk | HEAD equivalent | disposition |
|---|---|---|---|
| 7c691bb50 | `villager/brain.rs`: the `foot` water rejection in `wander_to` (13 lines) | same code at `brain.rs:694-704`, reformatted by b4f9aa2d6 | **included** |
| 7c691bb50 | `villager/stroll_swim_tests.rs`: 39 test lines | present | kept (test) |
| 7c691bb50 | `docs/bugs/ledger.toml` | moved on | kept (docs) |
| b4f9aa2d6 | `settlements/allocator.rs`: the vanilla WATER malus 80 in `find_path_ctx` (14 lines) and the `cost` addition | same code at `allocator.rs:1116-1130` | **included** (the work-route half of this row) |
| b4f9aa2d6 | `villager/brain.rs`: formatting of the `foot` expression | rewritten by the same commit's later reformat; the expression at HEAD is the reformatted equivalent of the included hunk | **included in effect** — no separate hunk: the included `brain.rs` hunk at HEAD already carries the formatting |
| b4f9aa2d6 | `settlements/autonomy_tests.rs`, `construction_planner_tests.rs`, `villager/stroll_swim_tests.rs` | at HEAD | kept (test) |
| b4f9aa2d6 | `settlements/registry.rs`, `settlements/construction_founding*.rs` (production files) | at HEAD | **omitted, reason (a)**: their behaviour belongs to other rows (registry/founding culture work); reversing it would turn the control red on another row's assertion |
| b4f9aa2d6 | `docs/architecture/settlements.md` | at HEAD | kept (docs) |

No a/-side adaptation was needed.

## Proving refs

- `cargo:mc-core:villager::brain::tests::stroll_swim_tests::short_stroll_rejects_a_roofed_well_but_keeps_a_dry_neighbor`
  — reversing the `brain.rs` hunk accepts the wet-foot stroll node again.
- `cargo:mc-core:villager::brain::tests::stroll_swim_tests::work_route_avoids_roofed_well_and_reaches_the_opposite_rim`
  — reversing the `allocator.rs` hunk makes the work route cross the well's water.

## a/-side audit (B5, 2026-11-24)

Mechanical check: `git show <origin>^:<file>` vs the file after `git apply -R`
of this patch at the audit head; script and full output in
`receipts/D123/D123CivGates/a-side-audit.md`. Every line outside the origin
commit's own change, with the commit that last changed it and whether the
line is in `origin^:file`:

- `crates/mc-core/src/villager/brain.rs` `@@ -688,8 +688,19 @@` (adapted; origin `7c691bb50`, discovered origin):
  - HEAD `let foot = state` — 7c691bb50 fix(villagers): keep short land strolls out of water wells; in origin^: no
  - HEAD `.world` — b4f9aa2d6 fix(settlements): prefer dry work routes around village wells; in origin^: yes
  - HEAD `.block_at(next.x, end[1].floor() as i32, next.z)?;` — b4f9aa2d6 fix(settlements): prefer dry work routes around village wells; in origin^: no

HEAD-side lines whose last change is another commit are that later commit's
form: reversing them is the HEAD drift a control must reverse to reach the
pre-fix decision (rule (c)), not part of the fix. An a/-side line that is not
in `origin^` is an adaptation this control relies on; each such line is named
in the list above and changes no behaviour beyond the pre-fix decision.
