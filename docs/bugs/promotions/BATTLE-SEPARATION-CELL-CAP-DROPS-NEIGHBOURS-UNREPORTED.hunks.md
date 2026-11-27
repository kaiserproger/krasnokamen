# BATTLE-SEPARATION-CELL-CAP-DROPS-NEIGHBOURS-UNREPORTED — fix-hunk map

The row's symptom is a *reporting* gap, not a lost body: `SeparationGrid::rebuild` has always
dropped candidates past `CELL_CAP = 8` per one-block sub-cell and kept its own `overflow` count
(asserted by `dense_cells_bound_inspection_and_impulses_to_five_neighbours`), but nothing mirrored
that count into `FormationWork`, so a RANKS gate receipt could not say whether a heap was measured
with bodies missing. Fix `c393b9240` added the field, the mirror helper and the call from
`steering::step`.

## Original hunks and their HEAD equivalents

| original file | hunk | HEAD equivalent |
|---|---|---|
| `crates/mc-core/src/battle/formation.rs` | `Work::separation_overflow` field (+4) | same field at `formation.rs:260-264` |
| `crates/mc-core/src/battle/steering.rs` | `record_grid_overflow` helper (+8 with its doc) | same helper at `steering.rs:462-467` |
| `crates/mc-core/src/battle/steering.rs` | the call `record_grid_overflow(&mut battle.formation);` in `step` (+1) | **carried by this patch** |
| `crates/mc-core/src/battle/steering_tests.rs` | the test `separation_overflow_reaches_the_battle_work_counters` | test file: not carried |
| `entity_phase*`, `system_graph*`, `world.rs`, `mc-physics`, `dolphin.rs`, `passive_follow.rs` | dash pooling, row borrowing, `clip_y` tolerance, import cleanup | **(a)** another row's behaviour (performance/entity-phase rows), not this row's path |

Omitted hunks:

- `formation.rs` field — **(b)** API the kept refs call: the registered test reads
  `Work::separation_overflow`, and `ranks_tests` already names it in the RANKS line. Reversing it
  would stop the control from compiling instead of letting an assertion fail.
- `steering.rs` helper — **accepted kept-helper class**: a helper the fix created stays in the
  control tree unreferenced after the reversal; its only caller is the carried hunk. It carries no
  behaviour unless it is called, so the control still measures the missing mirror.
- every other file — **(a)** another row's/owner's behaviour; reversing them would move unrelated
  rows and is not this row's path.

No a/-side adaptation lines exist: the patch is only the production call, and the reversed tree is
byte-identical to the pre-image of that call.

## What the patch proves

The registered ref is

```
cargo:mc-core:battle::ranks_tests::the_production_tick_reports_the_separation_grids_dropped_candidates
```

It stages a 60-a-side battle, runs one production `run_tick`, moves every soldier's **entity** into
one sub-cell, and runs the tick again: the assertion is that
`battle.formation.work.separation_overflow == battle.formation.neighbours.overflow` and is non-zero.
With the mirror call reverted the work counter stays at its zero default while the grid still drops
992-of-1000-class candidates, so the ref reds at its own assertion.

The older `a_dense_heap_reports_dropped_separation_candidates` (ranks_tests) and
`separation_overflow_reaches_the_battle_work_counters` (steering_tests) both call the helper
themselves; its own comment (HANDOFF-5 H4) says a removed mirror call leaves them green. They stay
in the tree as counter-shape tests and are **not** registered on the row.
