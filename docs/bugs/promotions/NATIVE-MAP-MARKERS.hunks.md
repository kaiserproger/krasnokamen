# NATIVE-MAP-MARKERS — DRAFT hunk map for a committed reversal pair (no promotion)

**Status: draft. There is no `promotions/NATIVE-MAP-MARKERS.json`; the row is `status = "open"` and
its `fix_commit` is `client:e1b22a5a` (`docs/bugs/rows/NATIVE-MAP-MARKERS.toml:7,10`).** The pair
below is committed on `wt/D123ClientNativeF` and waits for a drawn-marker predicate that
discriminates; the re-land `55b8374ed7` is a *candidate* `fix_commit`, not the row's.

Client pair: reversal `client:864a757fe9` → re-land `client:55b8374ed7`.

## The pair's own diff

```
git diff 864a757fe9 55b8374ed7 -- src/ui/packets.rs
@@ -929,14 +929,13 @@ impl MapState
-                * Mat4::from_translation(Vec3::new(-0.125, 0.125, -0.001 * index as f32))
-                * Mat4::from_scale(Vec3::new(1., -1., 1.));
+                * Mat4::from_translation(Vec3::new(-0.125, 0.125, -0.001 * index as f32));
-                [0., 0., 1., 1.]
+                [0., 1., 1., 0.]
```

(The comment lines the fix added move with the hunk; `non_test_patch` of the re-land is this patch.)

## Why a reversal and not the pre-image pin

`e1b22a5a` is post-rename and its one production hunk reverses on the wave-2 tip, but the row needs a
graphical control: the historical pre-image clients cannot start in the current private graph. The
committed pair keeps the tip's code and changes only the marker transform.

## Hunks

| file | hunk | content | disposition |
|---|---|---|---|
| `src/ui/packets.rs` | `@@ -929,14 +929,13 @@` | flip the sampled image (`[0., 1., 1., 0.]`) and drop the `Mat4::from_scale(1., -1., 1.)` that reversed the quad's winding; the comment explains that back-face culling hid the marker | **REVERSED** — the pre-fix transform and UVs are restored verbatim from `e1b22a5a^` |
| `src/ui/packets.rs` | test modules | `held_map_tests` / `map_lifecycle_tests` | KEPT (test hunks, the row's unit refs) |

## a/-side adaptation

None: `git apply -R` of `e1b22a5a`'s production hunk applies to the tip with intact context, so every
reversed line is the historical pre-image and `git diff 864a757fe9 55b8374ed7` is exactly that patch.

## Proving ref — NOT YET DISCRIMINATING (row stays open)

`harness:entities:native-v2-map-frame-waypoint` keeps its model check
(`map_distant_clone_keeps_exactly_one_frame_waypoint`, true on both clients), and four drawn-marker
shapes were measured on the real scene; none produced a usable control, so the backend was restored
byte-for-byte to `ce570b55ab` (core `536472a493`) and the scene is green again. The pair
`864a757fe9 → 55b8374ed7` stays committed for the next attempt.

| shape | measured result | receipt |
|---|---|---|
| remove the frame (`kill @e[item_frame]`) and wait for the decoration to vanish | the server never drops the decoration; the 900 s budget burned after the model check | `20261124T144615-…-shui7fge`, `…150136-…-bxd_o6a8`, `…151701-…-1qxdqmuk` |
| two maps of one centre, wait for identical terrain **before** switching slots | the client only receives the *held* map's data, so the equality never held (180 s) | `20261124T153835-…-a0n75906`, `…154219-…-p6x2vr6u`, `…154549-…-v28rco7_` |
| same wait **after** switching slots | the framed map's retained colours are partial, so equality still failed inside 180 s | same runs (the reorder is `dd67e4a2c1`, reverted) |
| measured threshold | never reached: no shape got to the diff, so no counts exist to derive a threshold from | — |

Next shape (not attempted): keep the **same map id** and move the frame instead of removing it — frame
it north, capture the held map near the frame, then break and re-place the frame east with a fresh
clone, capture again. Same map id means the terrain cannot differ and the only rendered change is the
marker icon moving; a client that never draws it produces identical frames. `_scene`/`_frame` already
give the near-capture seam (`map-framed-copy-near`).
