# MODEL-ELEMENT-EMISSION — hunk map for the tip reversal pair

Promotion receipt: `promotions/MODEL-ELEMENT-EMISSION.json` (written by `ledger promote`).
Client pair: reversal `client:dde97f91` → re-land `client:3cf928ed` (the row's `fix_commit`).
The pair's net tree is client `main` `530556e8`.

## Original commit

`client:83994480` ("feat(client): integrate ClientNext slices on RC5", 2026-11-09). The row's path is
its element-emission plumbing: `BakedQuad::light_emission`, `BakedQuad::light_with_emission`, the
baker's `element["light_emission"]` read and the three draw sites that apply it. 26.3 models such as
`block/cross_emissive` (parent of `firefly_bush` and `open_eyeblossom`) carry two extra elements with
`light_emission: 15` over a separate emissive texture, so at night their overlay texels must ignore
the sampled world light.

## Hunks carried by the pair

| file | HEAD form | disposition |
|---|---|---|
| `src/terrain/mesher.rs` (`model_quad`, `light: quad.light_with_emission(light[i].light)`) | the terrain quad's light | REVERSED — the control restores `83994480`'s pre-image `light[i].light` (rc-file `@@ -303,7 +320,7 @@`), the re-land restores the call |
| `src/entities/presentation.rs` `@@ -1569,7 +1839,7 @@` (`append_block_quads` → `quad.light_with_emission(light)`) | block quads in the entity pass | REVERSED — control: `light`; re-land: the call |
| `src/entities/presentation.rs` `@@ -1460,7 +1730,7 @@ pub(super) fn append_baked(` (`light_with_emission` on the baked-item quad emit) | item/special quads | REVERSED — control: `light`; re-land: the call |

## Pair-only lines (measured 2026-11-25, review A-B1)

`git diff dde97f91 3cf928ed -U0` in the client repo, compared line by line with
`git show 83994480 -- <path>`: 6 changed lines, 2 not byte-equal to an origin line, and **both are
indentation-only** — zero real deviations. The two are the `-`/`+` sides of
`presentation.rs:2878` (`light,` → `quad.light_with_emission(light),`), and blame at the pair's net
tree (`git blame -C -C -L 2874,2880 530556e8 -- src/entities/presentation.rs`) names
`9a645aa99` — the 2026-11-14 rename/sync re-indented that call site into a struct literal, so the
same expression sits at a different column than in `83994480`, class (c) code rewritten later.
No pair-only line changes behaviour; no gate re-run was needed for this section.

## Why the method, field and baker read are kept (reason (b))

`BakedQuad::light_with_emission` itself, the `light_emission` field and the baker's
`element["light_emission"]` read are the same commit's lines, but reversing them is not this row's
control: the field is read by `ui/items.rs`'s item-cache key (`ae2c4fa68`) and `crowd.rs`'s vertex
emission packing (`a89b5664d`/`9a645aa99`), and the method is called by the kept unit tests
`protocol::models::tests::element_emission_is_baked_per_element_and_bounded` and
`terrain::border_tests::element_emission_stays_on_its_quad_across_chunk_boundaries`. Removing them
would reverse later commits' lines and stop the control from compiling its own suite. The control
therefore keeps the plumbing and removes only the application — exactly the pre-fix draw path — and
the row's own `border_tests` ref reds on the control at the emitted vertex light, which is the
symptom.

## Every emission-path hunk of `83994480`

`git show 83994480 -- src | grep` for `light_emission`/`light_with_emission` finds exactly sixteen
hunks; each is dispositioned here. Everything else in the commit (43 `presentation.rs` hunks,
`render_blocks.rs` (7), `render_blocks/surfaces.rs`, `ui/state.rs` (3), `entities.rs` (2), the rig
assets and the other protocol/UI slices) is (a) another row's behaviour — the commit is the bulk
ClientNext integration, and `CM1-REVERSIBILITY.md`'s addendum records that those files reverse only
3-way with conflicts.

| file | hunk | disposition |
|---|---|---|
| `src/terrain/mesher.rs` | `@@ -303,7 +320,7 @@ fn model_quad(` | CARRIED (the terrain draw site) |
| `src/entities/presentation.rs` | `@@ -1569,7 +1839,7 @@ append_block_quads` | CARRIED (block quads) |
| `src/entities/presentation.rs` | `@@ -1460,7 +1730,7 @@ pub(super) fn append_baked(` | CARRIED (baked-item quads) |
| `src/protocol/models.rs` | `@@ -41,12 +41,23 @@ pub struct BakedQuad` | (b) kept: the field and the `light_with_emission` method the carried call sites and the kept tests use |
| `src/protocol/models.rs` | `@@ -577,6 +588,11 @@ fn bake_elements(` | (b) kept: the baker's `element["light_emission"]` read, asserted by the kept baker test |
| `src/protocol/models.rs` | `@@ -690,6 +706,7 @@ fn bake_elements(` | (b) kept: the element quad literal needs the field; the value is the element's own read |
| `src/protocol/models.rs` | `@@ -799,6 +816,7 @@ fn generated_elements(` | (b) kept: literal `light_emission: 0`; inert |
| `src/entities/presentation.rs` | `@@ -2019,6 +2293,7 @@`, `@@ -2047,6 +2322,7 @@`, `@@ -2091,6 +2367,7 @@ special_item(` | (b) kept: three quad literals with `light_emission: 0`; the field is required to compile and the value is inert |
| `src/protocol/content.rs` | `@@ -2359,6 +2359,7 @@ impl Content` | (b) kept: same literal |
| `src/protocol/models.rs` | `@@ -1236,6 +1254,23 @@`, `@@ -1362,9 +1397,11 @@`, `@@ -1403,9 +1440,12 @@ mod tests` | test hunks: the baker test (`element_emission_is_baked_per_element_and_bounded`, green on both trees because the baker and field are kept) and two fixture literals |
| `src/protocol/models/lighting.rs` | `@@ -208,6 +208,7 @@ mod tests` | test hunk: a fixture literal |
| `src/terrain/border_tests.rs` | `@@ -1144,3 +1144,70 @@` | test hunk: the mesher emission test, which reds on the control at the emitted vertex light (the row's own assertion) |

## Proving ref

`harness:scene-shot:native-element-emission` → `emissive_overlay_full_bright`
(`tools/harness/backends/scene_shot_checks.py::check_element_emission`). One `minecraft:firefly_bush`
stands in a roofed stone cell (sky light 0 inside) at midnight; the HUD is hidden (F1) because the
crosshair is a fixed white cross. `firefly_bush_emissive.png` has 9 texels with every channel above
200 and the base texture has none, so the count of near-white pixels in the bush's region can only be
the emissive overlay. Measured: control **0**, re-land **306** (threshold 60).

The two unit refs: the baker test (emission is baked per element, 0..=15 bounded) is green on both
trees; the mesher test (the emissive quad's vertices carry light 15) is the row's own assertion and
was measured on 2026-11-25 — a detached worktree at the control `dde97f91` fails it at
`src/terrain/border_tests.rs:1330` (`assertion failed: vertices.iter().all(|vertex| vertex.light == expected)`),
and the same test on `3cf928ed` passes (`1 passed`, 1377 filtered).

**Flake observed, its cause, and the fixture fix.** Two runs of the same pinned client read 0
near-white pixels while a `ledger promote`'s cargo test held a build token. The measured cause is
the unknown-light sentinel, not a missing application: the section was meshed while one of the
bush quad's light samples was still `[-1.; 2]` (light packet not yet applied), and
`light_with_emission` deliberately passes that sample through, so the quad's vertices mixed `15`
with `-1`; the interpolated fragments land on intermediate lightmap coordinates and the brightest
texel of the overlay read (178,173,135) / (213,211,178) instead of (254,254,224) — a continuum
between the control's (52,49,40) and the fixed frame, which is exactly why it is a false red rather
than a wrong pass. The scene now removes the race: it lights a torch behind the camera in the same
section, waits for that section's remesh (`wait_terrain`), reads the bush cell with
`minecraft_read_block` (block light 7 is only readable once the client received the section's
block-light layer — the positive control), removes the torch, waits for the remesh again and reads
the cell back (sky 0, block 0). Both reads are asserted in `check_element_emission`
(`bush_cell_block_light_received`, `bush_cell_dark_before_shot`), so the asserted frame is drawn
from a mesh built after the light layer was proven present; the near-white count stays the row's
own assertion, measured 306 on the re-land and 0 on the control.

### Run history of the hardened fixture (core `e665a7d69`, server pin `e665a7d6967a`)

| run | client pin | status | evidence |
|---|---|---|---|
| `20261124T170757-native-element-emission-8lmg58tb` | `3cf928edb6fb` (re-land) | passed | 294 near-white px, top texel (254,254,224) |
| `20261124T170823-native-element-emission-h96uc23k` | `3cf928edb6fb` | passed | 167 px, top texel (254,254,224) |
| `20261124T170846-native-element-emission-zyp5ronb` | `3cf928edb6fb` | passed | 306 px, top texel (254,254,224) |
| `20261124T170927-native-element-emission-m74j00di` | `dde97f91649b` (control) | failed at `emissive_overlay_full_bright` only | 0 px; both light assertions true |
| `20261124T171651-native-element-emission-hed9ssnz` | `dde97f91649b` | failed at `emissive_overlay_full_bright` only | 0 px; both light assertions true |
| `20261124T171718-native-element-emission-clcvui9i` | `dde97f91649b` | failed at `emissive_overlay_full_bright` only | 0 px; both light assertions true |

All six read `bush_cell_under_torch` = sky 0 / block 7 and `bush_cell_before_shot` = 0/0, and both
`wait_terrain` receipts report `status: ready` with `selected_for_draw: true`. Three consecutive green
runs and three red controls, each red on the row's own assertion and on nothing else.

**Negative example — the first form of the scene was a tautology.** `e51a555f5` registered the scene
without hiding the HUD and without the roofed cell; on that form the control `dde97f91` read
`20261124T160301` **147** near-white pixels (crosshair plus sky in the ROI) and *passed*. `72cd5e50a`
roofed the cell and hid the HUD, after which the control read 0 in every run (`162302`, `162841`,
`163147`, `163517`). The threshold 60 is bounded by measurements (minimum green 167 after the
hardening; control 0), and the plain `0` red of the pre-hardening fixture was indistinguishable from
the load-delayed `0`, which is why the light probe above exists.

## a/-side adaptation

None: the three control lines are `83994480`'s own pre-image lines (`light`, `light`,
`light[i].light`), byte-exact from its diff, and the re-land restores its `+` lines. No stub, no
flag, no early return.
