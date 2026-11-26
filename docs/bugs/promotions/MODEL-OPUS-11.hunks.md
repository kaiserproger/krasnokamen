# MODEL-OPUS-11 — hunk map for the promoted pair

Promotion receipt: `promotions/MODEL-OPUS-11.json` (written by `ledger promote`; stale until the
redesign below lands).
Client pair used: control `client:e3678206` → fix `client:3bfbd7b6`. This map supersedes the older
`4092ac53 → 92dcb25d` text, which named a pair the lane never ran and printed quad counts as if they
were indices.

## What the pair carries for this row

| file | change | role |
|---|---|---|
| `assets/krasnokamen/models/entity/allay.json` | whole file: 34 quads → 42 (252 indices); the wings gain their faces, the head is reached through the layer root, and the rest pose puts the wing tip 11.1 px from the head's front face | the row's allay subject |
| the nine horse-family rigs (`horse.json`, `horse_body_equipment.json`, `horse_saddle_equipment.json`, `donkey.json`, `donkey_saddle_equipment.json`, `mule.json`, `mule_saddle_equipment.json`, `skeleton_horse.json`, `zombie_horse.json`) | whole files | moved to MODEL-OPUS-11-HORSES (class (a)) |
| `bee_baby.json`, `cod.json`, `guardian.json`, `elder_guardian.json` | whole files | class (a): MODEL-OPUS-10 / MODEL-OPUS-9 |
| `src/entities/presentation.rs` `@@ -4290,9 +4290,25 @@ mod tests`, `asset_model_regression_tests.rs` (three hunks), `krasnokamen_models/tests.rs` (three hunks), `animation_tests.rs` (one hunk) | test-only | the row's unit refs |

No production code hunk exists outside the rig JSONs, so the earlier `204-252 = -48 → 0` arithmetic
was about quads, not indices: the allay's cutout is 252 indices on both sides of the pair.

## What the scene showed, and why it cannot promote

`native-rig-parity` reports the allay's own cutout bbox span: 1.122 blocks on the control against
0.690 on the fix, where the old check asserted `span <= 0.85`. Two things are wrong with that:

1. the threshold was chosen after those two measurements, and its stated justification ("body depth
   0.70") is wrong — the body is 2 px = 0.125 blocks; 0.694 is 11.1 px, the distance from the head's
   front face at -2.5 px to the wing tip at 0.6 + 8 px in the layer's REST pose;
2. the rest pose is not the pose the game shows. `javap -p -c` on the local 26.3 client jar
   (`net/minecraft/client/model/animal/allay/AllayModel`, Mojang names, never committed) shows
   `setupAnim` sets `right_wing.xRot = 0.4363 * (1 - min(walkSpeed / 0.3, 1))` and
   `right_wing.yRot = -0.7854 + f`, left mirrored — 25 degrees about x, ∓45 about y at idle. The
   client does not apply it (it only rolls the allay ±0.3 rad), so 0.690 is the client's rest pose,
   and a `|span - 0.694| <= 0.03` window would freeze exactly that defect.

Evaluated with the client's own bone convention, the dump's allay reaches 0.7696 blocks horizontally
when those two rotations are applied (x -0.385..0.385; the rest pose is 0.6937 with x ±0.156). The
redesign is therefore: implement the setupAnim wing rotation, then assert `|span - 0.770| <= 0.03`
two-sided, on a scene of the allay alone. Until then the row cannot be promoted, and the
`native-rig-parity` span check is labelled withdrawn evidence in the scenario entry.

## The pair that promotes it (2026-11-25, second pack)

Main's decision on the evidence class: the row's fix is the invented-rig -> vanilla-rig re-land
PLUS the setupAnim pose, and the derived envelope separates the invented-rig control from the
vanilla-rig+pose client. A pose-less control is not the row's pre-image and is out of the scene's
reach; that half is pinned by the client unit test.

| half | client | content |
|---|---|---|
| control | `bafbf78b40` | CM-4 list-form frame report + the fourteen invented rigs (`allay.json` with the wings baked spread), no pose |
| fix | `0eb3e0ea1e` | the vanilla rigs (the pose commit `715a7110af` is the candidate tip and rides as a unit ref, see below) |

Why the fix side is `0eb3e0ea1e` and not the tip: `ledger promote`'s client rule demands the two
pinned client heads differ by exactly the fix commit's non-test patch, and no single commit spans
the rig re-land and the pose. Pairing `bafbf78b40` with `0eb3e0ea1e` is therefore exactly
`client:3bfbd7b6`'s patch — the ledger's own `patch-id --verbatim` over the non-test hunks reads
`0622414eba9c283339a746290872e89a6bbebdc3` for `3bfbd7b6`, for `0eb3e0ea1e` and for the
`bafbf78b40..0eb3e0ea1e` delta (`promote.json: client_control_delta.fix_patch_id`; an earlier draft
printed `6cf9f2be…` from a whole-commit `--stable` id, which is a different measurement) — and the
pose commit is covered by the row's `client:` unit ref, which the promote runs on the candidate
client tree.

Non-test hunks carried: the fourteen rig JSONs (the same patch that `client:3bfbd7b6` carries) plus
the `procedural_part` hunk of `715a7110af` (the allay arm and its `walk_pos` parameter, threaded from
`evaluate_posed_rig`). The unit-test hunks (`allay_wings_take_the_vanilla_setup_anim_pose`, and the
fire test's own) are test hunks and are not part of the non-test patch. `fix_commit` is therefore a
multi-commit value: the promote run prints the exact `patch:<sha256>` (or `client:<sha>`) to record,
and that value goes into the row.

## Promoted 2026-11-26 (this lane)

`ledger promote MODEL-OPUS-11` on the lane's core tree at `aba1e0d57fae` (clean fingerprint
`e3b0c442…`) with the tree's own server pin `pins/aba1e0d57fae-server-…`:

| side | client pin | receipt | result |
|---|---|---|---|
| control | `bafbf78b400a` | `20261126T122219-native-rig-allay-v1n7an4g` | `failed` at `allay_wing_span_within_wings`, measured span **1.116** |
| restore | `0eb3e0ea1eec` | `20261126T122427-native-rig-allay-jioofe4h` | `passed`, span **0.693** (inside 0.68..1.03) |

Promotion artifact `20261126T122732-ledger-promote-8wb8lfk0`; the receipt records
`fix_commit client:3bfbd7b6`, the delta above and the 14 rig paths. The 0.693 span is the client's
rest pose: the envelope separates the invented rig (1.116) from the vanilla rig, it does not witness
the pose, which the client unit ref pins — exactly as the row states.

Rabbit/fox/panda/vex moved to MODEL-OPUS-11-RABBIT-FOX-PANDA-VEX and the horse family to
MODEL-OPUS-11-HORSES; both are open and unit-covered only.

