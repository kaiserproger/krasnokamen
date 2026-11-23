# SLIME-LOCOMOTION-WALKS-INSTEAD-OF-HOPS — fix-hunk map

The production-only patch `SLIME-LOCOMOTION-WALKS-INSTEAD-OF-HOPS.patch` is the forward-ported fix at HEAD,
SHA-256 of its non-test hunks
`3413f57c66ceb55b55040b09355bb937c921c93a3da5142dc48296864323de4d`.

| Commit | What it closes for this row |
|---|---|
| `2c3e1fa2e2a8ac6af9f649dc2917e9ebac2636c2` | `crates/mc-core/src/entity_motion.rs` `step`: the `if input.hops { … }` block — a grounded hopper jumps only on its jump tick with vanilla `getJumpPower()` (+ magma `size * 0.1`), keeps no ground traction between hops, and steers in the air. This is the row's symptom (slimes and magma cubes walked). |
| `c7ae9108f2` | Corrects the jump cadence: `hop_due(…, aggressive)` divides the scaled delay by three for an aggressive body (`Slime$SlimeMoveControl.tick`, javap 26.1.2) and `MotionInput.hop_aggressive` carries it. |

| Original production change | Current patch equivalent |
|---|---|
| `2c3e1fa2e2`: the 41-line `if input.hops { … }` block in `entity_motion::step` | Patch's only hunk: a/-side is HEAD without the block (walker behaviour, exactly the pre-fix code), b/-side is HEAD's block. |

Original non-test exclusions, reason (b) — API the kept tests call:
`MotionInput.hop_aggressive`, `hop_due`'s fourth parameter and its `/3` divisor (both from `c7ae9108f2`).
`entity_motion_tests::an_aggressive_body_divides_its_scaled_jump_delay_by_three` calls the four-argument
`hop_due` and `tick/entity_phase/prepare.rs` sets `hop_aggressive`, so reversing them stops the control from
compiling; they stay on both sides. Their own negative control is already recorded for that row path
(`.analysis/validation/20261123T155102-test-a0uzfp5l`: "aggressive delay for scale 1 out of 3..=9: 29") and
it is not what the walking symptom needs.

Proving refs, red in the control: `cargo:mc-core:entity_motion_tests::slime_hops_without_walking_between_jumps`
and `cargo:mc-core:entity_motion_tests::hop_tick_rises_the_vanilla_jump_speed_on_its_first_tick`. Also
registered: `an_aggressive_body_divides_its_scaled_jump_delay_by_three`,
`slime_hop_climbs_a_full_block_step`,
`cargo:mc-core:tick::entity_phase::entity_phase_tests::far_idle_slime_does_not_rest_on_its_hop_tick`.

Vanilla cadence and launch power are recorded in `docs/COMPATIBILITY.md` and ADR 0016, with the local javap
receipts `receipts/D123/D123Battle/javap/slime-move-control-aggressive-div3.txt`. Source/test row, no
real-client surface.
