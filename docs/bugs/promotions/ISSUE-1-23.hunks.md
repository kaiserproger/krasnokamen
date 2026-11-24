# ISSUE-1-23 — hunk map

Row: `ISSUE-1-23` (client, major) — villager and witch held items are not rendered.
Refs: `client:krasnokamen-client:entities::animation_tests::crossed_arms_rigs_emit_the_held_item_at_the_source_anchor`,
`harness:scene-shot:issue1-23-held-items` (proving assertion `emerald_held_pixels`),
`harness:scene-shot:issue1-23-witch-potion` (the original complaint: a witch holding a potion).

## Real fix and its pre-image

Client `aa20d301` ("fix(client): villager-family held items attach at the crossed arms / witch nose
like the source layers", 2026-10-19) adds a `crossed_arms` role (and the witch's `nose` role) to the
villager, wandering-trader and witch models, the `CrossedArmsItemLayer`/`WitchItemLayer` attachment
in `presentation.rs` (including the nose tilt while an item is held), the role enum/parse in
`resources.rs`/`solaris_models.rs`, and a test-only model helper in `items.rs`. Its pre-image is the
client before that commit (`git show aa20d301^:assets/solaris/models/entity/villager.json`): the
villager, wandering trader and witch expose only `head`, `right_leg` and `left_leg`, so
`append_equipment` takes the anatomical-hand path, finds no `RightArm`/`LeftArm` bone, records
`rig has no matching equipment attachment`, and nothing is drawn — the row's symptom.

The client predates the rename, and the fix's model paths moved from `assets/solaris/...` to
`assets/krasnokamen/...`, so the control is a committed reversal at HEAD whose a/-side is the
commit's own role lines:

- reversal `27a3806c616a44a7975f177df34cdcedcd44e9a5` — the model files lose the `crossed_arms`
  role (the witch also `nose`); the rig has no arm role, so the held item is dropped.
- re-land `586b2d07d2943638d92854918d94ee74c83d5fde` — the same lines restored; row `fix_commit`.
- `git diff 27a3806c61 586b2d07d2` is exactly the model files' role lines below (4 files, 5 added and
  4 removed lines).

The earlier assigned reversal `40ca5591` was inert (it removed a guard after the drawing branch) and
is not used; the previous lane read ("no discrete fix commit exists") came from `git log` without
`-S crossed_arms`, which misses the fix because the file was renamed.

## The pair's four model files: three from aa20d301, one second origin

The pair carries four model hunks while `aa20d301` changes three. Blob ids prove the provenance
(the pair's and the original commit's index lines are the same):

| pair file (`assets/krasnokamen/...`) | pair blob | aa20d301 hunk | class |
|---|---|---|---|
| `villager.json` | `55c54747..03ac3ff7` | `@@ -5,7 +5,7` — `"left_leg": ["left_leg"], "crossed_arms": ["arms"]` | the real fix, byte-identical |
| `wandering_trader.json` | `b373ee54..301ac325` | `@@ -5,7 +5,7` — the same line | the real fix, byte-identical |
| `witch.json` | `c5b04c60..d7fff456` | `@@ -5,7 +5,7` — `"left_leg": ["left_leg"], "crossed_arms": ["arms"], "nose": ["nose"]` | the real fix, byte-identical |
| `villager_baby.json` | `2e999927..20f1e53c` | none — `aa20d301` never touches the baby | **second origin**: the b-side blob `20f1e53c` is exactly `adf5153c`'s product blob for this file (`git show adf5153c` model hunk `assets/solaris/models/entity/villager_baby.json` lines 1137-1160, the `+    "crossed_arms": ["arms"]` line). The a-side `2e999927` is the rebranded file where the old `right_arm`/`left_arm` roles were already dropped by the merge, so this hunk re-applies adf5153c's role line to that state rather than reversing `adf5153c`'s whole hunk |

The baby model is included because the pair's product must be the integrated HEAD model set (the
`Issue1Client` WIP also fixed the baby). It renders nothing in this scenario — the fixture spawns an
adult villager and a witch — so it neither helps nor hides the asserted pixels; its pair deletion is
the same single `crossed_arms` role, and its origin is `adf5153c`, not `aa20d301`.

## Every other hunk of aa20d301 and why it stays on both sides

