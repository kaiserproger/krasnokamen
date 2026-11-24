# MODEL-SHADE-OVERRIDE — hunk map for the tip reversal pair

Promotion receipt: `promotions/MODEL-SHADE-OVERRIDE.json`.
Client pair: reversal `client:da965e30` → re-land `client:858518eb` (the row's `fix_commit`).
Both pins were built with `tools/dev/pin-release --client` from a clean tree
(`da965e30aa08`, `858518eb7ffa`); the pair's net tree is byte-identical to client `main`
`530556e8`, so the branch tip is the wave-2 base tree.

## Original commit

`client:43362918` — "Integrate Solaris client rc4 candidate", 2026-11-03, 217 files: the bulk
integration that first implemented 26.3 `shade_direction_override` in the terrain mesher. This
row's behaviour is the shade decision in `src/terrain/mesher.rs::model_quad`; the pair forward-ports
only that hunk.

## Hunks

| file | original hunk | content | disposition |
|---|---|---|---|
| `src/terrain/mesher.rs` | `@@ -283,11 +283,15 @@ fn model_quad(` | `let shade = if quad.shade { rules.directional_shade[quad.face as usize] } else { rules.directional_shade[1] };` → `rules.directional_shade[match quad.shade_direction { Some(face) => face as usize, None if quad.shade => quad.face as usize, None => 1 }]` plus the 26.3 comment | REVERSED — the one hunk `da965e30` reverses and `858518eb` restores; at HEAD it sits at `src/terrain/mesher.rs:299-313` verbatim (later commits did not touch the expression) |
| every other file of `43362918` | — | other rows' models, rigs, UI and their tests | OMITTED — (a) another row's behaviour; `CM1-REVERSIBILITY.md` records, per file, whether the hunk still reverses, and no other file carries this row's symptom |

## a/-side adaptation

None. The control's `a/`-side is the byte-exact pre-fix expression from `43362918^` — the same two
lines that commit replaced — so the pair's diff is the fix's production patch and nothing else. No
stub, no flag, no early return.

## Proving ref

`harness:scene-shot:native-shade-override` → `torch_faces_same_shade`
(`tools/harness/backends/scene_shot_checks.py:261`): one `minecraft:torch` (model
`template_torch`, `shade_direction_override: "up"`, one texture region on all four side faces) seen
from its north face and from its east face must read the same mean luminance. With the pre-fix
expression the two faces take their own cardinal weights — overworld 0.8 north, 0.6 east, a 4:3
ratio — so the control reds the row's own symptom; with the override both take the up weight (1.0).
The three `client:krasnokamen-client:terrain::border_tests::shade_direction_override_*` refs cover
the weight selection at unit level and stay green on the candidate.

The `north`/`down` directions the row also names have no placeable subject on this server:
`flowerbed_*` (the only `north` models) is absent from the block data, and `snow_height2` overrides
its own single `down` face with `down`, which shades identically before and after the fix. The `up`
direction is the same code path (`quad.shade_direction`) and covers the row's majority blockstates
(crosses, crops, torches, lanterns, chains, redstone dust, repeaters, tripwires, vines).
