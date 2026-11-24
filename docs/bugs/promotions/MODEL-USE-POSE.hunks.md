# MODEL-USE-POSE — hunk map for the tip reversal pair

Promotion receipt: `promotions/MODEL-USE-POSE.json` (written by `ledger promote`).
Client pair: reversal `client:250f7822a5` → re-land `client:2563ce9514` (the row's `fix_commit`).
Both pins clean (`250f7822a53d`, `2563ce9514b6`); the pair's net tree is client `main` `530556e8`
(the re-land's tree equals the lane tip `3cf928ed`, whose tree is `530556e8`).

## The commit, and what it did on this path

`client:83994480` ("feat(client): integrate ClientNext slices on RC5", 2026-11-09) replaced the
pre-fix third-person use pose — one generic raise of the using hand — with a per-item table:
`enum ArmAction`, `fn use_action` (Bow, CrossbowCharge/Hold, Trident, Spear, Spyglass, Horn, Brush,
Block, Item) and `fn equipped_arm_rotations` (the per-action arm angles, the charged-crossbow branch
and the two-handed bow/crossbow block). `evaluate_rig` reaches it through the unified
`BoneRole::RightArm | BoneRole::LeftArm` arm, which reads the `arms` map. Pre-fix the same bones did
`if using && flags & 2 == 0 { rotation.x = -1.35 }` (right) / `!= 0` (left) — the row's "collapse to
one" (`REPORT.md` §1 of the row's source).

## Hunks carried by the pair

| commit | file | original hunk | content | disposition |
|---|---|---|---|---|
| `83994480` | `src/entities/presentation.rs` | `@@ -1097,10 +1125,223 @@ pub(super) fn draw_entity` (the block adding `held_item_use`, `blocking_arm`, `use_action`, `ArmAction`, `equipped_arm_rotations`) | the row's part is `use_action`'s per-item match, the per-action arms and the two-handed block inside `equipped_arm_rotations` | SPLIT + REVERSED — the control deletes the per-item match, the charged-crossbow branch and the two-handed block and restores the pre-image `-1.35` raise; the re-land restores them. `held_item_use` and `blocking_arm` stay (see the omission table) |
| `83994480` | `src/entities/presentation.rs` | `@@ -1174,21 +1422,20 @@ pub(super) fn evaluate_rig` | the `BoneRole::RightArm`/`BoneRole::LeftArm` blocks become `rotation += arms[arm]` with the spell-casting override | (b)/(c): the kept `arms` map is read by the spell-casting block and by the guard rows' code, so the unified arm stays; its `-` side's `-1.35` lines are the control's own pre-image text |

## a/-side adaptation

- `rotation.x = -1.35` → `arms[arm].x = -1.35`: the pre-image's own literal (from the
  `@@ -1174,21 +1422,20 @@` hunk's `-` side) with the target the kept structure exposes — the
  "instrumented kept type / surviving half of a mixed hunk" class.
- `content` → `_content` on `equipped_arm_rotations`: the kept signature (callers in `evaluate_rig`
  and the kept animation tests) with no reader in the control — the `_`-parameter rename class.
- The prelude `arms[arm].x = arms[arm].x * 0.5 - 18°` stays in both trees (it is the same commit's
  line but not the row's symptom; the scene compares frames within one tree, so its constant offset
  cancels).

## Omitted hunks of `83994480` on this path

| hunk | reason |
|---|---|
| `@@ -1126,8 +1367,11 @@ pub(super) fn evaluate_rig` (`flags`/`using` → `head`/`age`/`arms`) | (c) later rows' kept code reads these locals (the wing flap uses `age`, the guard rows and spell casting use `arms`); reversing them stops the control from compiling |
| `@@ -1148,10 +1392,14 @@` (wing flap by `age`) | (a) MODEL-CHICKEN-WINGS' behaviour |
| `@@ -1198,19 +1445,30 @@` (zombie arm pose) | (a) MODEL-ZOMBIE-ARMS, promoted on its own pair |
| the other 39 `presentation.rs` hunks and every other file of the commit | (a) other rows' behaviour; `CM1-REVERSIBILITY.md` records per file whether the hunk still reverses |

## Pair-only lines (measured 2026-11-25)

`git diff 250f7822a5 2563ce9514 -U0` in the client repo, compared line by line with
`git show 83994480 -- src/entities/presentation.rs`: 114 changed lines, 59 not byte-equal to an
origin line, of which 3 are indentation-only → 56 real, in two groups (blame at the pair's net tree,
`git blame -C -C -L 1830,1960 530556e8 -- src/entities/presentation.rs`):

| pair lines | blame at `530556e8` | class |
|---|---|---|
| 6 `-` lines: `_content: &Content`, `_stack`, the two pre-image comment lines, `if using && active == hand {`, `arms[arm].x = -1.35;` | the control (`250f7822a5`) for the two `_` renames, `83994480`'s `-` side for the rest | (c)/adaptation: the renames are the accepted `_`-parameter/kept-binding class above; the `-1.35` condition and literal are the pre-image's own text |
| 50 `+` lines: the spears' `let ticks`/`let delay`/`let raised` blocks, the spyglass's wrapped `arms[arm].x = (head.x … )` and the two-handed `Vec3::new(…)` literals | `9a645aa99` (rustfmt reflow); the remaining literals such as `.map_or(0., \|s\| s.item_use_ticks(partial));` and `arms[arm].y = …` stay `83994480` | (c) the 2026-11-14 sync rewrapped the origin's compact expressions; no line changes behaviour |

No pair-only line introduces behaviour the pre-image did not have, so no second fix commit joins
the map.

## Proving ref

`harness:scene-shot:native-use-pose` → `trident_use_pose_raised` (and `use_poses_differ_by_item`)
(`tools/harness/backends/scene_shot_checks.py::check_use_pose`). One own avatar in third person on a
stone pad, one camera; a bow, a spyglass and a trident are held idle and with `use` held
(`use_phases`), so the frames are the same client, camera and lighting. The trident's pose is `-PI`
(overhead), which leaves the torso silhouette beside/above the head; the boxes there hold only sky
while the arm hangs. Measured on the promotion receipts (core `ff3d2ba8b0`, server pin
`ff3d2ba8b046`):

- control `client:250f7822a5` → `20261124T180348-native-use-pose-dbo4amf1` **failed at
  `trident_use_pose_raised`** (above-shoulder change 42 left / 101 right against the 800 floor);
  the drawn-bow vs using-spyglass count read 531, below its own 2500 floor, though that assertion is
  not reached because the run stops at the first failure;
- re-land `client:2563ce9514` → `20261124T180418-native-use-pose-xiunrgic` passed: trident change
  84/1715 (right 1715 > 800), drawn-bow vs using-spyglass 5131 (> 2500).

Unit refs: `client:krasnokamen-client:entities::animation_tests::animation_aimed_and_single_hand_use_actions_have_distinct_silhouettes`
and `::animation_drowned_trident_pose_takes_precedence_on_the_actual_main_arm` — both red on the
control (every action collapses to `-1.35`) and green on the candidate, as the promote's
`candidate_refs` records.
