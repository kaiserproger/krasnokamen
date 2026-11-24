# ENTITY-CROWD-PUSH-INTO-WALL — fix-hunk map

The production patch `ENTITY-CROWD-PUSH-INTO-WALL.patch` is the fix's non-test hunks,
`patch:ad7b2e86f3610630cc57e5f052180737aeab7b831e1c884ac370893568838529`. It reverse-applies on the
candidate (`git apply -R --check`, clean tree at `4aadc22cf`+34af8faf86).

## The fix, in two commits (one path)

| commit | what it closes |
|---|---|
| `e282a959b` fix(physics): keep a body within the contact tolerance out of the block it touches | `clip_x`/`clip_z` take the vanilla symmetric `1e-7` tolerance, so a face the body sits up to `COLLISION_EPSILON` past is a contact, not "not overlapping" |
| `fe6e7d36f` fix(physics,core): clamp the contact tolerance in the sweep | the pre-clip pass is folded into `contact_entry`/`sweep_axis_interval`: a signed face gap within `-COLLISION_EPSILON..0` is entry time 0, which the sweep cannot drop as a past hit |

`d65bfc936` (the pen regressions) is test-only. `c393b9240`/`f3cdf12a4` later touched the same
functions for other rows (`clip_y` tolerance, the sub-epsilon early return); the patch keeps them.

## Hunks the patch carries (all in `crates/mc-physics/src/lib.rs`)

| hunk | a/-side |
|---|---|
| `resolve_horizontal_sweep` comment | the pre-image had no comment between `remaining` and the passes |
| `contact_entry` | the pre-image body `gap / movement.abs()`; the function is kept (README (b)) because `physics_tests.rs:2394,2396,2399` call it |
| `clip_x` loop | the pre-`e282a959b` loop (`if dx > 0.0 && body.max_x <= obstacle.min_x { dx = dx.min(..) } ...`) with HEAD's sub-epsilon early return kept above it (that return is `f3cdf12a4`'s, another row's) |
| `clip_z` loop | same for `dz` |

`clip_y` and the three `dx/dz/dy.abs() < COLLISION_EPSILON` early returns are identical on both
sides: they are `PHYSICS-CLIP-Y-PARITY` / `PHYSICS-CLIP-MISSING-VANILLA-EPSILON-EARLY-RETURN`
behaviour (reason (a), left in place).

## Why every other hunk of the two commits stays out

- **`7cca1476d`** (body sizes from the f32 contract, in the same fix series): every hunk is reason
  **(c)** — `fe6e7d36f` and later commits rewrote those lines, so `git apply -R` of
  `git diff 7cca1476d^ 7cca1476d -- crates/` fails on `blueprint.rs`, `falling.rs`,
  `tick/{auth,elytra,players,input/*}.rs`. Its `blueprint.rs` hunk also introduced
  `contract_dimensions`, `PLAYER_CROUCHING_HEIGHT` and `PLAYER_FALL_FLYING_HEIGHT`, which the kept
  tests call (`blueprint_tests.rs:383,424,433,446,450`, `elytra_tests.rs:150,423`,
  `sleep_tests.rs:1070,1074,1160`, `villager/brain_tests.rs:983`) — reason **(b)**.
- **`fe6e7d36f`'s `blueprint.rs` and live-body-size call sites**: reason **(b)/(a)** as above; the
  pen control pins the pre-report half width itself (`set_half_width(slot, 0.3)`), so the kernel is
  the only variable.
- **`crates/xtask/src/code_health/ratchet.rs`**: reason **(c)**, `f45907ef0` rewrote the ceiling.

## Observed control

Reversing this patch on `4aadc22cf`+34af8faf86 makes the row's own assertion red:

```
panicked at crates/mc-core/src/crowd_pen_tests.rs:155:5:
bodies resolved inside pen blocks:
tick 14 slot 4 pos [10.483160921360135, 64.75319997403622, 512.2435248766278] half_width 0.3 ...
  cells [(10, 64, 511), (10, 65, 511), (10, 66, 511)]
```

(`.analysis/validation/20261124T101830-test-hp7qj5gt`, filter `crowded_pen_bodies_never_enter_the_wall`),
i.e. the body AABB again spans into the `z=511` pen wall — the row's reported 0.25-block embed. The row
names that `file:line` in `proving_assertions`. The promotion run's own control/restore receipt is under
`.analysis/validation/<stamp>-ledger-promote-*/`.

Superseded attempt: a patch with only `fe6e7d36f`'s kernel hunk stayed **green** on the control
(`20261124T095201-ledger-promote-cirofgca`) because `e282a959b`'s clip tolerance independently closes
the band — the reason this patch carries both commits.
