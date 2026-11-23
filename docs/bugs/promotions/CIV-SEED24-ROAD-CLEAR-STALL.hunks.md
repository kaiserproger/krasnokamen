# CIV-SEED24-ROAD-CLEAR-STALL — HEAD re-break control patch

Patch: `docs/bugs/promotions/CIV-SEED24-ROAD-CLEAR-STALL.patch` (`patch:a3360a2db04ed9c2ef5d9219901abf926e00ab85dcecdd6398d2230a19f90fd1`), the row's
`fix_commit`. It reverses on the committed tree `6ef75a905` (`git apply -R
--check`, plain context) and is what a promotion passes as `--fix-patch`.

## What the fix is (origin `2b8219d1`)

Files: crates/mc-core/src/villager/building.rs.

`preparation_stand` requires a cell the navigator can end a route on (`allocator::standing`) instead of the collision-shape `clear` test that counted a 1.5-high fence top as ground.

## HEAD equivalents and omissions

HEAD carries a later `lava_safe` term (D10, another row) which the re-break keeps; only `stands`/`dry` are reverted to `clear`.

## Reverse-apply evidence

`git apply -R --check docs/bugs/promotions/CIV-SEED24-ROAD-CLEAR-STALL.patch` on `6ef75a905`
succeeds (clean tree); the real-client red is the later integrated sweep's
control run, not claimed here.
