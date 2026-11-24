# CIV-SEED24-CROSS-VILLAGE-TREE-SWEEP — HEAD re-break (2026-11-24, D123CivE)

Patch: `docs/bugs/promotions/CIV-SEED24-CROSS-VILLAGE-TREE-SWEEP.patch`
(`patch:172bfd4304f08e5b8586737185ea6256836787a2d043677631d04e2bb3594591`), the
row's `fix_commit`. Reverse-applies at `978d656826` (`git apply -R --check`).

## Origin

Fix `197beb170` ("search nearby trees before crossing the village"), hunks in
`crates/mc-core/src/villager/lumber.rs` and
`crates/mc-core/src/settlements/construction.rs`. The patch carries every
non-test hunk of that commit, ported to HEAD. It was produced by applying
`git show 197beb170 -- <files>` **in reverse with `--3way`** and resolving only
the two places later commits rewrote, so every a/-side line is the base blob
unless named below.

## lumber.rs — reversed hunks (a/-side = `197beb170^` text)

1. Module doc: "scans its settlement radius a fixed number of columns" (the
   fix's "searches outwards from its feet" is b/).
2. `SKIP_PER_ATTEMPT` doc: "Inactive chunk strips skipped per work attempt, one
   lookup each".
3. `search` doc: the pre-image comment about the persisted cursor and the
   strip skip.
4. `search` signature: no `position: [i32; 2]` parameter.
5. `search` body: the grid column mapping (`slot / side`, `slot % side`)
   computes `[x, z]` inline instead of `column_at(origin, slot)`; `total` comes
   from `columns_total(radius)`, with no saved origin and no `reach`; the
   inactive-chunk **strip** skip (`(16 - z.rem_euclid(16))`) is back and the
   cursor advances after the activity check.
6. `replant`: the fix's `search_origin = None; cursor = 0;` reset is gone (the
   pre-image had only `drop_target(v)` before the later `swept`/`plant_cursor`
   lines).
7. `work`: the binding snapshot maps to `(building.id.0, building.revision,
   building.bounds)` again and the `natural`/`source` grove-cursor preset is
   gone.

### Kept on the pre-image side (named)

- **Class (b) — API the kept tests call.** `Progress.search_origin`,
  `StoredProgress.search_origin` (`#[serde(default)]`) and the `From` mapping
  stay: `lumber_tests.rs:328` builds a `Progress` with it and
  `persistence_tests.rs:799/947` round-trip it. `column_at` stays the 2-argument
  ring form because `lumber_tests.rs:304` asserts its ring corner
  (`column_at([0, 0], total - 1) == [-16, -15]`). Because `column_at` keeps the
  ring body, the pre-image grid mapping is **inlined in `search`** instead of
  calling it; that inline body is the verbatim pre-image `column_at` body
  (`197beb170^`), only its location changes.
- **Class (c) — later commits.** `swept`, `plant_cursor`, `plant_zone`, the
  pre-activity wrap ordering, and the `swept`/`plant_cursor` resets in
  `replant` (8657e77e4, f928f617f, 8cd4effc8) are kept; the pre-activity
  overflow decision is untouched, so `plant_zone` still runs exactly as at HEAD
  once the sweep wraps.

## construction.rs — reversed hunks

8. The `MC_CORE_PHASE_LOG` "settlement planner"/"settlement resident" diagnostic
   block inside `tick` (the fix's instrumentation; no test reads it).
9. `#[derive(Debug)]` on `WaitingKind` and `WaitingNeed`, which existed only for
   that block's `{:?}` (no other reader).

## Red (observed on the reversed tree, both refs)

Scratch worktree `wt/D123Civ-ctl-lumber` at `a57a56706e` with this patch
reversed, `data/` provisioned:

- `lumberjack_searches_near_worker_before_cross_village_sweep` FAILED at
  `lumber_tests.rs:170` — `left: [1, 201, 10]`, `right: [10, 201, 12]`: the
  worker took the cross-village tree instead of the near one. Receipt
  `.analysis/validation/20261124T112028-test-uz4_0ywp` (exit 100).
- `lumberjack_restarts_nearby_search_after_harvest` FAILED at
  `lumber_tests.rs:350` — `left: [0, 0, 0]`, `right: [10, 201, 10]`: after a
  harvest the search did not restart at the worker. Receipt
  `.analysis/validation/20261124T112218-test-a84mskd3` (exit 100).

## Green (observed on the clean candidate tree)

- `lumberjack_searches_near_worker_before_cross_village_sweep` PASS
  (`.analysis/validation/20261124T112447-test-5ewnvajo`, 2 passed).
- `lumberjack_restarts_nearby_search_after_harvest` PASS
  (`.analysis/validation/20261124T113127-test-9e8mujs_`).

## Red-then-green note

The first attempt (`20261124T110821-test-f3bzi73h`) compiled the control but
aborted before the assertions because the scratch worktree had no `data/`
(`Registry(...allay.json: No such file)`) — a fixture failure, not evidence;
`data/` was hardlink-copied from the lane worktree and the run repeated.

## Still missing (not a control problem)

The row has **no registered graphical check at all**: the proposed
`harness:civilization-seed-qa:cross_village_tree_sweep` milestone is not
authored and not a registered verdict, and the civilization gate itself is red
on its progression rule (CIV-6 wood bootstrap). A `client`-repro row cannot be
promoted on a cargo pair, so it stays `open`.
