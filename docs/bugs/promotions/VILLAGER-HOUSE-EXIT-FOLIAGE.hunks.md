# VILLAGER-HOUSE-EXIT-FOLIAGE — HEAD re-break control patch

Patch: `docs/bugs/promotions/VILLAGER-HOUSE-EXIT-FOLIAGE.patch` (`patch:6d225aa51498dbf4aebf199d2b77e0fb5a5d78a121536afd85e394ebf835692a`), the row's
`fix_commit`. It reverses on the committed tree `6ef75a905` (`git apply -R
--check`, plain context) and is what a promotion passes as `--fix-patch`.

## What the fix is (origin `4e6874ded`)

Files: crates/mc-worldgen/src/terrain/trees.rs.

Same guard as CIVILIZATION-GENERATED-DOOR-TREE from the villager side: doorway headroom is reserved through the village plans so door-exit foliage cannot trap residents.

## HEAD equivalents and omissions

Identical patch to CIVILIZATION-GENERATED-DOOR-TREE (one row's guard closes both findings); same omissions and reasons.

## Reverse-apply evidence

`git apply -R --check docs/bugs/promotions/VILLAGER-HOUSE-EXIT-FOLIAGE.patch` on `6ef75a905`
succeeds (clean tree); the real-client red is the later integrated sweep's
control run, not claimed here.
