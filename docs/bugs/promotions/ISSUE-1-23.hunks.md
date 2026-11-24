# ISSUE-1-23 — hunk map

Row: `ISSUE-1-23` (client, major) — villager and witch held items are not rendered.
Refs: `client:krasnokamen-client:entities::animation_tests::crossed_arms_rigs_emit_the_held_item_at_the_source_anchor`,
`harness:scene-shot:issue1-23-held-items` (proving assertion `emerald_held_pixels`).

## Real fix and its pre-image

Client `aa20d301` ("fix(client): villager-family held items attach at the crossed arms / witch nose
like the source layers", 2026-10-19) adds a `crossed_arms` role (and the witch's `nose` role) to the
villager-family models and the `CrossedArmsItemLayer`/`WitchItemLayer` attachment in
`presentation.rs`, with the role enum/parse in `resources.rs`/`solaris_models.rs` and the `ground`
display matrix in `items.rs`. Its pre-image is the client before that commit
(`git show aa20d301^:assets/solaris/models/entity/villager.json`): the villager, baby villager,
wandering trader and witch expose only `head`, `right_leg` and `left_leg`, so `append_equipment`
takes the anatomical-hand path, finds no `RightArm`/`LeftArm` bone, records
`rig has no matching equipment attachment`, and nothing is drawn — the row's symptom.

The client predates the rename, and the fix's model paths moved from `assets/solaris/...` to
`assets/krasnokamen/...`, so the control is a committed reversal at HEAD whose a/-side is the
commit's own role lines, not a hand-written skip:

- reversal `27a3806c616a44a7975f177df34cdcedcd44e9a5` — the four model files lose the `crossed_arms`
  role (the witch also `nose`); the rig has no arm role, so the held item is dropped.
- re-land `586b2d07d2943638d92854918d94ee74c83d5fde` — the same lines restored; row `fix_commit`.
- `git diff 27a3806c61 586b2d07d2` is exactly the re-land's own non-test patch (4 files, the role
  lines).

The earlier assigned reversal `40ca5591` was inert (it removed a guard after the drawing branch) and
is not used; the previous lane read ("no discrete fix commit exists") came from `git log` without
`-S crossed_arms`, which misses the fix because the file was renamed.

## Every other hunk of aa20d301 and why it stays on both sides

| file | hunk | reason |
|---|---|---|
| `src/entities/presentation.rs` | `@@ -716` leg-swing half amplitude for the villager family | (b) the kept rig code; it changes gait, not held items, and reversing it would turn another behaviour's frames red |
| `src/entities/presentation.rs` | `@@ -872`, `@@ -903` the `CrossedArms`/`Nose` attachment block and the two helpers | (b) the kept code the reversed tree compiles against; without the model role the block never runs, so it emits no pixel in either direction |
| `src/entities/resources.rs` | `BoneRole::CrossedArms`/`Nose` variants and their doc lines | (b) the enum the kept presentation code matches on |
| `src/entities/solaris_models.rs` (HEAD: `krasnokamen_models.rs`) | the role name → field mapping | (b) same: parsed only when the model names the role |
| `src/ui/items.rs` | the `ground` display-matrix helper | (b) used by the kept attachment code |
| `src/entities/animation_tests.rs` | the unit test | test hunk (the client ref above); kept unchanged on both sides |
| `docs/AGENT_TESTING.md`, `docs/ENTITY_MODELS.md` | documentation | (c) docs |

No line of the pair is invented: the reversal restores the commit's own a/-side text at HEAD.

## Gate

`scene-shot` scenario `issue1-23-held-items` (`tools/harness/issue1_23_held_item_scene.json`): a
native windowed client on a grass platform with the day clock and weather held; a villager at
(999.5, 160, 1003.5) and a witch at (1001.5, 160, 1003.5), both `NoAI` and facing the camera; the
operator world equips them through the ordinary command path (`/item replace entity … weapon.mainhand
with minecraft:emerald` and `… minecraft:redstone`), the scene waits for both received entities, then
captures one frame and counts emerald-green pixels at the villager's crossed arms and redstone-red
pixels at the witch's.

- `emerald_held_pixels` and `redstone_held_pixels` require the colours inside the fixed windows;
  a pre-image client draws neither item. The proving assertion is `emerald_held_pixels`.
- Counts, windows and receipts are recorded here after the candidate/control runs.
