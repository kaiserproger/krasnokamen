# VILLAGER-STAIR-PENETRATION-STALL — HEAD re-break control patch

Patch: `docs/bugs/promotions/VILLAGER-STAIR-PENETRATION-STALL.patch`
(`patch:255a2ab9a37798c2f77f2f42b022f57dfa89d7ff413e1088ad6cc512dd9fe38f`),
the row's `fix_commit`. It reverses on the clean committed tree
(`git apply -R --check`, plain context) and is what a promotion passes as
`--fix-patch`.

## What the fix is (origin `4e6874ded`)

The origin commit is `4e6874ded` (*land CivSim-5 civ-sim slice…*). Its
`crates/mc-core/src/villager/brain.rs` change replaces the seat placement, not the
seat itself:

```rust
-                    && standing(..., body.half_width, body.height)
-                    .is_some()
+                    && let Some(position) = standing(..., body.half_width, body.height)
                 {
-                    return Some(seat);
+                    return Some((seat, position));
                 }
```

Pre-image (`git show 4e6874ded^:crates/mc-core/src/villager/brain.rs`): the
`standing(...)` guard and the `let body = state.entities.body(slot);` binding are
both already there (they were added by `2e5fa2944`, *a stair seat needs body room
above it*), and the seat feature that `f763216ae` added sits below them: the
guard validates a free standing position one block above the stair, and the
caller then places the physical body at the stair's own centre,
`[x+0.5, y+0.5, z+0.5]` — the penetration this row reports. The fix returns the
*validated* position instead and seats the body there; the seat anchor stays the
stair block for the presentation edge.

## HEAD equivalent: one hunk

At HEAD, `stair_seat` returns `(BlockPos, [f64; 3])` and the caller does
`state.entities.set_pos(slot, position)`. The control keeps HEAD's signature, keeps
the `standing(...)` guard and the `let body` binding, and reverts only what the fix
changed: the returned position is the unchecked stair centre again, so the caller
seats the body where the pre-image did.

Reversing that one line is the whole behavioural reverse: the pre-image's other
caller hunks (`stair_seat(state, slot) != Some(seat)` and the `if let Some(seat)`
placement) only plumbed the tuple; at HEAD the same callers compare
`.map(|(position, _)| position)` — the seat *anchor* — and place whatever position
the function returns, so with the centre restored they compute exactly the
pre-image. Nothing else in the seat lifecycle (`seat`, `clear_seat`, `can_sit`,
the edge marker) differs between the fix and HEAD.

## Differences from the real pre-image

- **a/-side adaptation (named):** the pre-image returned `Option<BlockPos>` and its
  caller built the centre array; HEAD's caller destructures `(seat, position)`, so
  the reversal returns the centre inside HEAD's tuple, `Some((seat, [f64::from(seat.x)
  + 0.5, …]))`. This is the only a/-side text outside the pre-image, and it changes
  no behaviour — it is the pre-image's own placement array in HEAD's constructor.
- **Kept:** `let body = state.entities.body(slot);` (context on both sides) and the
  guard. The earlier control dropped both, which reversed more than the fix and let
  the guard's absence look like part of the pre-image; that was wrong and is fixed
  here.

## Omissions

- `villager/mod.rs` (`seat` field), `villager/persistence.rs` (journal reset),
  `boundary.rs`, `core/recovery.rs`, `villager/nameplates.rs`,
  `mc-edge/src/plan.rs`, `mc-edge/src/plan/player_changes.rs`,
  `mc-edge/src/settlement_labels.rs`, `brain_tests.rs` —
  **(b)**: the kept tests (`brain_tests.rs` reads `villager_seat`, edge
  `plan_tests.rs` drives the spawn/link/remove order, persistence tests compare the
  journal image) call this API; deleting the field or the edge steps stops the
  control from compiling.
- The rest of `4e6874ded` — **(a)**: CivSim-5 funding/ration behaviour belongs to
  other rows (FUNDED-PLANNER-IDLE, PREMATURE-LOG-DELIVERY, …); reversing it would
  turn the control red on someone else's assertion.
- `docs/SETTLEMENTS_GUIDE.md` (the seating bullet) — **(c)**: the line is no
  longer at HEAD (a later commit rewrote the guide).

## Reverse-apply evidence

`git apply -R --check docs/bugs/promotions/VILLAGER-STAIR-PENETRATION-STALL.patch`
on the committed tree at the promotion head succeeds (plain context, one file).

## Control runs (own reverted tree)

- Cargo control: `cargo:mc-core:villager::brain::tests::resting_villager_moves_onto_nearby_indoor_stair`
  goes red at the row's own assertion `crates/mc-core/src/villager/brain_tests.rs`
  ("the physical seated body must not overlap its stair"): with the guard kept and
  the centre returned, the body stands inside the stair. Receipt in the promotion
  JSON's `negative_control`.
- Graphical control: `harness:scene-shot:native-village-stair-penetration` on the
  same reverted tree, with its own pin. The earlier 001844/002042 runs were one
  shared reverse of three patches with a tree-identity mismatch (the pin belonged to
  a dirty tree, the receipt's `tree` recorded the clean commit) and are not per-row
  proofs; see the promotion JSON for this row's own run.

## a/-side audit (B5, 2026-11-24)

Script and full output: `receipts/D123/D123CivGates/a-side-audit.md`. Against the origin
`4e6874ded`, the second hunk is outside the origin's own change only in its tuple
constructor: HEAD's `return Some((` (last changed by `4e6874ded` itself — the fix's
tuple) and the restored `Some((seat, [x+0.5, …]))` lines are this map's named
adaptation. The first hunk (the guard binding) is explained by the origin. No
over-revert remains: the `standing(...)` guard and `let body` are context on both
sides.
