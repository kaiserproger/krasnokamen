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

## Every other hunk of `83994480`

`83994480` is a bulk ClientNext integration (`@@` hunks across `presentation.rs` (43),
`render_blocks.rs` (7), `render_blocks/surfaces.rs`, `protocol/models.rs` (7), `ui/state.rs` (3),
`entities.rs` (2) plus rig assets). Every hunk not listed above is (a) another row's behaviour
(command feedback, GUI panes, use poses, battle materials) or (b) API the kept tests call;
`CM1-REVERSIBILITY.md`'s addendum records that those files reverse only 3-way with conflicts, and the
task's rule is to carry only the hunks that still reverse — the three above do, so the rest stay
queued under CM-1 for their own rows.

## Proving ref

`harness:scene-shot:native-element-emission` → `emissive_overlay_full_bright`
(`tools/harness/backends/scene_shot_checks.py::check_element_emission`). One `minecraft:firefly_bush`
stands in a roofed stone cell (sky light 0 inside) at midnight; the HUD is hidden (F1) because the
crosshair is a fixed white cross. `firefly_bush_emissive.png` has 9 texels with every channel above
200 and the base texture has none, so the count of near-white pixels in the bush's region can only be
the emissive overlay. Measured: control **0**, re-land **306** (threshold 60).

The two unit refs stay green on the candidate: the baker test (emission is baked per element, 0..=15
bounded) and the mesher test (the emissive quad's vertices carry light 15). The latter reds on the
control by design — it is the row's own assertion.

## a/-side adaptation

None: the three control lines are `83994480`'s own pre-image lines (`light`, `light`,
`light[i].light`), byte-exact from its diff, and the re-land restores its `+` lines. No stub, no
flag, no early return.
