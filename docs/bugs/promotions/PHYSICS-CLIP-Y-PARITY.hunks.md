# PHYSICS-CLIP-Y-PARITY — fix-hunk map

The production-only patch `PHYSICS-CLIP-Y-PARITY.patch` is the forward-ported fix at HEAD for the row's
whole symptom, SHA-256 of its non-test hunks `9a3db29e771fadcc8d8708f52ba7b897d935cff7d3d9000c0c5567d6afd15f78`.

| Original production change | Current patch equivalent |
|---|---|
| `c393b924068f4211e75f8198a295b5d1058c47d3` (`fix(battle,physics,entity): address the hot-path review`): `crates/mc-physics/src/lib.rs` `clip_y` — the doc comment and the `1.0E-7` contact tolerance (`clip >= -COLLISION_EPSILON && clip < dy` / `clip <= COLLISION_EPSILON && clip > dy`) replacing `dy > 0.0 && body.max_y <= obstacle.min_y` / `dy < 0.0 && body.min_y >= obstacle.max_y` | Byte-identical at HEAD; the patch's only hunk. This is the row's entire fix. |

Original non-test exclusions, reason (a) — another row's behaviour: every other non-test hunk of
`c393b924068` belongs to `ENTITY-ADMISSION-SCRATCH-ALLOCATED-PER-CHUNK-EACH-TICK`,
`ENTITY-READ-ROW-248-BYTES-COPIED-PER-ENTITY-TICK`, `PHYSICS-READER-MEMO-REBUILT-PER-ENTITY-STEP` and
`BATTLE-SEPARATION-CELL-CAP-DROPS-NEIGHBOURS-UNREPORTED` (`entity_phase.rs`, `entity_phase/prepare.rs`,
`entity_phase/step.rs`, `passive_follow.rs`, `system_graph.rs`, `world.rs`, `battle/formation.rs`,
`battle/steering.rs`). Their reversal would turn the control red on those rows' assertions and stops their
kept tests from compiling. Test hunks (`physics_tests.rs`, `entity_phase_tests.rs`,
`system_graph_tests.rs`, `battle/steering_tests.rs`) stay in the candidate and are not part of the patch.

Proving ref: `cargo:mc-physics:physics_tests::clip_y_snaps_a_body_resting_within_the_contact_tolerance_of_a_face`
(`crates/mc-physics/src/physics_tests.rs`) — a body one ULP inside a floor and a ceiling must be snapped back
out to the face; the pre-fix `clip_y` skips that face and the assertion fails.

The vanilla behaviour is the local javap receipt `receipts/D123/D123Battle/javap/voxelshape-collidex.txt`
(`collideX` `1.0E-7` face interval, reached for every axis through `VoxelShape.collide` + `AxisCycle`).
