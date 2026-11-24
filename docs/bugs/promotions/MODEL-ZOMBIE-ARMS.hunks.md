# MODEL-ZOMBIE-ARMS — hunk map for the tip reversal pair

Promotion receipt: `promotions/MODEL-ZOMBIE-ARMS.json`.
Client pair: reversal `client:929e6e9eba` → re-land `client:3ad9f80b29` (the row's `fix_commit`).
The pair's net tree is byte-identical to client `main` `530556e8`.

## The commits that closed this row's path

The 2026-11-02 audit read binaries from before `43362918`: `evaluate_rig` had no per-type zombie
pose and the rig rest rotations are `None`, so every zombie family member hung its arms. The path was
then closed in three commits, all of the same one behaviour:

| commit | part of the path it closed | hunk |
|---|---|---|
| `client:43362918` (2026-11-03) | introduces the zombie arm pose: `rotation = Vec3::ZERO` for the family, `-90°` idle / `-2.1` aggressive, an attack term | `src/entities/presentation.rs` `@@ -1198,19 +1445,30 @@ pub(super) fn evaluate_rig(` (its `+` side is the pose block) |
| `client:83994480` (2026-11-09) | exact 26.3 angles: `-80°`/`-120°`, bob 0.07, swing 1.1/0.35, yaw and roll terms | same region, `@@ -1198,19 +1445,30 @@` of `83994480` — it rewrote the block `43362918` added |
| `client:ebe56023` (2026-11-12) | extracts `zombie_arm_rotation` (swing 1.2/0.4, bob 0.067, the baby-with-item hang) and moves the call to `role_pose` | `@@ -1466,6 +1466,41 @@ impl AxisMap` (the helper) and `@@ -1542,22 +1577,9 @@ pub(super) fn role_pose` (the call, which is the carried block) |
| `client:54b79f73d` (2026-11-14) | yields the pose on a spear/shield arm: the guard `blocking_arm != Some(arm)` and the `blocking_arm(e, …)` computation | the carried block's `if` condition; a behavioural refinement of the same path (README:258-261 "fix in several commits"), so it is named here, not omitted |
| `client:9a645aa99` (2026-11-14) | no behaviour: clippy/rustfmt reflow of the call and the `match` in `pot_side_texture` | the carried block's `AxisMap::set(zombie_arm_rotation(…))` line layout |

## Hunks carried by the pair

| file | HEAD form | disposition |
|---|---|---|
| `src/entities/presentation.rs` (`role_pose`, `arms` array) | `if zombie_arms && blocking_arm != Some(arm) { let side = …; map = AxisMap::set(zombie_arm_rotation(side, (zombie_arms_raised, aggressive), age, attack)); }` | REVERSED — `929e6e9eba` deletes these nine lines and nothing else; `3ad9f80b29` restores them byte-for-byte (see `git diff 929e6e9eba 3ad9f80b29`). The helper `zombie_arm_rotation` and the flags stay in place (reason (b): the kept `animation_tests` ref calls them, and deleting a `pub(super)` helper that tests name would stop the control from compiling); the control's arm map then carries only the walk swing, which is the row's symptom |

## Pair-only lines (Main's mechanical check: 9 of 9)

The pair's changed lines (`git diff 929e6e9eba 3ad9f80b29 -U0`) are 9 added and 0 removed. None of
them is in `ebe56023`'s own diff; `git blame -C -C 530556e8` names each:

| pair-only line (`src/entities/presentation.rs`, `role_pose`) | blame commit | class |
|---|---|---|
| `if zombie_arms && blocking_arm != Some(arm) {` | `54b79f73d` | **behavioural** — the spear/shield yield; added to the commit table above |
| `let side = if arm == 0 { 1. } else { -1. };` | `a89b5664d` | (c) pre-existing pose scaffolding (the crowd-instancing commit created the per-arm loop); it is an inner line of the deleted block and carries no behaviour of its own |
| `map = AxisMap::set(zombie_arm_rotation(` + its four argument lines + `));` | `9a645aa99` | (c) clippy/rustfmt reflow of `ebe56023`'s call |
| the closing `}` | `a89b5664d` | (c) the same pre-existing block terminator |

## Every omitted hunk on this path

| commit | hunk | disposition |
|---|---|---|
| `43362918` | `@@ -724,6 +1132,17 @@ evaluate_rig` — adds `zombie_arms`, `zombie_arms_raised`, `aggressive` | (b) kept: the carried block and the kept unit ref read these three flags; removing them stops the control from compiling and would red the formula ref, not the row's assertion |
| `43362918` | `@@ -772,6 +1196,61 @@ evaluate_rig` — the per-type pose block: zombie arms, spider leg swing, pet sitting, end-crystal float | (c) no HEAD lines remain: `83994480` rewrote the zombie part and `ebe56023` moved the pose to `role_pose`; the spider/pet/crystal parts are (a) other rows' behaviour and are superseded too. The surviving zombie behaviour is the carried `role_pose` block |
| `43362918` | `@@ -764,7 +1183,12 @@ evaluate_rig` — `WingNegativeX`/`WingPositiveX` roles | (a) another row (chicken/parrot wings) |
| `43362918` | the remaining `presentation.rs` hunks (`variant`, `material`, `append`, `special_item`, …) | (a) other rows' models and materials; none touches the arm pose |
| `83994480` | `@@ -1198,19 +1445,30 @@ evaluate_rig` — rewrote the zombie pose to `-80°`/`-120°`, bob 0.07, yaw/roll, swing 1.1/0.35, drowned trident | (c) no HEAD lines remain: `ebe56023` extracted the formula into `zombie_arm_rotation` and moved the call; the formula itself is kept (b) behind the kept unit ref |
| `83994480` | `@@ -1126,8 +1367,11 @@ evaluate_rig` — introduces `arms`/`equipped_arm_rotations` | (b) the per-item use-pose path (MODEL-USE-POSE's row) that computes the arm maps `role_pose` later overrides; kept |
| `83994480` | the remaining `presentation.rs` hunks (nearly the whole file: draw_entity, material, append, special_item, …) | (a) other rows' behaviour |
| `ebe56023` | `@@ -1466,6 +1466,41 @@ impl AxisMap` — adds `zombie_arm_rotation` | (b) kept: the carried call and the kept unit ref `animation_tests::zombie_arm_rotation_matches_the_26_3_animate_zombie_arms_formula` name it |
| `ebe56023` | `@@ -1542,22 +1577,9 @@ role_pose` — the pose block | CARRIED (see the table above) |
| `54b79f73d` | the `blocking_arm` guard inside the carried block | CARRIED; its `blocking_arm(e, models, content)` computation is (b) kept, because the carried condition reads it |
| `54b79f73d` | the rest of its stage0-drift repair (module declarations, dedupe, other files) | (a) build repair, not this row |

`CM1-REVERSIBILITY.md` records that both bulk commits' other presentation hunks no longer reverse at
HEAD; this table is the per-hunk form Main asked for.

## a/-side adaptation

None beyond the deletion. The control tree keeps `zombie_arm_rotation`, `zombie_arms`,
`zombie_arms_raised`, `aggressive` and `blocking_arm` exactly as HEAD has them; the only change is
that no arm is assigned the pose, so the reversed behaviour is "the arms never rise" — no invented
stub, no flag, no early return.

## Proving ref

`harness:scene-shot:issue1-19-hostile-motion` → `zombie_arm_raised_pixels`
(`tools/harness/backends/scene_shot_steps.py:255`, the scene's `zombie_arm_raised` pixel window over
`zombie-arms-side.png`, window `[665, 295, 730, 340]`, zombie-skin colour box, `min_pixels = 800`).
The frame is a fixed side view of a `NoAI` zombie at `999.5 160 1004.5` from `(1004.0, 161.4,
1004.5)` yaw 90, after 240 ticks so the idle bob has settled. The scenario's `mob_speed_*` refs are
not this row's assertion; after the walk windows moved to two-block steps they also assert the step
itself (`mob_step_zombie`/`mob_step_spider` inside `[1.5, 2.5]` blocks) and keep only a coarse
client-tick band `[0.02, 0.12]`, because each segment's elapsed ticks include every command
round-trip (measured 44-48 ticks for a two-block step, i.e. 0.042-0.045 blocks/tick). With the pose the window holds the
raised forearm (a measured 1802 skin pixels on the candidate); with hanging arms the window holds
only the sky/grass behind the torso, so the row's own assertion fails. The unit ref
`client:krasnokamen-client:entities::animation_tests::zombie_arm_rotation_matches_the_26_3_animate_zombie_arms_formula`
covers the formula and stays green on the candidate.
