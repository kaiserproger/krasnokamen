# ENTITY-CROWD-PUSH-INTO-WALL — fix-hunk map

The production patch `ENTITY-CROWD-PUSH-INTO-WALL.patch` is the fix's non-test hunks,
`patch:c6cf66c380f1467577aa538962202fe7803f42f07225c6dc0d88b5f530797755`. It reverse-applies on the
candidate (`git apply -R --check`, clean tree at `4aad4c22c`).

## The fix and the hunks the patch carries

| commit | file | hunk | in the patch? |
|---|---|---|---|
| `fe6e7d36f` | `crates/mc-physics/src/lib.rs` | `resolve_horizontal_sweep` loses the pre-clip pass; `contact_entry(gap, movement)` turns a signed face gap within `-COLLISION_EPSILON..0` into `entry = 0.0`; `sweep_axis_interval` routes both directions through it | **yes** |

`d65bfc936` added the pen regressions; it is a test-only commit and nothing is reversed.

## Why every other hunk of the two fix commits stays out

The row was merged from two commits (`7cca1476d` body sizes from the vanilla f32 contract + refuse
unsolvable moves, `fe6e7d36f` this kernel clamp + live body sizes). Only the kernel hunk can be
reversed without breaking the kept tests:

- **`7cca1476d`** — every hunk: reason **(c)**, no HEAD equivalent exists. Verified:
  `git apply -R --check` of `git diff 7cca1476d^ 7cca1476d -- crates/` on `4aad4c22c` fails on
  `blueprint.rs`, `falling.rs`, `tick/{auth,elytra,players,input/*}.rs`, because `fe6e7d36f` and later
  commits rewrote those lines. Its `blueprint.rs` hunk additionally introduced
  `contract_dimensions`, `PLAYER_CROUCHING_HEIGHT` and `PLAYER_FALL_FLYING_HEIGHT`, which the kept
  tests call directly (`blueprint_tests.rs:383,424,433,446,450`, `elytra_tests.rs:150,423`,
  `sleep_tests.rs:1070,1074,1160`, `villager/brain_tests.rs:983`) — reason **(b)**.
- **`fe6e7d36f`'s `blueprint.rs` hunk** — reason **(b)**: it makes `PLAYER_FALL_FLYING_HEIGHT` and
  `PLAYER_CROUCHING_HEIGHT` `const f64` and adds `blueprint_dimensions`/`living_dimensions`/
  `SLEEPING_POSE_SIZE`; `blueprint_tests` compares the constants to `f64` and `sleep_tests` divides
  `SLEEPING_POSE_SIZE`, so reversing it stops the control from compiling.
- **`fe6e7d36f`'s live-body-size call sites** (`villager/{mod,brain}.rs`, `sleep.rs`, `warden.rs`,
  `tick/input/movement.rs`, `falling.rs`, `entity_loot.rs`, `tick/{auth,elytra,players,input/session}.rs`)
  — reason **(a)**: they are the row's other half (a live refresh writing a hand-written body size),
  whose red is `a_summoned_villager_keeps_the_contract_body_through_the_appearance_refresh` /
  `a_woken_player_carries_the_player_contract_body`, not the kernel clamp this control measures. They are
  already at HEAD, so the control tree carries them.
- **`crates/xtask/src/code_health/ratchet.rs`** — reason **(c)**: `f45907ef0` rewrote the ceiling after
  the fix, so the identical line no longer exists.

## The proving ref and why the red is the symptom

`cargo:mc-core:tick::crowd_pen_tests::crowded_pen_bodies_never_enter_the_wall` is the row's own
assertion: 32 zombies pressed by a chase target against a pen wall for 240 ticks, and no body AABB may
overlap a solid cell. The test pins the pre-report half width itself (`set_half_width(slot, 0.3)`, the
hand-written f64 literal the wall report came from) precisely so it "stands on the movement kernel
alone and fails when only the physics hunk is reverted".

The two `mc-physics` refs are the kernel invariant at the same seam
(`wall_face_snap_never_lets_a_body_cross_the_face`,
`a_contact_never_grows_deeper_than_the_tolerance_behind_a_face`). The remaining registered core refs
(`blueprint_tests::*`, `sleep::tests::a_woken_player_carries_the_player_contract_body`,
`tick_tests::elytra_tests::an_unsolvable_reported_move_is_refused_instead_of_teleporting_the_player`)
are green on both trees and are not evidence for this control.

## Observed control and restore

Filled in by the promotion receipt (`promote.json` under `.analysis/validation/`). The a/-side is the
real pre-image of `fe6e7d36f^` for `mc-physics/src/lib.rs` — the pre-clip block with its original
comment, and `sweep_axis_interval` dividing the raw gap. No adaptation line exists on this patch.
