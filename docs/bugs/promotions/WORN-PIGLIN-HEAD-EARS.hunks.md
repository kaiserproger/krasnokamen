# WORN-PIGLIN-HEAD-EARS — hunk map for the pinned pair

Promotion receipt: `promotions/WORN-PIGLIN-HEAD-EARS.json` (written by `ledger promote`).
Client pair: control `client:8a7817c8` (the fix commit's parent, already pinned before this pack) →
restore `client:78fb611f` (the row's `fix_commit`). Both pins clean.

## Why the pair, and what the commit contains

`client:78fb611f` ("Fix skull models and resolve standalone head profile textures", 2026-11-15)
closes the two worn-skull rows and the head-profile-texture row in one commit. For this row the two
production parts are:

- `assets/krasnokamen/models/entity/piglin_head.json`: the pre-fix rig gave the head two separate
  bones (`right_ear`/`left_ear`, pivots ±4, 3x5x1 horizontal cubes at the head's mid-height); the
  fix folds the flaps into the head bone (a 10-wide head cube plus 2x2x4 nostril cubes) and drops
  the ear bones;
- `src/entities/presentation.rs` (`special_item`): the static item/worn head now runs
  `render_blocks::skull_pose(&bone.name, 0., &mut rotation)` — "SkullModelBase.setupAnim also runs
  for static item/worn heads: animation position zero still opens the jaw and lowers both ears".
  That is what angles the merged flaps down.

## Hunks carried by the pair (control -> restore)

`git diff 8a7817c8 78fb611f -U0` = 995 changed lines across 14 files; the pair is the fix's own
parent, so every non-test hunk of the commit is carried. The hunks this row's assertion depends on:

| file | hunk | role |
|---|---|---|
| `assets/krasnokamen/models/entity/piglin_head.json` | the whole file rewrite | the row's subject: separate horizontal ear bones -> flap geometry inside the head |
| `src/entities/presentation.rs` | `@@ -3386,8 +3385,14 @@ pub fn special_item` | applies `skull_pose` to the static/worn head rig; lowers the ears and opens the jaw |

## Hunks carried for other rows (class (a))

| file | hunk | disposition |
|---|---|---|
| `assets/krasnokamen/models/entity/dragon_skull.json` | whole file rewrite | (a) WORN-DRAGON-HEAD-GEOMETRY's subject (open, needs its own fixture); inert for the piglin ear strips |
| `src/entities/presentation.rs` | `@@ -3266,11 +3266,10 @@` (`head_skin(content, profiles, profile)`, no cross-player borrowing) | (a) the head-profile rows (`MODEL-PLAYER-HEAD-PROFILE`, `PLAYER-HEAD-UNLISTED-PROFILE-TEXTURE`) |
| `src/entities.rs`, `src/entities/render_blocks.rs`, `src/entities/profiles.rs`, `src/profile_textures.rs`, `src/profile_textures/heads.rs`, `src/protocol/content.rs`, `src/session.rs`, `src/session/profile_textures.rs` | the rest of the profile-texture plumbing | (a) the same profile rows |
| `docs/AGENT_TESTING.md` | doc | not production code |
| `src/entities/render_blocks/village_tests.rs`, `src/profile_textures/tests.rs` | test files | excluded from the non-test patch; `village_tests`' `skull_parity_dragon_model_and_static_jaw` / `skull_parity_piglin_ear_geometry_and_static_pose` pin the same geometry and stay green on the candidate |

Because the pair is the credited commit against its own parent, the control->restore delta equals
the commit's non-test patch exactly (`ledger.py:_check_client_control_delta` recomputes the patch-id
from the re-land commit, which is this commit). Nothing is hand-re-broken and no a/-side text is
invented; the only other-row content in the pair is class (a) and its refs are not run in this
row's control.

## Proving ref

`harness:scene-shot:native-worn-head` → `worn_piglin_ears_angle_down`: brown ear pixels in the two
strips of air beside the piglin head cube between the eye line and the jaw (boxes
`(450,300,560,380)` and `(720,300,830,380)`). Measured on the pinned pair at the same camera and
subject: control `8a7817c8` **542** px (horizontal ear boxes end at the head's mid-line), fix
`78fb611f` **3859** px (the flaps hang to the jaw). Floor 1500 = 2.8x above the control and 2.6x
below the fix. `worn_piglin_head_visible` (>= 200 px) only proves the frame shows the head.
