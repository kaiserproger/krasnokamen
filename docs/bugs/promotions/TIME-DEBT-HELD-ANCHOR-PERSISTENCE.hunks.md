# TIME-DEBT-HELD-ANCHOR-PERSISTENCE: patch-mode fix identity

`fix_commit = "patch:edeb0a8432f96c336ad86e90906f7e4b5105c1e74c97cac48ea0734a6fbbe1ff"`
(`docs/bugs/fix-patches/TIME-DEBT-HELD-ANCHOR-PERSISTENCE.patch`).

Original fix: `c4d8c0d01` "fix(core): anchor catch-up debt to held time". Its
abstract-simulation freeze gate is TIME-FROZEN-CATCHUP-DEBT (promoted,
`patch:e6c2c048`); its `level_metadata` slot is PERSIST-METADATA-RECAPTURE-BYTE-ORDER
(promoted); this row owns the durable held anchor.

## Why not the commit itself

All six row-owned files still apply in reverse at this HEAD
(`git show c4d8c0d01 -- <file> | git apply -R --check`), but reversing them as a
set does not compile: 60 errors (`.analysis/validation/20261124T092805-test-fk3s90cc`).
Later commits rewrote code that now uses the new API (`journal/replay.rs:698,1410-1413`,
`native_checkpoint/game_clock_upgrade.rs:171`, `world/clocks.rs:230`), and the kept
tests construct the new fields. The patch is therefore the row's real pre-image
forward-ported to the candidate, with the kept API restored as **named (b)
constructor lines** and nothing else invented.

## Carried (the real pre-image of the row's five reversible files)

| original (c4d8c0d01) | HEAD |
|---|---|
| `debt.rs` `project`: pushes `HELD_ANCHOR_KEY` | reverted to the pre-image body (no key push, no anchor range check); the 4-argument signature stays |
| `debt.rs` `adopt_native_extras`: reads `HELD_ANCHOR_KEY`, validates, stores it | reverted: no read, no `validate_anchor` call, `held_anchor: None` (constructor); the 5-argument signature stays |
| `weather.rs` `ClimateImage`: field, `capture` writes the epoch, `restore` re-anchors | field kept; `capture` writes `None` (constructor); the restore lines are gone |
| `core/recovery.rs`: restores the epoch after replay | lines removed (pre-image) |
| `world/capture.rs`: carries `held_anchor` in the deferred image and into `project` | pre-image destructuring; `held_anchor: None` in the image (constructor); `project(.., None)` |
| `world/residency.rs`: carries `held_anchor` in the eviction image and into `adopt_native_extras` | pre-image boundary tuple; `held_anchor: None` in the image (constructor); the kept 5-argument `adopt_native_extras` call |

`journal.rs` is **not carried**: `SimDebtImage.held_anchor` and `at()` are API the
class-(c) code (`journal/replay.rs:698,1413`, `world/clocks.rs`) and the kept tests
call, so reversing them is a compile failure; they stay on both sides. The debt
test-module declaration (`debt.rs` tail) also stays, otherwise the control deletes
the very tests it must run.

Constructor lines (class (b), behaviour of the pre-image, nothing invented):
`held_anchor: None` at the four image/record sites, `sim_debt_anchored_since: None`
in `ClimateImage::capture`, `None` as `project`'s fourth argument, and the `_`-
prefixed unused parameters `_held_anchor`, `_tick`, `_held_ticks` (kept signatures).

## The control

Reversed, the durable anchor is never projected into native extras, never adopted
back, never written into capture or eviction images and never restored from the
climate epoch — the row's symptom (held frames re-counted as dormant after a
restart) is back. Two refs fail by assertion
(`.analysis/validation/20261124T103613-test-41kab_gk`):

- `cargo:mc-core:debt::game_clock_tests::debt_game_clock_native_projection_keeps_explicit_zero_and_one_key`
  at `debt_game_clock_tests.rs:141` — the projected `HELD_ANCHOR_KEY` / `adopt_native_extras`
  round trip.
- `cargo:mc-core:debt::game_clock_tests::debt_game_clock_mid_tick_rejection_keeps_both_possible_native_anchors`
  at `debt_game_clock_tests.rs:122` — the image's two possible native anchors.

The row's two replay/world refs
(`debt_game_clock_held_activation_excludes_its_own_held_tick`,
`debt_game_clock_shared_projection_reanchors_all_dimensions_and_preserves_remaining`)
stay green in that control because their code lives in the class-(c) files; they
are recorded as green regressions, not as control evidence.
