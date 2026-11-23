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
