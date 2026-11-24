# SETTLEMENT-BUILDER-PROJECT-STALLS-AFTER-PLACED-BLOCK-BROKEN — HEAD re-break control patch

Patch: `docs/bugs/promotions/SETTLEMENT-BUILDER-PROJECT-STALLS-AFTER-PLACED-BLOCK-BROKEN.patch`
(`patch:7df366247147e68b…`), the row's `fix_commit`. It reverses on the clean
committed tree at `b67e1b0612` (`git apply -R --check`, plain context) and is
what a promotion passes as `--fix-patch`.

## What the fix is (origin `patch:7212599e…`, kept whole in `promotions/SETTLEMENT-BUILDER-PROJECT-STALLS-AFTER-PLACED-BLOCK-BROKEN.origin.patch`, HEAD form of the breaker fix)

A player breaking an already-placed cell of an active project left a hole that
was never re-placed: the project stayed active at 250/250 and held the only
slot. The fix queues an emptied placed cell into the `repair` stage:

1. `settlement_builds::recover` scans the finished cells of `liquid`/`fill`/
   `build` stages and appends the emptied ones as a newly certified `repair`
   stage (a solid foreign block is left alone);
2. `finish_project` runs that scan for a committed structure
   (`reopen_emptied`) and keeps the project open when it queued a repair;
3. `finish_project` first calls `executor::recover` so a paused structure is
   resumed before completion is judged.

## HEAD equivalents

All three effects exist at HEAD in the current ledger API and are reversed:

- `settlement_builds.rs` hunk 1: `let committed = ...` / `if !paused && !committed`
  back to the pre-fix `if !paused`;
- `settlement_builds.rs` hunks 2-3: the repair scan and the `repair` stage push
  removed again, so no emptied cell is ever queued;
- `construction.rs` hunks 4-5: the `executor::recover` guard and the
  `executor::reopen_emptied` guard removed from `finish_project`.

## Omissions

