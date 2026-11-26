# MODEL-ITEM-POSE — hunk map for the committed reversal pair

Promotion: `docs/bugs/promotions/MODEL-ITEM-POSE.json` (written by `ledger promote`).
Scene: `native-item-pose` (`tools/harness/item_pose_scene.json`), proving assertion
`statue_pose_changes_layer`.

The real fix `ad0fd066` predates the client rename (`f0a6abf6`, 2026-11-12), so its tree cannot
build a pre-fix client. The control is the committed reversal on the current tip, and the
re-landing commit is the row's `fix_commit`:

| half | client commit | content |
|---|---|---|
| control | `client:c11d48b7` (`wt/D123PP3ItemPoseCtl`) | the real pre-image of this row's two hunks: `special_item` renders every statue as `minecraft:copper_golem#main` and the item root has no `copper_golem_statue` branch (the statue's feet-at-0 half-turn) |
| restore (= `fix_commit`) | `client:2668619b` | the re-landing; `git diff c11d48b7 2668619b` is exactly this row's forward-ported fix (the pose match plus the root transform) |

## Hunk map of `ad0fd066`

| original hunk | HEAD equivalent | kept? |
|---|---|---|
| `presentation.rs` @@ -2362,8 +2362,16 @@ `special_item`: the `copper_golem_statue` pose match (`standing`/`running`/`sitting`/`star`) | `presentation.rs:3463` (`git blame -C -C` names `ad0fd066`; the arm text lost its comment but kept the `unwrap_or("standing")` default) | **yes — this row's fix** |
| `presentation.rs` @@ -2438,6 +2446,11 @@ `special_item`: the `item_root` branch translating the statue to its feet and half-turning it about Z | `presentation.rs:3555`, text unchanged | **yes — this row's fix** |
| `player/visual.rs` (offhand display fallbacks, `swap_animation_scale`, one-handed map arm) | later rewrites of the first-person item path | (a) other rows' behaviour (held-item display contexts, swap scale, map arm) |
| `src/session/block_effects.rs` (potion impact spell particles) | later rewrites | (a) other rows' behaviour (splash potion impacts) |
| `src/ui/items.rs` (display-context fallbacks) | later rewrites | (a) other rows' behaviour |
| `src/protocol/models.rs` (display transforms) | later rewrites | (a) other rows' behaviour |
| `src/entities/render_blocks/village_tests.rs` (statue scene-pass test) | test file | excluded: a test hunk |
| `tools/held_items_scene.json`, `tools/held_items_oracle.py` | test tooling | excluded: test tooling, not production |
| `docs/AGENT_TESTING.md` | doc file | excluded: not production |

The a/-side of the control is the real pre-image (the pose match and the statue root branch did not
exist before `ad0fd066`); the two kept hunks are the ones whose behaviour the row names. The 7
spear `swap_animation_scale` items are the *same commit's other symptom* and stay open under their
own row, which is why their hunks are (a)-omitted here.
