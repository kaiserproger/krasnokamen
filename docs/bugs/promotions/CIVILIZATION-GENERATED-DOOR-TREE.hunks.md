# CIVILIZATION-GENERATED-DOOR-TREE — HEAD re-break control patch

Patch: `docs/bugs/promotions/CIVILIZATION-GENERATED-DOOR-TREE.patch` (`patch:6d225aa51498dbf4aebf199d2b77e0fb5a5d78a121536afd85e394ebf835692a`), the row's
`fix_commit`. It reverses on the committed tree `6ef75a905` (`git apply -R
--check`, plain context) and is what a promotion passes as `--fix-patch`.

## What the fix is (origin `4e6874ded`)

Files: crates/mc-worldgen/src/terrain/trees.rs.

Tree placement rejects a trunk or leaf cell that lies in a village plan's `entrance_headroom`, so generated foliage never blocks a village doorway or its approach.

## HEAD equivalents and omissions

HEAD keeps `terrain.rs`'s `village` argument and `plan_source.rs`'s `entrance_headroom` (later plumbing). Reversing the two leaf guards alone restores the obstruction; the plumbing hunks are **omitted (b)** (API those files keep; the helper has no other caller) and are not behavioural by themselves.

## Reverse-apply evidence

`git apply -R --check docs/bugs/promotions/CIVILIZATION-GENERATED-DOOR-TREE.patch` on `6ef75a905`
succeeds (clean tree); the real-client red is the later integrated sweep's
control run, not claimed here.
