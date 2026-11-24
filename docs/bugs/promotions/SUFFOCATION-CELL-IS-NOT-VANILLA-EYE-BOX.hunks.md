# SUFFOCATION-CELL-IS-NOT-VANILLA-EYE-BOX — fix patch

The row had no fix commit before this work (the finding is a missing vanilla rule). The fix is this
production change, promoted in patch mode from
`docs/bugs/promotions/SUFFOCATION-CELL-IS-NOT-VANILLA-EYE-BOX.patch`; SHA-256 of its non-test hunks is
recorded on the row as `fix_commit`. It was revised after the round-7 review: the first version used
`0.4 * half_width` for the eye box's half size (0.12 for a player instead of vanilla's 0.24) and looked the
eye-height contract up by the raw type id (data-defined blueprints fell back to `0.9 * height`).

Fix commits on `wt/D123Battle` (all in the patch):

| commit | what |
|---|---|
| `505deb2f5` | the eye box + per-type eye height + `ContactStamp<3>` |
| `58e411e55` | rustfmt of the probe |
| `e984a1118` | `* 0.8` half-size, `wire_type_id` contract lookup, and the discriminating test position |
| `dde2bd0ed` | rustfmt of the probe (part of the b-side; file hashes 60c8a6519 → f6915c6d3) |


One production file (`crates/mc-core/src/contact_hazards.rs`); no hunks are omitted. The kept regression
`suffocation_uses_the_vanilla_eye_box_not_the_foot_column_pin` and the corrected wake test live in
`crates/mc-core/src/contact_hazards_tests.rs` and are not part of the patch.

## b/-side (HEAD)

- `eye_height(kind, pose, height)`: vanilla `Entity.getEyeHeight` above the feet — the player pose height,
  else the type contract's `eye_height` scaled to the live body height (`0.9 * height` only when the
  contract carries none). The contract is resolved through `blueprint::wire_type_id(type_id)`, exactly like
  `LIVING_BY_TYPE`, so a data-defined blueprint keeps its vanilla type's eye height. This is the second half of the row: a spider's contract eye is 0.65, where the
  old `0.9 * height` gave 0.81 and read the cell above.
- `eye_box(pos, shape, eye)`: vanilla `Entity.isInWall`'s `AABB.ofSize(eyePosition, width * 0.8, 1.0E-6,
  width * 0.8)` — half size `0.4 * width = 0.8 * half_width` (0.24 for a 0.6-wide player), a ±5e-7 y span.
- `contact_cells` now stores `[support, eye_min, eye_max]`; the memo stamp grows from `ContactStamp<2>` to
  `ContactStamp<3>` so a change of the eye box on an unchanged body box wakes the re-probe (the old stamp
  only carried the pin cell, which the new rule no longer keys on).
- Both suffocation decisions test every cell the box covers: the pure `exactly_clear` classifier and the
  serial `scan_row`.

## a/-side (the control)

The single foot-column pin cell at `floor(pos.y + 0.9 * height)` and the old `ContactStamp<2>`
(`[support, eye pin]`). A player 0.8 into the cell has its eye box in two x cells while the pin cell is
air; with the pin rule it does not suffocate. The kept test's discriminating position is `pos[0] + 0.3`
(0.8 into the cell): the vanilla half-size 0.24 reaches 1.04 and suffocates against the next cell's solid,
while the reviewed 0.12 reached only 0.92 and would not — so that wrong half-size fails this test.

## Documented limits

- The cell predicate is the engine's `registry.light.suffocating` state flag, which stands in for vanilla's
  `isSuffocating(state) && collision shape intersects the box`; the shape-intersection part is not modelled
  (a suffocating state whose collision shape does not reach the eye box would count here).
- The eye height is the contract value scaled by the live body height; vanilla uses
  `EntityDimensions.eyeHeight` for the current pose, which the engine does not track per pose beyond the
  player's.

# Why the kept wake test changed

`support_and_eye_cells_wake_when_body_box_is_unchanged` keyed its vertical nudge on `0.9 * 1.4` for the
cow. The cow's contract eye height is 1.3, so the nudge is now 64.69 → 64.71 (eye 65.99 → 66.01) and the
stamp's `probes[1][1]` (now the box's minimum cell) is still 65 for the tall position.
