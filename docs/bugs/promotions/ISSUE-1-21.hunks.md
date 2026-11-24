# ISSUE-1-21 — hunk map

Row: `ISSUE-1-21` (client, major) — the first-person tool clips through a block.
Ref: `harness:scene-shot:issue1-21-tool-clip` (proving assertion `tool_wall_visible`; the scene also
requires `tool_hand_visible` at range on both pins).

## Real fix, its pre-image, and where the same code sits at HEAD

The real fix is the Stage3 `Issue1Client` hand-pass patch
(`receipts/Stage3/S3Issue1/clip-fix-product.patch`). The patch's own index lines identify its product
tree exactly: `src/api.rs` `c093a860..4d07ddb7`, `src/player/visual.rs` `10d8f186..2a3a7979`,
`src/render.rs` `8e30b9c8..41b0d11d`, `src/render/frame.rs` `524f9c26..b1f503a3`,
`src/render/residency.rs` `a91c10b6..8cf95e3a` — that is `adf5153c` ("wip(stage0): Issue1Client
uncommitted work at the 21:00 stop"), so `adf5153c` is the fix's *product* commit, not its pre-image.

**The pre-image is `2edbb590` (`adf5153c^`)**: there `append_first_person` submits the bare arm and
the held-item quads to `scene.dynamic` (the a/-side of the patch's two `visual.rs` hunks), so the hand
is world geometry and the wall tests its depth. The first version of this map named `adf5153c` itself
as the pre-image; that was wrong — `adf5153c`'s public patch already *adds* the field and the two
submission changes (`clip-fix-product.patch:5-14` adds `pub hand: DynamicGeometry`, `:23` and
`:32` move the arm and the item quads), and `scene.dynamic` exists in its parent.

At HEAD the same separation lives under the integrated names: `Scene.first_person`
(`src/api.rs:460`), `Renderer.first_person` (`src/render.rs:422`), `targets.first_person_depth_view`
(`src/render.rs:205`), and the hand pass labelled `first person, independent world depth`
(`src/render/frame.rs:1202-1256`).

The integration history is wider than the Stage3 branch: `43362918` ("Integrate Solaris client rc4
candidate", 2026-11-03) carries the same product change as `first_person`, extended in two ways — the
held map and the item-foil marking also submit to that stream, and the hand pass gets its own depth
attachment (`first_person_depth_view`, `StoreOp::Discard`) instead of clearing the scene depth view.
`adf5153c` is a duplicate WIP on the `Issue1Client` branch (it spells the field `hand`); the merge
`62adc12f` ("merge(stage0): Issue1Client") reconciled the duplicate to `first_person`, and HEAD has
no `hand`. Both `43362918` and `adf5153c` are ancestors of the pair, which is why the control is a
committed reversal at HEAD rather than a checkout.

## The pair, line by line

`git diff e4a470fb7c 3c3d9fba9a` is exactly seven lines in `src/player/visual.rs` and nothing else.
Every line, its source and its class (a *moved* line is the fix's own a/-side text at a call site a
later commit restructured; a *later* line belongs to `43362918`'s extension of the same change):

| HEAD line | a/-side (reversal) | b/-side (re-land) | source of the line | class |
|---|---|---|---|---|
| 518 | `&mut scene.dynamic` | `&mut scene.first_person` | the Stage3 arm submission (`clip-fix-product.patch:19-27`; `append_local_arm … &mut scene.dynamic` at `2edbb590:src/player/visual.rs:401`) | moved: `43362918` turned the single arm call into the held-map `[true, false]` arms loop, and `9a645aa99` (2026-11-14 formatting carrier) split the call across lines |
| 531 | `&mut scene.dynamic` | `&mut scene.first_person` | the same Stage3 arm submission | later: the `else` arm of `43362918`'s map pose; it already submitted to `first_person`, and the pair restores that same stream for the same arm geometry |
| 539 | `&mut scene.dynamic` | `&mut scene.first_person` | `43362918:src/player/visual.rs:443` (`append_held_map`) | later: no Stage3 a/-side, the held map did not exist in the Stage3 patch |
| 585 | `&mut scene.dynamic` | `&mut scene.first_person` | the Stage3 arm submission (`clip-fix-product.patch:19-27`) | moved: the bare-arm fallback; `9a645aa99` reformatted the call, the target is the patch's own a/-side |
| 606 | `scene.dynamic.vertices.len()` | `scene.first_person.vertices.len()` | `43362918:src/player/visual.rs:503` | later: the foil-coverage start; no Stage3 a/-side |
| 622 | `&mut scene.dynamic` | `&mut scene.first_person` | the Stage3 item submission (`clip-fix-product.patch:28-36`; `presentation_emit … &mut scene.dynamic` at `2edbb590:src/player/visual.rs:430`) | moved: `9a645aa99` split the four-quad call, the target is the patch's own a/-side |
| 631 | `&mut scene.dynamic.vertices[first..]` | `&mut scene.first_person.vertices[first..]` | `43362918:src/player/visual.rs:523` (`mark_foil`) | later: marks the same item quads line 622 submits, so it must follow their stream |

No line is invented: the three arm/item sites restore the Stage3 patch's own a/-side text, the held
map and the two foil lines restore the world-geometry submission their introducing commit `43362918`
had already replaced with `first_person`. Read right to left, the pair is the forward-ported fix at
HEAD — the Stage3 patch's two submission places plus the three later submissions of the same
integrated change, nothing else.

## The hand-geometry reset stays on both sides

The Stage3 patch also clears the hand geometry together with the world geometry (`git show
adf5153c` lines 2125-2131, and the scene-wide reset at `:2086`: `self.scene.hand.vertices.clear()` and every
`self.scene.hand.indices` cleared beside `scene.dynamic`). At HEAD the same reset is
`self.scene.first_person.vertices.clear()` plus the per-family indices inside `if dynamic`
(`src/session/observation.rs:869-872`, from `dd36acd57`, 2026-11-13), and the scene-wide
`self.scene.first_person = Default::default()` on disconnect/dimension change (`src/session.rs:773`,
from `43362918`). The reversal keeps both: an uncleared stream would grow instead of testing
clipping, and they are identical on both pins, so they are not part of the pair.

## Every other hunk of the real fix and why it stays on both sides

| patch hunk | file | HEAD equivalent | reason |
|---|---|---|---|
| `@@ -437,6 +437,9` | `src/api.rs` | `Scene.first_person` (`src/api.rs:460`) | (b) the kept pass, residency and upload read it; present on both sides and carries no geometry in the reversed tree |
| `@@ -398,7 +398,7`, `@@ -427,7 +427,7` | `src/player/visual.rs` | the seven pair lines above | in the pair (see the line table) |
| `@@ -363,6 +363,7`, `@@ -672,6 +673,7`, `@@ -743,6 +745,7` | `src/render.rs` | `Renderer.first_person` stream `:422`, its construction `:802`, its field init `:891` | (b) plumbing the kept pass and residency call; an empty stream draws nothing |
| `@@ -1315,6 +1318,7` | `src/render.rs` | `metadata["gait_frame"]` (`src/render.rs:1582`) | (a) the gait capture is another row's observation; it changes no frame and is kept on both sides |
| `@@ -847,6 +847,49` | `src/render/frame.rs` | the hand pass `first person, independent world depth` (`src/render/frame.rs:1202-1256`) | (b) with no hand geometry `first_person.ranges` is all empty and the pass is skipped, so it emits no pixel in either direction; at HEAD it uses its own depth attachment, the `43362918` form of the same fence |
| `@@ -192,14 +192,15`, `@@ -239,12 +240,14`, `@@ -387,6 +390,13` | `src/render/residency.rs` | directional/portal scans, size validation, upload and lightmap chain fold `scene.first_person` (`src/render/residency.rs:195,215,253-254,270,424-428`) | (b) the same kept API; an empty stream validates and uploads nothing |

`43362918`'s own additional plumbing — `first_person_depth_view`, `first_person_camera` and its
bind group, `pipelines.first_person`, `render/first_person_tests.rs` — is part of the same integrated
change, exists on both pins, and is inert while the stream is empty: class (b), not a later commit.

## Gate

`scene-shot` scenario `issue1-21-tool-clip` (`tools/harness/tool_clipping_scene.json`): a diamond
pickaxe against an obsidian wall, one frame 2.5 blocks away and one at 0.6 blocks, attack frames kept
as unasserted evidence.

- `tool_hand_visible` (`tool-far`, 2.0–3.2 blocks) requires the received obsidian target, the pickaxe
  and turquoise tool pixels in the fixed hand window. It must pass on **both** pins: the pre-image
  renders the hand in world geometry and is not occluded at that range.
- `tool_wall_visible` (`tool-rest`, 0.6 blocks) requires the same fixture plus a dark wall behind the
  tool and at least 500 turquoise tool pixels; the world-geometry control draws the tool behind the
  wall. It is the row's proving assertion.

Measured on the committed pair (server pin `f1e61cb286a2`, core `f1e61cb286`; both receipts on that
candidate tree):

| pin | `tool_hand_visible` (2.5 blocks) | `tool_wall_visible` (0.5 blocks) |
|---|---|---|
| candidate `3c3d9fba9a` | tool 5526 px, wall 8695, distance 2.5 — PASS | tool 5526 px, wall 15459, distance 0.5 — PASS |
| control `e4a470fb7c` (pre-image) | tool 5526 px, wall 8695, distance 2.5 — PASS | tool 0 px, wall 16697, distance 0.5 — **FAIL** |

So the pick is visible away from the wall on both pins and only the close frame differs: the control
draws the tool behind the wall, the fix in front of it. Receipts: candidate
`.analysis/validation/20261124T162258-issue1-21-tool-clip-ujxwerro`, control
`.analysis/validation/20261124T162352-issue1-21-tool-clip-gcr1cj_q`.
