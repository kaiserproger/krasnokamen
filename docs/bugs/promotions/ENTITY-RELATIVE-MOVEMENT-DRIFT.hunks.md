# ENTITY-RELATIVE-MOVEMENT-DRIFT — hunk map (patch mode)

Row: `ENTITY-RELATIVE-MOVEMENT-DRIFT` (client, major) — relative entity movement accumulates
viewer drift on closed trajectories. Refs:
`cargo:mc-edge:plan_tests::shared_moves_tests::relative_movement_cycles_return_the_decoded_consumer_to_origin`,
`cargo:mc-edge:plan_tests::shared_moves_tests::relative_movement_rounds_endpoints_at_signed_half_step_boundaries`,
`cargo:mc-edge:plan_tests::shared_moves_tests::relative_movement_preserves_unsent_components_and_accumulates_small_steps`,
`cargo:mc-edge:plan_tests::shared_moves_tests::relative_movement_range_uses_quantized_endpoints_and_sync_resets_the_consumer`.

## Real fix and why the patch is forward-ported

The fix is `4256150e2` ("fix(edge): encode relative movement from quantized endpoints",
2026-11-15). Its three product hunks lived in the pre-split `crates/mc-edge/src/plan.rs`. `9d71feb62`
split `plan.rs` into owner modules, so the original patch no longer reverse-applies. The committed
patch `docs/bugs/fix-patches/ENTITY-RELATIVE-MOVEMENT-DRIFT.patch` (sha256 `e42892e8…`) is the
HEAD-equivalent of those three hunks and nothing else; `git apply --reverse --check` succeeds at HEAD.

| original hunk (4256150e2, `plan.rs`) | HEAD equivalent | note |
|---|---|---|
| `@@ -314,8 +314,8 @@` `MoveBaseline` doc | `crates/mc-edge/src/plan.rs:127-129` | the type moved to the parent module; same doc text |
| `@@ -2107,7 +2107,15 @@` the `rel` block in `entity_movement` | `crates/mc-edge/src/plan/entity_motion.rs:60-68` | moved verbatim into `plan/entity_motion.rs` |
| `@@ -2144,9 +2152,9 @@` the baseline comment | `crates/mc-edge/src/plan/entity_motion.rs:105-107` | moved verbatim |
| `@@ -2843,10 +2851,18 @@` `delta_to_short` → `relative_move_delta` | `crates/mc-edge/src/plan/entity_motion.rs:154-166` | moved verbatim; `floor_i32` is still used by `tick_index.rs`/`viewer.rs`, so the reversed tree compiles |

Omitted, with the review classes: `crates/mc-edge/src/plan/floor_tests.rs` (10 removed lines) is a
test hunk; `docs/development/protocol-changes.md` and `docs/bugs/ledger.toml` are documentation/ledger.
No a/-side adaptation is invented: every a/-side line is the fix's own pre-image text
(`let rel = (delta_to_short(dx), …)`, the old baseline comment, and `delta_to_short` itself).

## What is missing before promotion

A real-client ref for a closed 1024-cycle relative trajectory (the drift is 0.25 blocks; no scene
drives one). The row stays un-promoted with that exact missing proof; the owner question is CR-5 in
`receipts/D123/D123ClientRest/OWNER-DECISIONS-NEEDED.md`.
