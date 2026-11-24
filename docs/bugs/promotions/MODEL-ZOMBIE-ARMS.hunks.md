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
| `client:ebe56023` (2026-11-12) | extracts `zombie_arm_rotation` (swing 1.2/0.4, bob 0.067, the baby-with-item hang) and yields a blocking arm | the pose block becomes `if zombie_arms && blocking_arm != Some(arm) { … zombie_arm_rotation(…) }` in `role_pose` |

## Hunks carried by the pair

| file | HEAD form | disposition |
|---|---|---|
| `src/entities/presentation.rs` (`role_pose`, `arms` array) | `if zombie_arms && blocking_arm != Some(arm) { let side = …; map = AxisMap::set(zombie_arm_rotation(side, (zombie_arms_raised, aggressive), age, attack)); }` | REVERSED — `929e6e9eba` deletes these nine lines and nothing else; `3ad9f80b29` restores them byte-for-byte (see `git diff 929e6e9eba 3ad9f80b29`). The helper `zombie_arm_rotation` and the flags stay in place (reason (b)-like: the kept `animation_tests` call them, and deleting a `pub(super)` helper that tests name would stop the control from compiling); the control's arm map then carries only the walk swing, which is the row's symptom |
| the rest of `43362918` / `83994480` / `ebe56023` | — | OMITTED — (a) other rows' behaviour, or (b) API the kept client unit refs call; `CM1-REVERSIBILITY.md` records that both bulk commits' other presentation hunks no longer reverse at HEAD |

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
1004.5)` yaw 90, after 240 ticks so the idle bob has settled. With the pose the window holds the
raised forearm (a measured 1802 skin pixels on the candidate); with hanging arms the window holds
only the sky/grass behind the torso, so the row's own assertion fails. The unit ref
`client:krasnokamen-client:entities::animation_tests::zombie_arm_rotation_matches_the_26_3_animate_zombie_arms_formula`
covers the formula and stays green on the candidate.
