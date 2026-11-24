# CIVILIZATION-PREMATURE-LOG-DELIVERY — forward-ported fix patch

Forward patch: `docs/bugs/promotions/CIVILIZATION-PREMATURE-LOG-DELIVERY.patch`
(`patch:29f14df11329b16959cc52fa841f5c67385a5c8b9d8639cc5d26d09eb2d6555a`), applies
in reverse on the candidate `wt/D123Civ` at
`f9a8d45596696880fc6a0c9d92f5cd1758002651` (`git apply -R --check`).

## Why patch mode

Commit `a946fa286518018d1c4291d97d833db940550348` (2026-11-07) no longer
reverses on the candidate: `candidate_funded_by_delivery` was rewritten by later
commits (`joint_active_bill`, `access::projects`), so the original hunk's
pre-image does not exist. A control worktree at that commit cannot build the
registered ref either (the planner test module moved to
`construction::planner_tests`).

## Original commit hunks → HEAD equivalents

| original hunk | HEAD equivalent | disposition |
|---|---|---|
| `construction.rs` `candidate_funded_by_delivery`: factor `funded(images)`, probe it before appending the carry and return `false` when the candidate is already funded; the tail calls `funded(&images)` | same code at `construction.rs:805-825`, using the current `joint_active_bill`/`access::projects` names and `can_fund(state, bill, images)` | **included** |
| `construction_planner_tests.rs`: `an_already_funded_candidate_does_not_send_each_harvested_log_home` (29 lines) | present | kept (test) |
| `docs/bugs/ledger.toml` | moved to `docs/bugs/rows/` | kept (docs) |

No a/-side adaptation was needed: the pre-image is the same function with the
`funded` closure removed and the tail restored to the inline
`access::projects(...)` branch.

## Proving ref

`cargo:mc-core:settlements::construction::planner_tests::an_already_funded_candidate_does_not_send_each_harvested_log_home`
— with the early funded probe reversed, each harvested log is sent home despite
an already funded candidate, which the test's delivery count rejects.

## a/-side audit (B5, 2026-11-24)

Mechanical check: `git show <origin>^:<file>` vs the file after `git apply -R`
of this patch at the audit head; script and full output in
`receipts/D123/D123CivGates/a-side-audit.md`. Every line outside the origin
commit's own change, with the commit that last changed it and whether the
line is in `origin^:file`:

- `crates/mc-core/src/settlements/construction.rs` `@@ -803,16 +803,24 @@` (adapted; origin `a946fa286`, discovered origin):
  - HEAD `if access::projects(record).next().is_some() {` — ab417a937 refactor(settlements)!: store construction projects and orders as vectors; in origin^: no
  - a/-side `if access::projects(record).next().is_some() {` — 57c150f44 settlements: several funded projects build in parallel (D8 S2); in origin^: no
  - a/-side `active_bill` — a946fa286 fix(settlements): keep harvesting when construction is already funded; in origin^: no
  - a/-side `.as_ref()` — 24de2c2ef settlements: D8 review round 2 (SOL-2..6, incumbent liveness, per-project lookups); in origin^: yes
  - a/-side `.is_some_and(|bill| super::construction_crafting::can_fund(state, bill, &images))` — no commit; in origin^: no

HEAD-side lines whose last change is another commit are that later commit's
form: reversing them is the HEAD drift a control must reverse to reach the
pre-fix decision (rule (c)), not part of the fix. An a/-side line that is not
in `origin^` is an adaptation this control relies on; each such line is named
in the list above and changes no behaviour beyond the pre-fix decision.
