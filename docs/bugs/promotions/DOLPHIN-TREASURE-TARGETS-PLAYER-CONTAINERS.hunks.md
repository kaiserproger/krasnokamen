# DOLPHIN-TREASURE-TARGETS-PLAYER-CONTAINERS — fix-hunk map

The production-only patch `DOLPHIN-TREASURE-TARGETS-PLAYER-CONTAINERS.patch` is the forward-ported fix at
HEAD, SHA-256 of its non-test hunks
`e44b5f3e0866a8cd3068568fa51a585adf2fe273cb18c7d2378b7ffd2d05d096`.

Regenerated on `1c2e9db153` to reverse the **whole** dolphin path of `2c3e1fa2e` (the round-6 review: only
the `Missing` arm was reverted, so the control still held the new locate request, the `Found`/`plan_treasure`
branches and the lead steering, and went red on a weak goal-state assertion). The a/-side is now the pre-fix
`treasure_search` + `nearest_treasure` (the file's bytes at `2c3e1fa2e^`:195-367), which scans player
containers and steps the dolphin toward them; every hunk of `2c3e1fa2e` on this path is either in the patch
or listed below with a reason.

The fix landed in two commits on `wt/D123Battle`:

| Commit | What it closes for this row |
|---|---|
| `2c3e1fa2e2a8ac6af9f649dc2917e9ebac2636c2` (`fix(slime,dolphin): hop locomotion and located-structure treasure leads`) | Deletes `nearest_treasure` (the 48-block chest/trapped-chest/barrel scan) and the direct 5-block container step; a fed dolphin now requests a `#minecraft:dolphin_located` structure lead from the async locate worker, walks water waypoints toward it, and no located structure is vanilla's `stuck` (`stop` clears GotFish). |
| `c7ae9108f2` (`fix(slime,dolphin): restore the aggressive jump delay and retarget the treasure regressions`) | Retargets the two dolphin regressions so they discriminate the container approximation (player parked off-axis, chest 10 blocks away on the side opposite the wreck; the missing-structure case asserts no lead and closest approach > 3 blocks). Test-file only for this row. |

## Original `2c3e1fa2e2` hunks on `crates/mc-core/src/dolphin.rs` and their HEAD equivalent

| Original hunk | Current patch |
|---|---|
| `@@ -1,7 +1,7` — drop `EntityKind` from `use crate::entities::{EntityKind, EntitySlot};` | Included: the a/-side restores the import, because the pre-fix body matches on `EntityKind::Mob`. |
| `@@ -12,7 +12,6` — the blank line between `DolphinState` and `is_dolphin` | Included (cosmetic; a/-side restores the blank line so the patch is a complete reversal of the commit's `dolphin.rs` text). |
| `@@ -37,12 +36,14` — `drive`'s doc comment and the new `treasure: Option<[f64; 3]>` parameter | Doc comment included (a/-side is the pre-fix wording); **the parameter is retained, reason (b)**: `crates/mc-core/src/dolphin_tests.rs:204` and `:217` call `drive(pos, air, speed, None, …)` with that argument, so reversing the parameter stops the control from compiling. Its `#[allow(clippy::too_many_arguments)]` comment rides along with the parameter. |
| `@@ -61,6 +62,17` — the `if let Some(target) = treasure { … }` steering branch inside `drive` | Included: the a/-side omits the branch, so the control steers only at players/toys like the pre-fix code. |
| `@@ -97,6 +109,7` — `treasure_search(state);` before `run`'s early `return` for an empty dolphin list | Included: the a/-side restores the plain early return (no behaviour difference: with no dolphin there is nothing to search). |
| `@@ -192,178 +205,243` — the pre-fix `nearest_treasure` + container-scanning `treasure_search` replaced by the locate-lead `treasure_search`, `stop_treasure`, `plan_treasure` and `water_waypoints` | Included: the a/-side is the pre-fix `nearest_treasure` + `treasure_search` verbatim (48×24×48 overlay/chunk scan for chest/trapped chest/barrel, 5-block collision-resolved step, clear within 16 squared blocks) and drops `stop_treasure` (only the replaced body called it). `plan_treasure` and `water_waypoints` stay, reason (b): `crates/mc-core/src/dolphin_tests.rs:428` calls `plan_treasure`, and `water_waypoints` is its helper (reversing them stops the control from compiling). |

## Retained hunks and why (rule (b) — API the kept tests call)

- `crates/mc-core/src/tick/entity_phase/step.rs`: `StepCtx.dolphin_targets` and the
  `ctx.dolphin_targets.get(&row.id).copied()` argument at the `crate::dolphin::drive(…)` call, plus
  `crates/mc-core/src/tick/entity_phase.rs`'s `dolphin_targets: std::sync::Arc::new(state.locate.dolphin_targets(state.dimension))`.
  They supply `drive`'s retained `treasure` parameter; the only reversal of the call site is to drop the
  argument, which changes the arity of the callee those kept tests call — the control would not compile.
- `crates/mc-core/src/command_locate.rs`: `DolphinTreasure` (with its `Pending`/`Found`/`Missing`/`Failed`
  arms), `DolphinNav`, `dolphin_treasure`, `dolphin_navigation`, `set_dolphin_navigation`,
  `request_dolphin_treasure` and the `#[cfg(test)] await_queued_search` hook. The kept tests call
  `dolphin_treasure` (`dolphin_tests.rs`, `command_locate_tests.rs`), `request_dolphin_treasure`
  (`command_locate_tests.rs`), `dolphin_navigation` and `await_queued_search` (`dolphin_tests.rs`), so
  reversing the module stops the control from compiling. (The hook lives in a non-test file but is
  `#[cfg(test)]`; it is not and must not be recorded as a `fix_commit`.)
- Reason (a): the slime hunks of `2c3e1fa2e2`/`c7ae9108f2` are `SLIME-LOCOMOTION-WALKS-INSTEAD-OF-HOPS`'s
  behaviour and stay on both sides.

## Control

Reversing the patch on the candidate gives the pre-fix `treasure_search`: a fed dolphin in water scans for
the nearest chest/trapped chest/barrel within 48 blocks and steps 5 blocks toward it every tenth tick. In
`a_player_chest_without_a_located_structure_is_not_a_treasure_lead` the chest sits 10 blocks on +x, so the
control walks the dolphin onto it and fails at the **headline** assertion
`crates/mc-core/src/dolphin_tests.rs:616` (the `assert!` macro; the expression is :617) — `a player chest must not attract the dolphin: closest squared
{closest}` — now ordered before the goal-state assertions (they fail on any non-stop `Missing` arm and are
not evidence on their own). The structure ref
`cargo:mc-core:dolphin::tests::fed_dolphin_locates_the_structure_through_its_normal_goal_tick` goes red on
the lead it can no longer produce; the restore is green on both refs.

Proving refs: `cargo:mc-core:dolphin::tests::a_player_chest_without_a_located_structure_is_not_a_treasure_lead`
and `cargo:mc-core:dolphin::tests::fed_dolphin_locates_the_structure_through_its_normal_goal_tick`.

Vanilla reference: `Dolphin$DolphinSwimToTreasureGoal` 26.1.2 (`findNearestMapStructure` with the
`#minecraft:dolphin_located` tag, radius 50; `stuck` → `stop` clears `GotFish`). Source/test row, no
real-client surface.
