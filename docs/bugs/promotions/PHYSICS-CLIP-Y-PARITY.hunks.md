# PHYSICS-CLIP-Y-PARITY — fix-hunk map

The production-only patch `PHYSICS-CLIP-Y-PARITY.patch` is the forward-ported fix at HEAD, SHA-256 of its
non-test hunks `patch:6f14a921444557b0c0c3217978577f7932157c37c849da601abc8b519e4a0eed`. Both hunks
reverse-apply on the candidate (`git apply -R --check` rc 0); the reversal is the pre-fix `clip_y` with no
doc comment.

The patch carries the row's **whole** original non-test change, in two hunks:

| Original production change (`c393b924068f4211e75f8198a295b5d1058c47d3`, `fix(battle,physics,entity): address the hot-path review`) | Current patch |
|---|---|
| `crates/mc-physics/src/lib.rs`: the four-line `///` doc comment above `fn clip_y` (`lib.rs:1820-1823` at HEAD) | Hunk 1 (`@@ -1817,6 +1817,10 @@`): a/-side has no doc comment, b/-side is HEAD's four lines verbatim. |
| `crates/mc-physics/src/lib.rs` `clip_y`: the `1.0E-7` contact tolerance (`clip >= -COLLISION_EPSILON && clip < dy` / `clip <= COLLISION_EPSILON && clip > dy`) replacing `dy > 0.0 && body.max_y <= obstacle.min_y` / `dy < 0.0 && body.min_y >= obstacle.max_y` | Hunk 2 (`@@ -1828,10 +1832,16 @@`): b/-side is HEAD's body verbatim. The sub-epsilon early-return guard added later by `f3cdf12a48` sits **before** this hunk's first context line (the loop body at `lib.rs:1832`), so the reversal does not touch it: the guard's three comment lines are hunk 1's trailing context, and the `if !collision_boxes.is_empty() …` guard itself and the `for obstacle` line lie outside both hunks. |

Original non-test exclusions, reason (a) — another row's behaviour: every other non-test hunk of
`c393b924068` belongs to `ENTITY-ADMISSION-SCRATCH-ALLOCATED-PER-CHUNK-EACH-TICK`,
`ENTITY-READ-ROW-248-BYTES-COPIED-PER-ENTITY-TICK`, `PHYSICS-READER-MEMO-REBUILT-PER-ENTITY-STEP` and
`BATTLE-SEPARATION-CELL-CAP-DROPS-NEIGHBOURS-UNREPORTED` (`entity_phase.rs`, `entity_phase/prepare.rs`,
`entity_phase/step.rs`, `passive_follow.rs`, `system_graph.rs`, `world.rs`, `battle/formation.rs`,
`battle/steering.rs`). Their reversal would turn the control red on those rows' assertions and stops their
kept tests from compiling. Test hunks (`physics_tests.rs`, `entity_phase_tests.rs`,
`system_graph_tests.rs`, `battle/steering_tests.rs`) stay in the candidate and are not part of the patch.

Proving ref: `cargo:mc-physics:tests::clip_y_snaps_a_body_resting_within_the_contact_tolerance_of_a_face`
(`lib.rs` mounts the file as `mod tests`) (`crates/mc-physics/src/physics_tests.rs`) — a body one ULP inside
a floor and a ceiling must be snapped back out to the face; the pre-fix `clip_y` skips that face and the
assertion fails. The early-return ref `a_sub_epsilon_axis_step_is_zero_on_every_axis` stays green in this
control (the guard is untouched), which is what makes the reversal specific to this row.

The vanilla behaviour is the local javap receipt `receipts/D123/D123Battle/javap/voxelshape-collidex.txt`
(`collideX` `1.0E-7` face interval, reached for every axis through `VoxelShape.collide` + `AxisCycle`).
