# AUTOCRAFT-DEAD-WORKER — forward-ported fix patch

Forward patch: `docs/bugs/promotions/AUTOCRAFT-DEAD-WORKER.patch`
(`patch:d1a7ea797bb7d013ad233a911d526ea9ba59ae91298361ee702411c8a39bb773`), applies
in reverse on the candidate `wt/D123Civ` at
`f9a8d45596696880fc6a0c9d92f5cd1758002651` (`git apply -R --check`).

## Why patch mode

Commit `b620e3333d0d00f8315786e692d8a3ffd6db20e0` no longer reverses on the
candidate: `production.rs` was reshaped by later offscreen/tick-control commits
(`tick_control`, `offscreen::advance`, `consumed`), so the original hunks'
pre-image does not exist. A control worktree at that commit cannot run the
registered refs (they live under `settlements::autocraft::tests`, which the
commit itself introduced; the module registration came later).

## Original commit hunks → HEAD equivalents

| original hunk | HEAD equivalent | disposition |
|---|---|---|
| `production.rs`: `MISSING_WORKER` const and its doc line | `production.rs:29-30` | **included** (removed by the patch's a-side) |
| `production.rs` `Request::Resume`: re-take a dead worker's order instead of returning its failure | `production.rs:1305-1311` | **included** |
| `production.rs` `tick`: collect `Failure::NotFound` workers (`gone`) and park with `MISSING_WORKER` | `production.rs:1492-1501`; the park branch at HEAD also calls `offscreen::advance` and takes `job.deferred` (landed after b620e3333) | **included**, and the a-side **keeps** the advance/deferred lines: only the `gone` collection and the constant change |
| `production.rs` `tick`: `let mut gone = Vec::new();` and the `retake(...)` call in the return | `production.rs:1482, 1548-1549`; the return at HEAD also ORs `consumed` | **included**, a-side keeps `consumed` |
| `production.rs`: the `retake` fn (53 lines) | `production.rs:1531-1593` | **included** |
| `docs/settlements/production.md`: two lines | present | kept (docs) |
| `settlements/autocraft_tests.rs`: 73 test lines | present | kept (test) |

The a-side rebuilds the pre-fix bodies (no `gone`, no `retake`, plain
`"missing worker"` reason), using the current HEAD names, and keeps every line
that landed after b620e3333: the `offscreen::advance`/`job.deferred` park branch
and the `consumed` term of `tick`'s return. Reversing the patch therefore
removes the retake behaviour only; an earlier version of this patch also
reverted those later lines, which was a review finding and is fixed here. No
other row's behaviour is touched.

## Proving refs

- `cargo:mc-core:settlements::autocraft::tests::an_order_whose_worker_dies_passes_to_a_free_resident_and_finishes`
- `cargo:mc-core:settlements::autocraft::tests::resuming_a_held_order_of_a_dead_worker_hands_it_to_a_free_resident`

With the patch reversed the order stays bound to the dead worker (the resume
call returns `NotFound`), which both assertions reject.

## a/-side audit (B5, 2026-11-24)

Mechanical check: `git show <origin>^:<file>` vs the file after `git apply -R`
of this patch at the audit head; script and full output in
`receipts/D123/D123CivGates/a-side-audit.md`. Every line outside the origin
commit's own change, with the commit that last changed it and whether the
line is in `origin^:file`:

- `crates/mc-core/src/settlements/production.rs` `@@ -1482,10 +1489,13 @@ pub(crate) fn tick(world: &mut TickState, settlement: SettlementId, state: &mut` (adapted; origin `b620e3333`, discovered origin):
  - HEAD `return (park(job, MISSING_WORKER, tick_now.saturating_add(RETRY_TICKS))` — d28a29172 feat(settlements): reconcile offscreen production and daily food demand; in origin^: no
  - a/-side `return (park(job, "missing worker", tick_now.saturating_add(RETRY_TICKS))` — no commit; in origin^: no
- `crates/mc-core/src/settlements/production.rs` `@@ -1514,7 +1524,69 @@ pub(crate) fn tick(world: &mut TickState, settlement: SettlementId, state: &mut` (adapted; origin `b620e3333`, discovered origin):
  - HEAD `changed || consumed || retaken || !finished.is_empty()` — d28a29172 feat(settlements): reconcile offscreen production and daily food demand; in origin^: no
  - HEAD `job.retry_at = 0;` — d28a29172 feat(settlements): reconcile offscreen production and daily food demand; in origin^: no
  - a/-side `changed || consumed || !finished.is_empty()` — no commit; in origin^: no

HEAD-side lines whose last change is another commit are that later commit's
form: reversing them is the HEAD drift a control must reverse to reach the
pre-fix decision (rule (c)), not part of the fix. An a/-side line that is not
in `origin^` is an adaptation this control relies on; each such line is named
in the list above and changes no behaviour beyond the pre-fix decision.
