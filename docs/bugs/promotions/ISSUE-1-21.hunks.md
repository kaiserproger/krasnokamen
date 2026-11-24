# ISSUE-1-21 — hunk map

Row: `ISSUE-1-21` (client, major) — the first-person tool clips through a block.
Ref: `harness:scene-shot:issue1-21-tool-clip` (proving assertion `tool_wall_visible`; the scene also
requires `tool_hand_visible` at range on both pins).

## Real fix and its pre-image

The real fix is the Stage3 `Issue1Client` hand-pass patch
(`receipts/Stage3/S3Issue1/clip-fix-product.patch`), integrated into the client history by
`43362918` ("Integrate Solaris client rc4 candidate"): it adds `Scene.first_person`, appends the
first-person arm, held map and held item there instead of `Scene.dynamic`, and draws that geometry in
its own pass after a depth clear. Its pre-image is the client before that patch, where the same
`append_first_person` submitted to `scene.dynamic` — the a/-side lines of the patch at 15-34
(`&mut scene.dynamic` for the arm) and 73-124 (the bare arm and the held-item quads); the historical
tree is `adf5153c` (`wip(stage0): Issue1Client uncommitted work at the 21:00 stop`), whose
`append_first_person` writes through the entity store into the world geometry.

The 26.3 renderer keeps the same separation at HEAD (`Scene.first_person`, `Renderer.first_person`,
`targets.first_person_depth_view`, the `first person, independent world depth` pass), so the control
is a committed reversal at HEAD whose a/-side is the patch's own pre-image lines, not a hand-written
re-break:

- reversal `e4a470fb7c6b168208c1b53e0dc936fc37dc4b11` — every `append_first_person` submission
  (`append_local_arm` ×3, `append_held_map`, `presentation_emit` and the foil marking) targets
  `scene.dynamic` again, so the hand is world geometry and is depth-tested against the wall. The
  separate pass finds `first_person.ranges` empty and is skipped.
- re-land `3c3d9fba9a80b0de906f428815d59bb524075763` — the same lines target `scene.first_person`
  again; row `fix_commit`.
- `git diff e4a470fb7c 3c3d9fba9a` is exactly the re-land's own non-test patch (7 lines).

The earlier pair `6cd212a514`/`de9fde8dd9` is withdrawn: its pre-image loaded world depth into the
*separate* `first_person_depth_view` (`Load`), which has no world content, so the pick disappeared
even 2.5 blocks from the wall and the red was hand visibility, not wall clipping
(`REVIEW-wave2-9` finding 1).

## Every other hunk of the real fix and why it stays on both sides

| scale | file | hunk | reason |
|---|---|---|---|
| patch 1-12 | `src/api.rs` | `Scene.first_person` field | (b) the kept pass (`render/frame.rs`) and the residency upload read it; present on both sides, carrying no geometry in the reversed tree |
| patch 35-72 | `src/player/visual.rs` | the two append targets | **in the pair** (reversal/re-land above); at HEAD the same function has three append targets (arm, held map, item) and the foil marking, all carried |
| patch 78-131 | `src/render.rs` | `Renderer.first_person` stream, its init, the `hand` naming | (b) plumbing the kept residency/pass code calls; an empty stream draws nothing |
| patch 133-176 | `src/render/frame.rs` | the separate hand pass (at HEAD: `first person, independent world depth`, `first_person_camera_bind`, `Clear(1.)`, `StoreOp::Discard`) | (b) with no hand geometry the pass is skipped (`first_person.ranges` all empty), so it emits no pixels in either direction; removing it would delete ~60 lines without changing a frame |
| patch 178-201 | `src/render/residency.rs` | directional-light and portal scans, size validation, upload and the lightmap chain include `scene.first_person` | (b) the same kept API; an empty stream validates and uploads nothing |
| later | `src/render.rs` | `first_person_depth_view`, `first_person_camera` + bind group, `pipelines.first_person`, `render/first_person_tests.rs` | (c) added after the Stage3 patch to keep world depth for post-processing; the reversal keeps them, inert |

No line of the pair is invented: the reversal restores the patch's own a/-side text at HEAD
(`&mut scene.dynamic` in `append_first_person`), and the re-land restores its b/-side text.

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