- The doc-comment lines of the original `settlement_builds.rs` hunk (the
  paragraph about queued repairs) — **(c)**: a later commit rewrote that prose
  ("Completed fill/build damage is appended as a newly certified repair
  portion…"), so the pre-fix sentence has no HEAD equivalent.

The control keeps the later `retry_preparation` and liquid `revalidate` paths
of `recover` for a *paused* structure: they belong to other rows (owner-order
re-siting, N3 sapling cancel, liquid proofs) and reversing them would turn
their assertions red — **(a)**. With the early-out restored they are
unreachable for a committed structure, which is the pre-fix state.

## Reverse-apply evidence

`git apply -R --check docs/bugs/promotions/SETTLEMENT-BUILDER-PROJECT-STALLS-AFTER-PLACED-BLOCK-BROKEN.patch`
on the committed tree `b67e1b0612` succeeds.

Control run (reversed tree, then restored): `python3 -m tools.harness run test
-p mc-core --lib settlements::construction::fencing_tests::a_placed_cell_broken_to_air_is_replaced`
→ `.analysis/validation/20261123T200456-test-5yvracl_` (`status: failed`,
`dirty` tree at head `dd94645dda`). The failure is the row's own stall: the
helper at `crates/mc-core/src/settlements/construction_fencing_tests.rs:346`
panics "construction did not finish" with the status
`house · level 1 · 100% / Blocks: 249/249 · Committed` and materials still
outstanding — the broken cell is never re-placed and the project stays open at
249/249. (A first attempt that reversed only `reopen_emptied`, leaving the
`recover` scan, stayed green: `.analysis/validation/20261123T200022-test-qj1pdsyz` —
that is why the scan is part of this control.) The graphical
`harness:buildings-qa:house.broken_cell_replaced` red is the integrated
sweep's control run, not claimed here.

## a/-side audit (B5, 2026-11-24)

Mechanical check: `git show <origin>^:<file>` vs the file after `git apply -R`
of this patch at the audit head; script and full output in
`receipts/D123/D123CivGates/a-side-audit.md`. Every line outside the origin
commit's own change, with the commit that last changed it and whether the
line is in `origin^:file`:

- `crates/mc-core/src/plugin_structures/settlement_builds.rs` `@@ -331,7 +331,8 @@ pub(crate) fn recover(` (adapted; origin `95dc2a9bd`, discovered origin):
  - a/-side `if !paused {` — no commit; in origin^: no
- `crates/mc-core/src/plugin_structures/settlement_builds.rs` `@@ -383,6 +384,51 @@ pub(crate) fn recover(` (over-reverted; origin `76a6239b4`, discovered origin):
  - HEAD `let mut repair = Vec::new();` — 95dc2a9bd fix(settlements): requeue broken builder project cells; in origin^: yes
  - HEAD `let mut seen = BTreeSet::new();` — 04b3ce757 feat(settlements): persist ordered preparation portion evidence; in origin^: yes
  - HEAD `for (index, stage) in record.snapshot.stages.iter().enumerate().rev() {` — 04b3ce757 feat(settlements): persist ordered preparation portion evidence; in origin^: no
  - HEAD `if !matches!(stage.stage.as_str(), "liquid" | "fill" | "build") {` — d275999bf Preserve liquid and free-ground invariants across portion execution; in origin^: no
  - HEAD `continue;` — 68a9ae1ff fix(settlements): bootstrap saturated storage with builder custody; in origin^: yes
  - HEAD `}` — 68a9ae1ff fix(settlements): bootstrap saturated storage with builder custody; in origin^: yes
  - HEAD `let current = record.snapshot.current_stage_index as usize;` — 95dc2a9bd fix(settlements): requeue broken builder project cells; in origin^: yes
  - HEAD `let completed = if current > index {` — 95dc2a9bd fix(settlements): requeue broken builder project cells; in origin^: yes
  - HEAD `record.cells[index].len()` — 95dc2a9bd fix(settlements): requeue broken builder project cells; in origin^: yes
  - HEAD `} else if current == index {` — 95dc2a9bd fix(settlements): requeue broken builder project cells; in origin^: yes
  - HEAD `record.snapshot.completed_work_units as usize` — 6d6cff536 settlements: a builder's haul carries the next stages' materials too; in origin^: yes
  - HEAD `} else {` — 6d6cff536 settlements: a builder's haul carries the next stages' materials too; in origin^: yes
  - HEAD `0` — 6d6cff536 settlements: a builder's haul carries the next stages' materials too; in origin^: yes
  - HEAD `};` — 68a9ae1ff fix(settlements): bootstrap saturated storage with builder custody; in origin^: yes
  - HEAD `for cell in record.cells[index].iter().take(completed).rev() {` — 04b3ce757 feat(settlements): persist ordered preparation portion evidence; in origin^: no
  - HEAD `if !seen.insert(cell.at) || free_soil_work(target, cell) {` — 1c59d1a06 test: reconcile preparation proofs with custody and record storage; in origin^: no
  - HEAD `continue;` — 68a9ae1ff fix(settlements): bootstrap saturated storage with builder custody; in origin^: yes
  - HEAD `}` — 68a9ae1ff fix(settlements): bootstrap saturated storage with builder custody; in origin^: yes
  - HEAD `.ok_or(Failure::Unloaded)?;` — 74bfb11de settlements: outside attribution and a dry cut before every write (custody review P1-1, P1-2); in origin^: yes
  - HEAD `.ok_or(Failure::InvalidRequest)?;` — 95dc2a9bd fix(settlements): requeue broken builder project cells; in origin^: yes
  - HEAD `.ok_or(Failure::InvalidRequest)?;` — 95dc2a9bd fix(settlements): requeue broken builder project cells; in origin^: yes
  - HEAD `repair.push(cell.clone());` — 95dc2a9bd fix(settlements): requeue broken builder project cells; in origin^: yes
  - HEAD `}` — 68a9ae1ff fix(settlements): bootstrap saturated storage with builder custody; in origin^: yes
  - HEAD `}` — 68a9ae1ff fix(settlements): bootstrap saturated storage with builder custody; in origin^: yes
  - HEAD `}` — 68a9ae1ff fix(settlements): bootstrap saturated storage with builder custody; in origin^: yes
  - HEAD `repair.reverse();` — 04b3ce757 feat(settlements): persist ordered preparation portion evidence; in origin^: no
  - HEAD `if committed && repair.is_empty() {` — 95dc2a9bd fix(settlements): requeue broken builder project cells; in origin^: yes
  - HEAD `return Ok(false);` — 95dc2a9bd fix(settlements): requeue broken builder project cells; in origin^: yes
  - HEAD `}` — 68a9ae1ff fix(settlements): bootstrap saturated storage with builder custody; in origin^: yes
- `crates/mc-core/src/plugin_structures/settlement_builds.rs` `@@ -411,6 +457,24 @@ pub(crate) fn recover(` (adapted; origin `95dc2a9bd`, discovered origin):
  - HEAD `"repair".into(),` — 95dc2a9bd fix(settlements): requeue broken builder project cells; in origin^: no
  - HEAD `repair.len() as u32,` — 76a6239b4 fix(settlements): structure recovery re-places only emptied cells; in origin^: no
  - HEAD `repair.len() as u64,` — 76a6239b4 fix(settlements): structure recovery re-places only emptied cells; in origin^: no
  - HEAD `materials(&counts),` — 76a6239b4 fix(settlements): structure recovery re-places only emptied cells; in origin^: no
  - HEAD `record` — 6d6cff536 settlements: a builder's haul carries the next stages' materials too; in origin^: no
  - HEAD `.portions` — 04b3ce757 feat(settlements): persist ordered preparation portion evidence; in origin^: no
  - HEAD `.extend(portions::certify(state, std::slice::from_ref(&repair))?);` — 04b3ce757 feat(settlements): persist ordered preparation portion evidence; in origin^: no

HEAD-side lines whose last change is another commit are that later commit's
form: reversing them is the HEAD drift a control must reverse to reach the
pre-fix decision (rule (c)), not part of the fix. An a/-side line that is not
in `origin^` is an adaptation this control relies on; each such line is named
in the list above and changes no behaviour beyond the pre-fix decision.
