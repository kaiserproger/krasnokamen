# CIVILIZATION-FUNDED-PLANNER-IDLE — HEAD re-break control patch

Patch: `docs/bugs/promotions/CIVILIZATION-FUNDED-PLANNER-IDLE.patch` (`patch:5549665bfbce8aaa422c34ea6585b88995d25ca79381c111bc9fd7a9a169dc54`), the row's
`fix_commit`. It reverses on the committed tree `6ef75a905` (`git apply -R
--check`, plain context) and is what a promotion passes as `--fix-patch`.

## What the fix is (origin `384e83f692d4`)

Files: crates/mc-core/src/settlements/construction.rs + construction_builders.rs.

The planner no longer returns early for the whole pass when the priority material worker is staffed; the hold applies only after funded candidates and the meadow scan, and `assign_builder` protects a staffed autonomous gatherer through `protected_gatherer`.

## HEAD equivalents and omissions

HEAD moved `assign_builder` into `construction_builders.rs` and generalized the protection into `protected_gatherer(state, id, ordered)` (a later refactor of the same fix). The HEAD equivalent of the pre-fix form is `needed_gatherer(joint_active_bill)` only, without the autonomous `gatherer_demand` branch. The `ordered` argument stays for signatures and is unused when reversed.

## Reverse-apply evidence

`git apply -R --check docs/bugs/promotions/CIVILIZATION-FUNDED-PLANNER-IDLE.patch` on `6ef75a905`
succeeds (clean tree); the real-client red is the later integrated sweep's
control run, not claimed here.
