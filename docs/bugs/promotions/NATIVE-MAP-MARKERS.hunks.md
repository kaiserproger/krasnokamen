# NATIVE-MAP-MARKERS — hunk map for the committed reversal pair

Promotion receipt: `promotions/NATIVE-MAP-MARKERS.json`.
Client pair: reversal `client:864a757fe9` → re-land `client:55b8374ed7` (the row's `fix_commit`).

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

## Proving ref

`harness:entities:native-v2-map-frame-waypoint` → **`waypoint_marker_drawn`** (core `b099c2bc9f`): the
scene keeps its model check (one received frame decoration,
`map_distant_clone_keeps_exactly_one_frame_waypoint`) and adds a *drawn* predicate — the held map is
captured, the frame decoration is removed through `kill @e[type=minecraft:item_frame]` (the frame is 80
blocks away and off-screen, and the map keeps its received colours), the held map is captured again,
and the pixels that changed inside the held-map regions are counted (`frame_marker_pixels`) and
asserted. A client that receives but never draws the marker produces the same two frames.

Measured counts (control vs restore) and the threshold choice: see "Gate receipts" below.
