# TIME-FROZEN-LIVE-MIRROR-IMAGE-CHURN — patch-mode fix identity (review-reworked)

Fix: `docs/bugs/fix-patches/TIME-FROZEN-LIVE-MIRROR-IMAGE-CHURN.patch`
(`fix_commit = "patch:04e20068872aec5d77762675d0435921f39580e4526415bdf596252277a3be67"`).
It is a diff from the pre-fix form to this candidate; `git apply -R` restores
the pre-fix projection. Re-audit:
`python3 -m tools.harness ledger promote TIME-FROZEN-LIVE-MIRROR-IMAGE-CHURN --fix-patch docs/bugs/fix-patches/TIME-FROZEN-LIVE-MIRROR-IMAGE-CHURN.patch --dry-run`

## Why patch mode

The fix landed in `0d718b991b` (the mirror) and a follow-up chunk
(`clock_since = tick - 1`, per-span cumulative held counts with
`partition_point`, doc corrections). Reversing the follow-up commit alone would
leave the original mirror in place, so the identity is the whole projection:
the `ChunkWorld` helpers in `world/clocks.rs` as this candidate has them.

## What the patch carries

The game-clock projection: `simulated_game_ticks` (game ticks simulated in
`(from, to]`), `for_each_running_interval` (running spans with the boundary
before each span) and `last_running_at`, all built from the held spans
`record_clock_tick` accumulates with per-span cumulative counts. Reversed, the
three helpers return the pre-fix values: the raw activity interval as if it
were running time, one uninterrupted span, and the tick itself.

## a/-side adaptation

`journal_touched.rs`'s `advance_mob_mirror`/`advance_villager_mirror` and the
differential fence keep their names and call shape on both sides because the
kept tests call `simulated_game_ticks` directly (the differential test). The
pre-fix form of `clocks.rs` therefore keeps the helpers as inert wrappers with
the old inline semantics (`to - from`, one span, `last_running_at = tick`)
instead of deleting them. That is a preserved-API constructor, and it
reproduces exactly what `journal_touched` computed inline before the mirror
existed (`simulation_interval` + `to - from`).

## Proving refs (all in `journal_touched_tests.rs`)

- `held_game_clock_does_not_churn_a_touched_mob` — the cow is touched every
  held tick, so the comparison itself runs; the control's raw advance emits an
  image and the test fails at its own `image_ids(...).is_empty()` assertion.
- `held_game_clock_holds_a_working_villager_baseline` — a non-wandering
  villager with the brain clock on (`no_ai = false`), touched every held tick;
  the control advances the age through the hold and fails at its own
  `is_empty()`/age assertions.
- `recovered_mob_clock_does_not_burst_on_the_first_batch` — a guard for the
  `clock_since = tick - 1` fix: recovery seeds the baseline at R and the first
  batch runs at R + 1. It is green in the control (the raw advance matched the
  live row); it reds if the recovery history is off by one.
- `the_mirror_agrees_with_the_replay_game_clock_over_holds` — differential
  coverage required by the design note: work → hold → step → hold → release,
  then every `(from, to)` pair of the window must match `ReplayGameClock`;
  the control's raw projection disagrees during the holds.

The earlier red (`assert_baselines_complete`, the test-only fence) is no longer
the evidence: `step_unfenced` keeps the fence out of these four tests.
