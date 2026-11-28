verdict: changes

# SPAININTEG-F1-TERRAIN-REACH review (committee-sol, 2026-11-28)

Scope: worktree `wt/SpainD7Re` head `187775ced`, delta `89fdcc7f5..187775ced`
(behaviour commits `ecdcb2269`, `de28aabb1`, `cc3c9b697` plus receipts/docs).
Reviewer saw code, saved patches, control commands and their logs; no test run,
no write tool (Main recorded this file).

## (a) Custody — PASS

Fill materialises as one paid `Fill` per cell; the authored floor is not paid
twice. The ramp reorders the same cells instead of adding free blocks
(`construction_preparation.rs:111-135,196-236`). `fetch` moves real stock into
carry; spending happens in `AdvanceStructure` before the cell write and the
cursor advance. Completion returns the remaining carry instead of crediting the
slice again (`settlement_builds.rs:1723-1731`,
`plugin_structures.rs:1222-1268,1298-1308`, `construction.rs:2839-2844`).

A cut credits a live block's drop in the same transaction. Storage first; an
admissible ground remainder stays in real carry, and a cell without room is not
cut. This conserves items, it does not guarantee the drop lands in the chest at
once (`settlement_builds.rs:1028-1081,1152-1173,1203-1212`;
`docs/settlements/construction.md:78-84`).

## (b) Refusals — PASS in code, funding gap in the control

Support under the builder, the falling block and late lava are checked before
any inventory change. Shortage of material or room stops the operation before
`inventory::commit`; without a successful debit neither the block write nor the
cursor advance happens (`settlement_builds.rs:975-1020,1100-1124,1152-1173,1211`;
`plugin_structures.rs:1225-1268`). The late-lava test must be funded: on the
control tree it gets `InsufficientItems` instead of making the wrong write, so it
separates two refusals but does not prove lava stops a **paid** placement
(`construction_terrain_tests.rs:2095-2124`; `receipts/SpainD7Re/control-b3.log:223-226`).

## (c) Territory and height — PASS; one admission rule — FAIL

A deep fill checks the bottom and top cell of the column in a 3D sphere; the cut
checks every cell, the ramp its steps. The old small-preparation exception is not
widened. The working box stays within 256 rows, the executor checks every cell
against the box, the DTO checks `reserved_footprint` height apart from X/Z
(`construction_site_refusals.rs:1346-1354,1412-1435`;
`construction_terrain.rs:856-866`; `construction_preparation.rs:431-449`;
`settlement_builds.rs:92-111`; `world_structure_operations.rs:225-235,1196-1199`).

**[P1] Admission checks only the deep subset while the builder applies
`fill-from-above` to the whole stage.** The plan checks fill only down to
`floor - SITE_BAND`; materialisation carries it to `floor` in cells without an
authored floor. One deep cell switches the whole stage to the
`target_y < feet_y` rule. Static counterexample: a lot with a deep pocket and the
rest of the ground no higher than `floor - 1`; the nearest stands have feet at
`floor`. The deep part is admitted, the open cells at `floor` are served neither
from those stands nor from columns already filled to `floor - 1`. The shipped
farm has such gaps; `Build` runs after `Fill`. Needs a test to completion and one
cell set/rule shared by admission and execution
(`construction_terrain.rs:123-126,779-789,829-835`;
`construction_preparation.rs:115-134`; `building.rs:425-443,512-527`;
`portions.rs:5-16`; `assets/blueprints/farm.json:16-29`).

## (d) Controls and budget — FAIL

- **B1 — PASS for the shown admission.** Four source files taken from `HEAD^`,
  tests kept. The log shows `SurfaceUnavailable` instead of admitting the
  12-block fill and four orders without a site: the row's symptoms, but not a
  separate proof of movement or debit (`SpainD7Re.jsonl:327-329`;
  `receipts/SpainD7Re/control-b1.log:197-200,218-281`).
- **B2 — FAIL.** The command inserts `return None` for **all** lots with
  non-empty terrain work. That is not the behaviour of `ecdcb2269`: the control
  breaks even the 12-block admission from B1. Ceiling, lava and reopen go red at
  the preliminary admission, not at their own checks. The narrow runs did print
  `None` instead of `TerrainNoAccess` and `260` instead of `0`, but on
  hand-modified branches, not on a Git pre-image (`SpainD7Re.jsonl:427,432,452,457`;
  `receipts/SpainD7Re/control-b2.log:192-193,233-254,272-274,297-324`).
- **B3 — partial proof, not a row control.** The real veto disables produced
  `Ok(false)` instead of `Blocked` on both slices. The budget test breaks when the
  `spent` record is deleted, not when the budget debit is disabled; the memo test
  only calls `Registry::terrain_stage`. A separate red `serves` is confirmed, but
  the command record shows the other mutations were not restored before that run
  (`SpainD7Re.jsonl:523-526,535,543`; `receipts/SpainD7Re/control-b3.log:202-205,244-247,286-287,305-308`;
  `construction_terrain_tests.rs:1941-2002`).
- **Accounting does not bound real work.** `covered_by` scans every uncovered
  cell per search step and `arrange` per path step, while the counter bills the
  step, not those passes. Winner re-check and materialisation call the plan again
  outside the debit in the main loop
  (`construction_terrain.rs:472-489,553-569,605-622`;
  `construction_site_refusals.rs:394-419,522-533,598-611`;
  `construction_preparation.rs:196-214`). Needs a control of actual shared-budget
  spend that includes the expensive refusal and the winner re-pass.

## (e) Persistence/WAL — PASS for the transaction, FAIL for docs and memo

No new terrain format: cell order, certificates, `cut_credits` and the cursor go
into the existing record; memo is memory-only. The reopen test checks the block,
the storage and the next cursor after replay (`settlement_builds.rs:176-193`;
`plugin_structures/ledger.rs:123-153`; `registry.rs:148-154`;
`construction_terrain_tests.rs:1694-1767`). The WAL writes the whole changed
record per step; a long manifest grows the WAL without growing the key count
(`plugin_structures/ledger.rs:125-137`).

The document mixes the working `settlement_preparation` with the snapshot
`reserved_footprint`: a tall hill grows the former while the snapshot ends at the
authored roof. The old 64 limit applied to the snapshot; the 70-row hill test
checks the working box, not an old-decoder refusal. Fix the claim and check a
deep-fill snapshot above 64 separately
(`docs/architecture/persistence.md:266-269`; `construction.rs:1424-1436`;
`plugin_structures.rs:882-886,1619-1629`;
`construction_terrain_tests.rs:1809-1846`; `de28aabb1`).

Memo is keyed by index and stage length, but recovery can replace `repair` with
another set of the same size at the same index. One deep cell instead of an
ordinary one changes the rule while the cache keeps the old answer, which is
recomputed after restart. Needs invalidation on manifest replacement or a
classification that does not depend on such a cache (`registry.rs:395-416`;
`settlement_builds.rs:449-476,708-727,1380-1404`).

## Before merge

1. Remove the admission/execution divergence (P1).
2. Tie the memo to the stage contents.
3. Prove the debit of real work, and replace the synthetic B2 with a real
   fix-only control.
4. The row has `repro = "client"` and an empty `regression`; promote needs a
   real-client ref and a receipt (`tools/harness/ledger.py:2945-2958`).

VOTE SPAININTEG-F1-TERRAIN-REACH: amend — custody is kept, but admission does
not guarantee the fill completes, and B2 does not reproduce the pre-fix
behaviour.
