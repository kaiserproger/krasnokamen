# NATIVE-MAP-MARKERS — hunk map for the committed reversal pair

Client pair: reversal `client:864a757fe9` → re-land `client:55b8374ed7`.
**REVOKED 2026-11-25**: promotion `20261124T210606-ledger-promote-ftjn0o0f` was withdrawn — the third control run
passed with 181 scattered pixels (`…232147`) and one restore failed the scene's model check (`…232050`). The row is `open` again.
re-land restores exactly that commit's production hunk, and `e1b22a5a` is an ancestor of the
re-land, so the row's `fix_commit` stays the historical commit).

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

`git patch-id --verbatim` of the pair equals that of `non_test_patch(client repository, e1b22a5a)`
(`937a7fcfa5aa1c5c…`) — the equality is what `git patch-id --verbatim` reports for both patches, and
the texts differ only in their `index` lines and `@@` line numbers (the tip added lines above the hunk),
which patch-id ignores.

## Why a reversal and not the pre-image pin

`e1b22a5a` is post-rename and its one production hunk reverses on the wave-2 tip, but the row needs a
real-client red/green pair and no client before the wave-2 tip is pinned for this lane. The committed
pair keeps the tip's code and changes only the marker transform.

## Hunks

| file | hunk | content | disposition |
|---|---|---|---|
| `src/ui/packets.rs` | `@@ -929,13 +929,14 @@` | flip the sampled image (`[0., 1., 1., 0.]`) and drop the `Mat4::from_scale(1., -1., 1.)` that reversed the quad's winding; the comment explains that back-face culling hid the marker | **REVERSED** — the pre-fix transform and UVs are restored verbatim from `e1b22a5a^` |
| `src/ui/packets.rs` | test modules | `held_map_tests` / `map_lifecycle_tests` | KEPT (test hunks, the row's unit refs) |

## a/-side adaptation

None: `git apply -R` of `e1b22a5a`'s production hunk applies to the tip with intact context, so every
reversed line is the historical pre-image and `git diff 864a757fe9 55b8374ed7` is exactly that patch.

## Proving ref — the drawn-marker predicate

`harness:entities:native-v2-map-frame-waypoint` → `waypoint_marker_drawn`.

`_frame_waypoint` now holds a **locked** copy of one map id (`_locked`: a cartography-locked map never
samples terrain — `crates/mc-core/src/maps/runtime.rs` gates sampling on `!map.locked` — while framed
copies still deliver pixels and markers) and takes two retained captures at one pose with the support
and the empty frame already placed:

1. the map held, the frame empty;
2. a second prepared copy inserted into the frame **by hand** (no command runs between the captures),
   the map held again.

The received colour array is asserted equal across the two captures (`map_pair_identical_terrain`,
0 changed cells in every run) and the chat is allowed to fade out first (510 ticks) so its vacated text
does not rewrite the map: the read region is `(620, 170, 930, 560)`. `_differing_pixels` counts a pixel
changed only when one channel moves by more than 24 (the daylight lightmap drift is ~6), and
`waypoint_marker_drawn` requires at least 150 changed pixels there.

Measured 2026-11-25 (first with the earlier head, then re-run for the promotion on core
`ba20240cb`, server pin `pins/ba20240cbc3e-…`):

| run | client | changed pixels in region0 | assertion |
|---|---|---|---|
| `20261124T205630-…-wqpvck5n` | `55b8374ed7` (restore) | **395** | passed |
| `20261124T205736-…-9gun21sq` | `864a757fe9` (control) | **0** | `waypoint_marker_drawn` false |
| `20261124T210350-…-40grn4zr` | `55b8374ed7` (restore, **promotion receipt**) | passed | `waypoint_marker_drawn` true |
| `20261124T210449-…-g_ro5jis` | `864a757fe9` (control, **promotion receipt**) | red | `waypoint_marker_drawn` false |

**REVOKED 2026-11-25**: promotion `20261124T210606-ledger-promote-ftjn0o0f` was withdrawn — the third control run
passed with 181 scattered pixels (`…232147`) and one restore failed the scene's model check (`…232050`). The row is `open` again.
re-run green against the candidate tree).

Shapes that did not discriminate, kept for the record:

| shape | measured result |
|---|---|
| remove the frame (`kill @e[item_frame]`), wait for the decoration to vanish | the server never drops the decoration; the 900 s budget burned |
| two maps of one centre, wait for identical terrain | the client receives only the held map's data; the equality never held |
| frame placed between the two captures | the map's own terrain changed (1408 / 16384 cells): an unlocked map samples only as its holder explores |
| two captures with the frame already placed, unlocked map | the map keeps sampling (17 patches in 35 s), colours drift |
| captures with the HUD hidden (F1) | F1 hides the held item too: two identical empty frames |
| captures with command feedback off but a `debug give` in between | one chat line scrolls the text over the map: 46..660 "changed" pixels on the control |
