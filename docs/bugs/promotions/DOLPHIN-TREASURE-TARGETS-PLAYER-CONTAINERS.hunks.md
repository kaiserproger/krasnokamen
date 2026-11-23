# DOLPHIN-TREASURE-TARGETS-PLAYER-CONTAINERS — fix-hunk map

The production-only patch `DOLPHIN-TREASURE-TARGETS-PLAYER-CONTAINERS.patch` is the forward-ported fix at
HEAD, SHA-256 of its non-test hunks
`543cfd56eed2745560a73f412e31e8f6485fb80ade0ce256a0cb92334a091855`.

The fix landed in two commits on `wt/D123Battle`:

| Commit | What it closes for this row |
|---|---|
| `2c3e1fa2e2a8ac6af9f649dc2917e9ebac2636c2` (`fix(slime,dolphin): hop locomotion and located-structure treasure leads`) | Deletes `nearest_treasure` (the 48-block chest/trapped-chest/barrel scan) and the direct 5-block container step; a fed dolphin now follows a `#minecraft:dolphin_located` structure lead from the async locate worker, and no located structure is vanilla `stuck` (`stop` clears GotFish). |
| `c7ae9108f2` (`fix(slime,dolphin): restore the aggressive jump delay and retarget the treasure regressions`) | Retargets the two dolphin regressions so they discriminate the container approximation (player parked off-axis, chest 10 blocks away on the side opposite the wreck; the missing-structure case asserts no lead and closest approach > 3 blocks). Test-file only for this row. |

| Original production change | Current patch equivalent |
|---|---|
| `2c3e1fa2e2`: `crates/mc-core/src/dolphin.rs` `treasure_search` — the container approximation (scan + 5-block step + clear within 16 squared blocks) replaced by the locate-worker lead and the `Missing` arm stopping the goal | Patch's only hunk, `DolphinTreasure::Missing` arm: a/-side re-expresses the deleted approximation at HEAD (nearest chest/trapped chest/barrel by `world.block_at` in the pre-fix 48×24×48 box, 5-block step through `resolve_collision_displacement`, `stop_treasure` within 16 squared blocks); b/-side is HEAD's `stop_treasure`, the vanilla `stuck` path. |

Original non-test exclusions:

- reason (c): the deleted `nearest_treasure` helper itself and the pre-fix `treasure_search` body no longer
  exist at HEAD, so their only HEAD equivalent is the patch's a/-side arm;
- reason (b): `crates/mc-core/src/command_locate.rs` (`DolphinTreasure`, `dolphin_treasure`,
  `request_dolphin_treasure`, `dolphin_navigation`, `set_dolphin_navigation`) — the kept tests call it, so
  reversing it stops the control from compiling;
- reason (a): the slime hunks of `2c3e1fa2e2`/`c7ae9108f2` are `SLIME-LOCOMOTION-WALKS-INSTEAD-OF-HOPS`'s
  behaviour and stay on both sides.

Proving ref: `cargo:mc-core:dolphin::tests::a_player_chest_without_a_located_structure_is_not_a_treasure_lead`
— the control stops red at `crates/mc-core/src/dolphin_tests.rs:590`
("a missing structure stops the goal and clears GotFish", receipt
`.analysis/validation/20261123T164031-test-n2mqlp2v/`, log
`.analysis/codex-logs/D123Battle/control-g2-missing-branch.log`). Also registered and green on both sides:
`cargo:mc-core:dolphin::tests::fed_dolphin_locates_the_structure_through_its_normal_goal_tick`.

Vanilla reference: `Dolphin$DolphinSwimToTreasureGoal` 26.1.2 (`findNearestMapStructure` with the
`#minecraft:dolphin_located` tag, radius 50; `stuck` → `stop` clears `GotFish`). Source/test row, no
real-client surface.
