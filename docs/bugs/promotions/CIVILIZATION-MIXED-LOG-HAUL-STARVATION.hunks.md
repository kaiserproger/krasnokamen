# CIVILIZATION-MIXED-LOG-HAUL-STARVATION — HEAD re-break control patch

Patch: `docs/bugs/promotions/CIVILIZATION-MIXED-LOG-HAUL-STARVATION.patch` (`patch:2168d386850015763cd26d6d6e314217a93a8eca5702d4e12da5d29d2a876030`), the row's
`fix_commit`. It reverses on the committed tree `6ef75a905` (`git apply -R
--check`, plain context) and is what a promotion passes as `--fix-patch`.

## What the fix is (origin `08939e773`)

Files: crates/mc-core/src/villager/lumber.rs + mining.rs.

the shared load predicate: `lumber::work` parks on `mining::heavy_load` (a whole stack of any wood) instead of `needs_deposit` (one species filling 64); `mining::heavy_load` is the new predicate.

## HEAD equivalents and omissions

Original commit's non-test diff is exactly these two files; all 4 hunks are included. Test files stay in the control.

## Reverse-apply evidence

`git apply -R --check docs/bugs/promotions/CIVILIZATION-MIXED-LOG-HAUL-STARVATION.patch` on `6ef75a905`
succeeds (clean tree); the real-client red is the later integrated sweep's
control run, not claimed here.