| hunk | file | HEAD equivalent | reason |
|---|---|---|---|
| `@@ -716,7 +716,12` | `src/entities/presentation.rs` | `BoneRole::Nose if e.equipment_value(0).is_some()` at `src/entities/presentation.rs:2433` | (b) the witch's **nose tilt while the main hand holds an item** (source `WitchModel.setupAnim`: pivot `(0,1,-1.5)/16`, `rotation.x = -0.9`); kept on both sides. It is not a leg-swing or step edit — the villager family's half-amplitude leg swing comes from `97e988d2`, outside this fix |
| `@@ -872,7 +877,24` | `src/entities/presentation.rs` | `crossed_arms_attachment` and `witch_potion_attachment` (`:2917-2935`) | (b) the kept helpers the attachment block calls; without the model role the block never runs |
| `@@ -903,6 +925,50` | `src/entities/presentation.rs` | the `CrossedArms`/`Nose` attachment block `:2980-3027` (`display: "ground"`, `item_matrix(model, "ground")`, `append_baked_stack`) | (b) the kept code the reversed tree compiles against; with no role it emits no pixel in either direction |
| `@@ -108,6 +108,10` | `src/entities/resources.rs` | `BoneRole::CrossedArms`/`Nose` `:623-626` | (b) the enum the kept presentation code matches on |
| `@@ -55,6 +55,8`, `@@ -174,6 +176,8` | `src/entities/solaris_models.rs` (HEAD `krasnokamen_models.rs`) | `Roles.crossed_arms` `:60` and the role mapping `:182` | (b) parsed only when the model names the role |
| `@@ -567,3 +567,69` | `src/entities/animation_tests.rs` | `crossed_arms_rigs_emit_the_held_item_at_the_source_anchor` `:1824` | test hunk (the client ref above); kept unchanged on both sides |
| `@@ -68,6 +68,15` | `src/ui/items.rs` | the `#[cfg(test)] insert_test_model` helper | test hunk: a fixture constructor inside a `#[cfg(test)]` item, not product code. The ground display matrix the fix uses lives in `presentation.rs` (`item_matrix(model, "ground")`), not here — the previous version of this map said otherwise |
| `@@ -481,6 +481,16`, `@@ -39,7 +39,9` | `docs/AGENT_TESTING.md`, `docs/ENTITY_MODELS.md` | the same sections at HEAD | (c) documentation |

No line of the pair is invented: the reversal restores the commit's own a/-side text at HEAD (the
three models verbatim) plus the second origin's role line for the baby.

## Gate

`scene-shot` scenario `issue1-23-held-items` (`tools/harness/issue1_23_held_item_scene.json`): a
native windowed client on a grass platform with the day clock and weather held; a villager at
(999.5, 160, 1003.5) and a witch at (1001.5, 160, 1003.5), both `NoAI` and facing the camera; the
operator world equips them through the ordinary command path (`/item replace entity … weapon.mainhand
with minecraft:emerald` and `… minecraft:redstone`), the scene waits for both received entities, then
captures one frame and counts emerald-green pixels at the villager's crossed arms and redstone-red
pixels at the witch's. The witch's hand is then replaced with `minecraft:potion` and a second frame
counts the potion at the nose.

- `emerald_held_pixels`, `redstone_held_pixels` and `potion_nose_pixels` require the colours inside
  the fixed windows; a pre-image client draws none of them. The proving assertion is
  `emerald_held_pixels`.
- Measured on the committed pair (server pin `54a4a124a7e7`, core `54a4a124a7`; both receipts on
  that candidate tree): candidate `586b2d07d2` emerald 2011 px and redstone 2148 px, all assertions
  true; control `27a3806c61` emerald **0 px** and `emerald_held_pixels` the only false assertion (the
  run aborts there, so the later windows are not reached). Receipts:
  `.analysis/validation/20261124T170135-issue1-23-held-items-xr7fmmk5` (candidate) and
  `.analysis/validation/20261124T170238-issue1-23-held-items-k5h_puso` (control).

Because that abort happens before the potion frame on the control, the potion-at-the-nose claim is its
own scenario, `issue1-23-witch-potion` (`tools/harness/issue1_23_witch_potion_scene.json`): one NoAI
witch with `minecraft:potion` in the main hand, one frame, `potion_nose_pixels` inside the fixed nose
window. Both pins run it, so the original complaint (a witch holding a potion with nothing drawn) has
its own green and its own red.

POTION_MEASUREMENTS_PLACEHOLDER
