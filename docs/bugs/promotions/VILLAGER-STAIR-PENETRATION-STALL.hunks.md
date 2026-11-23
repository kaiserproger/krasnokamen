# VILLAGER-STAIR-PENETRATION-STALL — HEAD re-break control patch

Patch: `docs/bugs/promotions/VILLAGER-STAIR-PENETRATION-STALL.patch`
(`patch:584575b9d287cdae…`), the row's `fix_commit`. It reverses on the clean
committed tree at `b67e1b0612` (`git apply -R --check`, plain context) and is
what a promotion passes as `--fix-patch`.

## What the fix is (origin `f763216ae`, VILLAGER-STAIR-PENETRATION-STALL)

The seat feature (`stair_seat`, `VillagerState::seat`) first placed the physical
body at the stair's own centre, `[x+0.5, y+0.5, z+0.5]`, i.e. inside the stair.
The same slice validates a free standing position one block above the stair
(`standing(...)` with the body's half width and height) and seats the body
there; the seat anchor stays the stair block for the presentation edge.

## HEAD equivalent: the two hunks

At HEAD `stair_seat` returns `(seat, position)`; the control drops the
`standing(...)` guard and the `body` binding and returns the stair centre
again. Everything else in the seat lifecycle (the `seat` field, `clear_seat`,
`can_sit`, the edge marker) is unchanged between the fix and HEAD.

## Omissions

- `villager/mod.rs` (`seat` field), `villager/persistence.rs` (journal reset),
  `boundary.rs`, `core/recovery.rs`, `villager/nameplates.rs`,
  `mc-edge/src/plan.rs`, `mc-edge/src/plan/player_changes.rs`,
  `mc-edge/src/settlement_labels.rs` — **(b)**: the kept tests
  (`brain_tests.rs` reads `villager_seat`, `edge plan_tests.rs` drives the
  spawn/link/remove order, persistence tests compare the journal image) call
  this API; deleting the field or the edge steps stops the control from
  compiling.
- `docs/SETTLEMENTS_GUIDE.md` (the seating bullet) — **(c)**: the line is no
  longer at HEAD (a later commit rewrote the guide), so the original hunk has
  no HEAD equivalent.

## Reverse-apply evidence

`git apply -R --check docs/bugs/promotions/VILLAGER-STAIR-PENETRATION-STALL.patch`
on the committed tree `b67e1b0612` succeeds.

Control run (reversed tree, then restored): `python3 -m tools.harness run test
-p mc-core --lib villager::brain::tests::resting_villager_moves_onto_nearby_indoor_stair`
→ `.analysis/validation/20261123T195357-test-i3pp9tvr` (`status: failed`,
`dirty` tree at head `8d3c925863`). The red is the row's own assertion
`crates/mc-core/src/villager/brain_tests.rs:1039`, "the physical seated body
must not overlap its stair" (the body sits at the stair's centre). The
graphical `harness:scene-shot:native-village-stair-penetration` red is the
integrated sweep's control run, not claimed here.
