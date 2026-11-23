# PHYSICS-CLIP-Y-PARITY — fix-hunk map

The production-only patch `PHYSICS-CLIP-Y-PARITY.patch` is the forward-ported fix at HEAD, SHA-256 of its
non-test hunks `c28b42e4a9b2f9028180fe46b2298e904e0fb1bcb26ee438f577f997e1bf6c14`.

Regenerated on `1c2e9db153` after `f3cdf12a48` inserted the sub-epsilon early-return guard between
`fn clip_y(…) {` and `for obstacle in collision_boxes {`: that guard is now unchanged **context** in the
hunk (it is `PHYSICS-CLIP-MISSING-VANILLA-EPSILON-EARLY-RETURN`'s behaviour), so the patch still reverses
on the candidate and reverses only this row's tolerance change.

| Original production change | Current patch equivalent |
|---|---|
| `c393b924068f4211e75f8198a295b5d1058c47d3` (`fix(battle,physics,entity): address the hot-path review`): `crates/mc-physics/src/lib.rs` `clip_y` — the doc comment and the `1.0E-7` contact tolerance (`clip >= -COLLISION_EPSILON && clip < dy` / `clip <= COLLISION_EPSILON && clip > dy`) replacing `dy > 0.0 && body.max_y <= obstacle.min_y` / `dy < 0.0 && body.min_y >= obstacle.max_y` | Byte-identical at HEAD (`git show HEAD:crates/mc-physics/src/lib.rs` at lines 1832-1847): the patch's only hunk, whose b/-side is the head body verbatim. This is the row's entire fix. |

Original non-test exclusions, reason (a) — another row's behaviour: every other non-test hunk of
`c393b924068` belongs to `ENTITY-ADMISSION-SCRATCH-ALLOCATED-PER-CHUNK-EACH-TICK`,
`ENTITY-READ-ROW-248-BYTES-COPIED-PER-ENTITY-TICK`, `PHYSICS-READER-MEMO-REBUILT-PER-ENTITY-STEP` and
`BATTLE-SEPARATION-CELL-CAP-DROPS-NEIGHBOURS-UNREPORTED` (`entity_phase.rs`, `entity_phase/prepare.rs`,
`entity_phase/step.rs`, `passive_follow.rs`, `system_graph.rs`, `world.rs`, `battle/formation.rs`,
`battle/steering.rs`). Their reversal would turn the control red on those rows' assertions and stops their
kept tests from compiling. Test hunks (`physics_tests.rs`, `entity_phase_tests.rs`,
`system_graph_tests.rs`, `battle/steering_tests.rs`) stay in the candidate and are not part of the patch.

Proving ref: `cargo:mc-physics:tests::clip_y_snaps_a_body_resting_within_the_contact_tolerance_of_a_face` (`lib.rs` mounts the file as `mod tests`)
(`crates/mc-physics/src/physics_tests.rs`) — a body one ULP inside a floor and a ceiling must be snapped back
out to the face; the pre-fix `clip_y` skips that face and the assertion fails.

The vanilla behaviour is the local javap receipt `receipts/D123/D123Battle/javap/voxelshape-collidex.txt`
(`collideX` `1.0E-7` face interval, reached for every axis through `VoxelShape.collide` + `AxisCycle`).
