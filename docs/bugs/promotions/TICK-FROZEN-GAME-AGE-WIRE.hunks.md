# TICK-FROZEN-GAME-AGE-WIRE: patch-mode fix identity

Original fix: `2a315d20f4` "fix(core): keep wire game age stationary during tick freeze".
`fix_commit = "patch:23096ad88864b3a0eb3711e51b76e0ef50820f72cfe7cd1c8e02bfe7c1cb5ac4"`
names that fix's wire hunks on the candidate in
`docs/bugs/promotions/TICK-FROZEN-GAME-AGE-WIRE.patch`; `git apply -R` sends
the WAL frame counter (`snapshot.tick`) again.
Re-audit:
`python3 -m tools.harness ledger promote TICK-FROZEN-GAME-AGE-WIRE --fix-patch docs/bugs/promotions/TICK-FROZEN-GAME-AGE-WIRE.patch --dry-run`

## Why not the commit itself

`2a315d20f4` cannot be reversed on this tip: `crates/mc-edge/src/plan.rs` was
split into `plan/viewer_plan.rs` and the reversal of `boundary.rs`/`tick.rs`
would remove `WorldSnapshot.game_time`, which the kept refs read
(`tick_control::tests::game_time_stands_still_while_frozen_and_counts_stepped_ticks`),
so the control would not compile (E0609).

## Carried (3 wire sites, the original a/-text verbatim)

| original (2a315d20f4 a/-side) | HEAD |
|---|---|
| `plan.rs` `ClientboundSetTime::overworld(i64::try_from(snapshot.tick)…)` | `plan/viewer_plan.rs:159` |
| `play.rs` initial join clock `i64::try_from(out.snapshot.tick)…` | `play.rs:202` |
| `play.rs` closing join clock `i64::try_from(out.snapshot.tick)…` | `play.rs:287` |

`git show 2a315d20f4 -- crates/mc-edge/src/plan.rs crates/mc-edge/src/play.rs`
shows exactly these three deletions with the same text, so the `a/`-side is the
real pre-image and nothing is invented. The b/-side is the tip text
(`snapshot.game_time`), confirmed by `git apply -R --check`.

## Excluded

- `boundary.rs` `WorldSnapshot.game_time` field and `tick.rs`'s
  `game_time: state.tick_control.game_ticks(state.tick)` initializer — (b) API:
  the kept refs read `snapshot.game_time`; reversing stops the control from
  compiling. Both stay on both sides, so the control still sends the *WAL
  counter* while the field keeps existing.
- Test files (`tick_control_tests.rs`, `clock_queue_tests.rs`, `plan_tests.rs`,
  `play_tests.rs`) — kept (test hunks are not reversed).
- `docs/architecture/state-ownership.md`, ADR 0013, ledger — prose.

## Proving ref

`harness:scene-shot:native-frozen-game-age`, proving assertion
`frozen_game_time_holds_across_teleport`
(`tools/harness/backends/scene_shot_execute.py:58-65`, driven by
`frozen_game_age_scene.json` `assert_same_game_time`): with `tick freeze` held,
a teleport and a `time add 1000`, the client's received game age must not move.
Reversed, the scene sends `snapshot.tick`, which advances while frozen, and the
assertion is red. The three cargo refs are green in that control and are
coverage, not control evidence.
