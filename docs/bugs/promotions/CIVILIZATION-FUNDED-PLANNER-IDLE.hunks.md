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

## a/-side audit (B5, 2026-11-24)

`git show 384e83f69^` vs the file after `git apply -R` on HEAD, per hunk:

- `construction.rs` `plan_pass`: the restored `return false;` in the priority
  worker's arm is the pre-image early return, placed in HEAD's newer arm; the
  meadow scan stays on both sides.
- `construction_builders.rs` `protected_gatherer`: the origin commit changed
  `construction.rs`; HEAD's function is a later refactor (`10ed5b28c`) of the same
  fix. Its a/-side rebuilds the pre-fix body. Two a/-side names are outside the
  origin's text and change no behaviour, and both are named here: the parameter is
  `_ordered: bool` because the pre-fix body never read it (no unused-variable
  warning), and the guard is `let _record = state.settlements.get(id)?;` — HEAD
  binds `record` for the fix's `gatherer_demand(state, record, &images)` call;
  with that call gone the binding is unused, so it keeps its `?` (still fails
  closed on a missing settlement) and is renamed. The origin's text has no
  `_record` binding at all. Nothing else differs.
