# KELP-ITEMS-SPAWN-ON-OCEAN-FLOOR — HEAD re-break control patch

Patch: `docs/bugs/promotions/KELP-ITEMS-SPAWN-ON-OCEAN-FLOOR.patch` (`patch:e6fee19a268eeee6a24d95c816b746f6b4209fa70c4e305b8bf85ba368289420`), the row's
`fix_commit`. It reverses on the committed tree `6ef75a905` (`git apply -R
--check`, plain context) and is what a promotion passes as `--fix-patch`.

## What the fix is (origin `af58aeb92`)

Files: crates/mc-core/src/fluids.rs.

`enter` decides `Flow::Wash` from the `washed_away_by_fluids` tag table (which excludes kelp/seagrass/waterloggable states) instead of the collision-shape test that washed every non-colliding plant.

## HEAD equivalents and omissions

HEAD carries later lava-bucket and spreading branches (other rows) which the re-break keeps. The `registry.rs` table hunks are **omitted (b)**: the dense table has no other reader, the control keeps it inert, and reversing the field would stop `enter` from compiling while the row's red assertion needs only the flow decision.

## Reverse-apply evidence

`git apply -R --check docs/bugs/promotions/KELP-ITEMS-SPAWN-ON-OCEAN-FLOOR.patch` on `6ef75a905`
succeeds (clean tree); the real-client red is the later integrated sweep's
control run, not claimed here.
