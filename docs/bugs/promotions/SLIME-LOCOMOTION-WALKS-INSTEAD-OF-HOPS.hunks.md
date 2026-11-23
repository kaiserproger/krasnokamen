# SLIME-LOCOMOTION-WALKS-INSTEAD-OF-HOPS — fix-hunk map

The production-only patch `SLIME-LOCOMOTION-WALKS-INSTEAD-OF-HOPS.patch` is the forward-ported fix at HEAD,
SHA-256 of its non-test hunks
`184f85c33642e65fe73c65a3b7df34af73176f492da525a61c4aee19a3d93e5c`.

Regenerated on `1c2e9db153` to reverse the whole walking behaviour (the round-6 review: only the
`if input.hops` block was reverted, while the hop-tick decision rule and the rest-path refusals stayed on
both sides and the omission list was incomplete). The patch now touches three files: the motion block, the
hop-tick decision rule in `prepare.rs`, and the early rest refusal in `rest.rs`.

The fix landed in two commits on `wt/D123Battle`:

| Commit | What it closes for this row |
|---|---|
| `2c3e1fa2e2a8ac6af9f649dc2917e9ebac2636c2` | `crates/mc-core/src/entity_motion.rs` `step`: the `if input.hops { … }` block — a grounded hopper jumps only on its jump tick with vanilla `getJumpPower()` (+ magma `size * 0.1`), keeps no ground traction between hops, and steers in the air; plus the hop-tick decision rule (`prepare.rs`) and the rest refusals (`rest.rs`). This is the row's symptom (slimes and magma cubes walked). |
| `c7ae9108f2` | Corrects/extends the wiring: `hop_due(…, aggressive)` divides the scaled delay by three for an aggressive body (`Slime$SlimeMoveControl.tick`, javap 26.1.2), `MotionInput.hop_aggressive` carries it, and the rest memo's own hop check was rewritten (see reason (c) below). |

## Original hunk → current patch

| Original hunk | Current patch |
|---|---|
| `2c3e1fa2e2` `entity_motion.rs`: the 41-line `if input.hops { … }` block in `entity_motion::step` | Included: a/-side is HEAD without the block (walker behaviour, exactly the pre-fix code), b/-side is HEAD's block. |
| `2c3e1fa2e2` `tick/entity_phase/prepare.rs` `read_row`: `let think = battle \|\| motion_thinks(…);` moved after the blueprint lookup and extended with `\|\| hop_tick`, plus the `hops`/`hop_delay_scale`/`hop_lift`/`hop_tick` locals and the `hops:` literal field | **Included** for the decision rule: the a/-side drops the `hop_tick` local and restores `let think = battle \|\| motion_thinks(view.tick, flags, period);`. The local field bindings and the `hops:`/`hop_delay_scale:`/`hop_lift:` literal fields are **retained, reason (b)**: `crates/mc-core/src/entity_phase_tests.rs:1499` asserts `row.hops` and `:1474` reads `row.hop_aggressive`, so removing the fields stops the control from compiling. |
| `2c3e1fa2e2` `tick/entity_phase/rest.rs` `early_entity_rest`: the hop-tick refusal before the ring/footprint shortcut | **Included**: a/-side removes it, so a far/mid-ring slime can rest on its hop tick as before the fix. |
| `2c3e1fa2e2` `tick/entity_phase/rest.rs` `idle_rest_proved`: the `(hops, hop_scale)` pair, the hop refusal and the `&& !hops` memo filter | **Reason (c)**: `c7ae9108f2` rewrote that code — at HEAD `idle_rest_proved` carries no hop check at all (its only hop protection is the reversed `early_entity_rest` refusal), so no HEAD equivalent of the memo hunk exists. |
| `2c3e1fa2e2` `tick/entity_phase.rs` `ReadRow.hops`, `c7ae9108f2` `ReadRow.hop_aggressive` | Retained, reason (b): asserted/read by the kept tests above. |
| `2c3e1fa2e2` `tick/dying.rs`, `tick/entity_phase/rest.rs`, `special_mobs/raiders.rs` struct literals gaining `hops: false, hop_delay_scale: 1, hop_lift: 0.0` | Retained, reason (b): they construct the `MotionInput` fields the kept `entity_motion_tests` literals also set; dropping one line leaves a struct literal missing a field. |
| `2c3e1fa2e2` `blueprint.rs` `Capabilities.hops`/`hop_delay_scale`/`hop_lift_per_size` and their `vanilla_capabilities` values | Retained, reason (b): `crates/mc-core/src/blueprint_tests.rs:22` reads `capabilities.hops`, and prepare/step read the other two. |
| `2c3e1fa2e2` `tick/entity_phase/step.rs` `MotionInput` literal `hops`/`hop_delay_scale`/`hop_lift`, `c7ae9108f2` `hop_aggressive` + `hop_due`'s fourth parameter and `/3` divisor | Retained, reason (b): `entity_motion_tests` construct these fields and call the four-argument `hop_due`; their own cadence control is `20261123T155102-test-a0uzfp5l`. |
| `docs/COMPATIBILITY.md` | Documentation, not a hunk. |

## Proving refs and the control's red

Reversing the patch reproduces the walker: a grounded slime gets ground traction every tick and never
leaves the ground, a far mid-ring slime can be rested on its hop tick, and the jump-tick rise is absent.
Red refs on the control:

- `cargo:mc-core:entity_motion_tests::slime_hops_without_walking_between_jumps` — "a slime must leave the
  ground" — the row's headline symptom;
- `cargo:mc-core:entity_motion_tests::hop_tick_rises_the_vanilla_jump_speed_on_its_first_tick`;
- `cargo:mc-core:entity_motion_tests::slime_hop_climbs_a_full_block_step`;
- `cargo:mc-core:tick::entity_phase::entity_phase_tests::far_idle_slime_does_not_rest_on_its_hop_tick`.

**`far_idle_slime_does_not_rest_on_its_hop_tick` does not prove the rest fix.** It goes red in the control
because the hop block is absent (there is no hop to protect), not because a rest path isolated the hop
tick; the rest refusal is reversed in the same control, so no registered ref isolates it. The ref stays
registered as a green restore only; the resting behaviour is covered by the reasoning above, not by this
ref.

Also green on both sides: `entity_motion_tests::an_aggressive_body_divides_its_scaled_jump_delay_by_three`
(the (b) cadence path, whose own control is the recorded `20261123T155102-test-a0uzfp5l`).

Vanilla cadence and launch power are recorded in `docs/COMPATIBILITY.md` and ADR 0016, with the local javap
receipts `receipts/D123/D123Battle/javap/slime-move-control-aggressive-div3.txt`. Source/test row, no
real-client surface.
