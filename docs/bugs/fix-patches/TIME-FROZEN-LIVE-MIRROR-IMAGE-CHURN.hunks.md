# TIME-FROZEN-LIVE-MIRROR-IMAGE-CHURN — patch-mode fix identity (second rework)

Fix: `docs/bugs/fix-patches/TIME-FROZEN-LIVE-MIRROR-IMAGE-CHURN.patch`
(`fix_commit = "patch:f33834c1ae41ebccc665a3a6d07894a0fec676b9b5302cb3fb824fc4b58aad62"`).
It is a diff from the pre-fix form to this candidate; `git apply -R` restores
the pre-fix projection. Re-audit:
`python3 -m tools.harness ledger promote TIME-FROZEN-LIVE-MIRROR-IMAGE-CHURN --fix-patch docs/bugs/fix-patches/TIME-FROZEN-LIVE-MIRROR-IMAGE-CHURN.patch --dry-run`

## Why patch mode

The fix landed in `0d718b991b`, completed by `c8568f0a9` (the arithmetic:
`clock_since = tick - 1`, per-span cumulative held counts with
`partition_point`) and `a2cfc9670` (rustfmt). Reversing either commit alone
leaves the other's half in place, so the identity is the whole projection as
this candidate has it.

## Commits carried and what each closes

| Commit | Closes | Hunks in the patch |
|---|---|---|
| `0d718b991b` | the row's path: the live mirror no longer projects the raw activity interval through holds | `world.rs` clock fields (`clock_spans`/`clock_since`/`clock_held_base`/`clock_recorded` + their initializers); `world/clocks.rs` recorder, `simulated_game_ticks`, `game_ticks_at`, `for_each_running_interval`, `last_running_at` and the `refresh_active_clock` record call; `journal_touched.rs` call-site rewrites and the two `advance_*_mirror` helpers (the pre-image text verbatim) |
| `c8568f0a9` | the recovery boundary and the O(log n) spans | the same `world/clocks.rs` and `world.rs` hunks, in their final form (the patch is HEAD's text, so the follow-up's arithmetic is inside it) |

No original hunk is left out: the three files above are all the non-test
files the two commits touched, including the `ClockSpan` struct that
`c8568f0a9` added at the top of `world/clocks.rs` (the a/-side has no such
type; the adapted helpers do not need it).

Where the reds were obtained:

- the touched-mob, working-villager and differential reds come from the promote
  control (dry-run `.analysis/validation/20261123T205817-ledger-promote-bm4puhcj`,
  `negative_control` in its `promote.json`), i.e. from the adapted a/-side
  helpers computing the old interval arithmetic while the kept tests call them;
- the recovery guard is deliberately green there; its evidence is the mutation
  observed in `.analysis/validation/20261123T203951-test-w4dur139/test.log`
  (`clock_since = Some(tick)` reddens its own `is_empty`), reverted before the
  promotion.

## a/-side adaptation (the only text outside the original pre-image)

The kept tests call `ChunkWorld::simulated_game_ticks`,
`for_each_running_interval` and `last_running_at` directly (the differential
test). The pre-image has no such API, so reversing those hunks would stop the
control from building — reason (b). The a/-side therefore keeps the three
helper signatures and implements them with the pre-fix interval arithmetic
(`to - from`, one uninterrupted span, `last_running_at = tick`), which is
exactly what `journal_touched` computed inline before the mirror existed.
Everything else in the control is the pre-image: `journal_touched.rs` carries
the pre-image text (inline `advance_clocks` / `to - from`), `world.rs` has no
clock fields, and `refresh_active_clock` does not record a tick.

## Proving refs (all in `journal_touched_tests.rs`)

- `held_game_clock_does_not_churn_a_touched_mob` — the cow is touched every
  held tick, so the comparison itself runs; the control's raw advance emits an
  image and the test fails at its own `image_ids(...).is_empty()` assertion.
- `held_game_clock_holds_a_working_villager_baseline` — a non-wandering
  villager with the brain clock on (`no_ai = false`), touched every held tick;
  the control advances the age through the hold and fails at its own
  `is_empty()`/age assertions.
- `the_mirror_agrees_with_the_replay_game_clock_over_holds` — differential
  coverage required by the design note: work → hold → step → hold → release,
  then every `(from, to)` pair of the window must match both
  `ReplayGameClock::between` and its `running_intervals`/`last_running_at`;
  the control's raw projection disagrees during the holds.
- `recovered_mob_clock_does_not_burst_on_the_first_batch` — guard for the
  `clock_since = tick - 1` boundary. It is green in the control (the old
  interval matched the live row, so it is not evidence for the reversal), and
  it reddens when the fix itself is mutated: with
  `clock_since = Some(tick)` the first batch re-images the cow and the guard
  fails at its own `is_empty()` assertion (observed, then reverted).

The earlier red (`assert_baselines_complete`, the test-only fence) is not
evidence: `step_unfenced` keeps the fence out of these four tests.

## Known limit (documented, not hidden)

A restart in the middle of a hold starts the span history at the first tick
recorded after recovery (`R + 1`), so `last_running_at` inside that
pre-recovery hold returns `R` where replay's `held_since - 1` would; a
working/sleeping/panicking villager can therefore emit one extra image at the
release. The shift is recorded in `world/clocks.rs` (`last_running_at`) and
in the row: it is a conservative lag, never a wrong value, and the alternative
(seeding from `TickControl::held_since`, which the world does not receive)
would couple the projection to the control's internals.
