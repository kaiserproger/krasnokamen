# ENTITY-COLD-ROW-READ-IN-FULL-PER-ADMITTED-ENTITY-TICK — fix-hunk map

The production-only patch `ENTITY-COLD-ROW-READ-IN-FULL-PER-ADMITTED-ENTITY-TICK.patch` is one hunk,
SHA-256 `patch:9022f2f1b701af5eb64a955224e4a39559b243dbed27c6db5ec43e6aa3d13978`. Reverse-applies on
the candidate (`git apply -R --check` on `e2e341a2bb`, clean tree).

## The fix and the hunk the patch carries

The fix (commit `071a68f8e4`, produced from the Opus design verdict in `COLD-ROW-DESIGN.md`) adds a
third derived column `hot: Vec<HotRow>` (32 B) beside `flags`/`hints`, refreshed only from the existing
dirty queue in `prepare_motion_hints`, and splits `read_row` into a gate plus `read_row_with`. The
patch reverses the gate only:

```diff
-    let cold = if view.entities.motion_dirty(slot) || hot.needs_cold() {
-        Some(view.entities.row(slot))
-    } else {
-        None
-    };
+    let cold = Some(view.entities.row(slot));
```

The column, its maintenance, `HotRow::needs_cold`, the debug cross-check in `EntityStore::hot_row` and
the test-only counter in `EntityStore::row` all stay on the control — so the control is the fix with
only the fast path removed, and every type and helper the kept tests use still exists.

## The a/-side against the real pre-image

The pre-image is the per-row code before `071a68f8e4`: `prepare.rs::read_row` read
`view.entities.row(slot)` unconditionally, with no `HotRow`, no gate and no `read_row_with` split. The
a/-side keeps the split (rule (b): the kept tests call `read_row` and the counter accessor) and the
column, and differs from it only by the column machinery the (b) adaptation requires. No invented
text, no `#[allow]`, no counter outside `EntityStore::row` (which both trees carry).

## Why this control is honest

The registered ref is
`cargo:mc-core:tick::entity_phase::entity_phase_tests::read_row_reads_no_cold_row_for_plain_admitted_rows`.
It prepares a plain fixture (cows in love, so `MOTION_REST` is off) and asserts
`EntityStore::take_cold_row_reads() == 0` for both the serial and the parallel prep. On the control
each admitted row reads the live row, so the count is one per admitted row — the row's own cost, no
counter increment the fix does not also perform. The other four refs pin the column's size, its
writers, the dirty-after-refresh path and live vehicle occupancy.

## Observed control and restore

Recorded below once promoted.

## What stays out of the row

The row's forbidden fields (age, harvest_ticks, no_action_time, uuid, villager internals, `occupied`)
are not in the column; `MOTION_EFFECTS`, riders, `special_mob_some` and a piglin body force the cold
path. `ColdRow`'s size and layout are unchanged, and no wall-clock claim is made.
