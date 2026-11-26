# MODEL-OPUS-N8 — hunk map for the committed reversal pair

Promotion: `docs/bugs/promotions/MODEL-OPUS-N8.json` (written by `ledger promote`).
Scene: `native-entity-bogged` (`tools/harness/bogged_scene.json`), proving assertion
`rig_layer_indices`.

| half | client commit | content |
|---|---|---|
| control | `client:285dd467` (`wt/D123PP3N8Ctl`) | `git apply -R` of `client:b40b8d63`'s non-test hunks at `ba1971d76215`: 15 `assets/krasnokamen/models/entity/*.json` rigs and the four `presentation.rs` lines of the same commit |
| restore (= `fix_commit`) | `client:b40b8d63` | the real fix; `git diff 285dd467 ba1971d76215` is that commit's non-test patch (patch id recorded by `ledger promote`) |

`b40b8d63`'s test hunk (`src/entities/asset_model_regression_tests.rs`) is not part of the patch:
the control keeps the tests, which is why the control pin fails them (the row's own red unit ref).

## What the frame separates

The client mesher emits six quads per cube in every rig (an authored `faces` map selects UV rects,
not quad count), so a rig's own layer indices are `6 * 6 * cubes` over every bone, nested bones
included. Walking both sides' JSONs gives the bogged **216 indices / 36 quads** in the pre-image and
**468 / 78** with the fix; the scene summons one `minecraft:bogged` (not a `crowd::KINDS` member, so
it stays on the CPU path and reports its layers) and asserts the largest `EntityCutout` slice of the
captured frame.

The mirror flags themselves (UV winding) have no count or bounds effect; they stay unit-covered by
`left_side_mirror_flags_follow_the_26_3_layers`, which the row's `repro_check` records as red with
the fix reverted.
