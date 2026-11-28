# SETTLEMENT-FURNACE-SITE-ON-ANOTHER-STRUCTURES-ROOF — control on bbf5b71e5

The row's fix is two commits of the same defect class:

- `9026719a07` "keep a furnace site at its store's level" — the planner clamp
  `(y - chest[1]).abs() > 2`.
- `bbfe2e0b9` "the furnace stand stays at the furnace's level" (review wave2-9,
  "same class as 9026719a07") — the tender's `stand` clamp. Its hunk was later
  rewritten by `e043f78ca` into the banded
  `surface_y_between(next[1]-3..=next[1]+1)` search plus the `.filter(|y|
  (y-next[1]).abs() <= 2)` fallback.

Reversing `9026719a07`'s hunk alone on `bbf5b71e5` stays GREEN: the plan's
`stand(state, &project, at, None)` admission now rejects the platform under the
stand's own level rule, so the planner guard is masked and the fixture never
reaches `[3,206,1]`. That is why the fix identity is a patch, not the single
commit (D131): `patch:822579e1e302205de429cb44cb37b2f93b1c8c9624f75df67bfdd964c84eb88c`
(`docs/bugs/fix-patches/SETTLEMENT-FURNACE-SITE-ON-ANOTHER-STRUCTURES-ROOF.patch`,
2 hunks, one production file) carries the HEAD equivalents of both:

1. planner: remove the level clamp (12 lines);
2. stand: restore the pre-image `let y = surface_y(state, record.anchor[1], x, z)?;`
   in place of the banded-search + filter.

`git apply -R --check` passes at `bbf5b71e5`. The reversed tree is the real
pre-fix behavior for both symptoms the test asserts (site on the well roof, stand
on a neighbour's roof).

- RED: `.analysis/validation/20261128T090043-test-6ydslkb4` FAIL, exit 100 — the
  row's own `a_furnace_site_stays_at_the_store_level_not_on_a_platform_above_it`
  panics at `construction_planner_tests.rs:555`,
  `the furnace must not stand on the platform above the store`
  (`assert_ne!(project.origin, [3, 206, 1])`).
- GREEN: `.analysis/validation/20261128T092429-test-07wzg06v` PASS on the clean
  candidate.
