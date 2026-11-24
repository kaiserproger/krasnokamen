# SLIME-HOPS-CARRY-NO-WANDER-DIRECTION — fix-hunk map

The production-only patch `SLIME-HOPS-CARRY-NO-WANDER-DIRECTION.patch` is the fix's behaviour hunk at
HEAD, SHA-256 of its non-test hunks
`patch:fa794bf4e20ecb656e08e7764ba19277c73b87b346d26fdbc6bf71f3034881a4`. It reverse-applies on the
candidate (`git apply -R --check` on the fix tree).

## The defect

The lane's SLIME-LOCOMOTION fix made `entity_motion::step` follow vanilla `SlimeMoveControl`: between
hops there is no ground traction, so a hop is a slime's only translation. The direction a hop carried
still came from `entity_motion::wander`, the *walker* drive, whose walk/dwell phase returns
`[0.0; 3]` for 80 of every 120 ticks. A slime therefore translated only on the hop that happened to
fall inside a 40-tick walk window; every other hop was in place, and the net displacement of a
90-tick spawn depends on the two clocks (jump period 10..29 ticks, walk window 40 of 120) lining up
for that entity id. Measured with a temporary trajectory probe on the species fixture: the hops fire
(y 65.0 → 66.25 → 65.0 at ticks 20, 46, 70), x/z move only on the first (0.5 → -0.068 by tick 32)
and stay at -0.0676/0.3054 across the later two.

## The fix

Vanilla keeps a wanted direction for a whole stroll, so a strolling slime's every hop moves it. The
fix splits the walker drive: `wander` keeps the walk/dwell gate for walkers, `wander_heading` is the
same heading without the gate, and `passive_follow::idle` uses `wander_heading` for a hopping body
(`ReadRow.hops`). Nothing else changes: `hop_due`, the jump velocity, the no-traction rule between
hops and the rest/admission path are untouched.

## The hunk the patch carries, and the one it does not

| file | hunk | in the patch? |
|---|---|---|
| `crates/mc-core/src/tick/passive_follow.rs` | `idle` uses `wander_heading` when `row.hops` | **yes** |
| `crates/mc-core/src/entity_motion.rs` | `wander` split; `wander_heading` added | no — kept (see below) |
| `crates/mc-core/src/entity_motion_tests.rs` | the shape test below | no — test file |

**Omitted hunk, reason (b) — API the kept test calls.** The registered shape test
`entity_motion_tests::a_hoppers_wander_heading_survives_the_walker_dwell_phase` imports and calls
`wander_heading`, so a control without the refactor cannot compile. The a/-side therefore keeps the
refactor and reverses only the `passive_follow` branch: `wander_heading` exists but nothing on the
tick path calls it, and a hopping body's drive is the gated `wander` again — exactly the pre-fix
behaviour.

**Proving ref.** `cargo:mc-core:species_tests::every_assigned_species_spawns_locomotes_and_dies_through_native_merge`
— the pre-existing species test that the integrated L2 caught red; on the control the slime hops in
place and the position never changes. The shape test is **not evidence**: it is green on both sides
(the a/-side keeps the ungated helper it calls); it guards the contract, not the wiring.

## What stays out of the row

`wander`'s walker semantics are unchanged (byte-identical body moved into `wander_heading`, gate kept
in `wander`). No change to `hop_due`, jump power, the between-hop no-traction rule, the rest proof
(`idle_wander_motion_unchanged` still models `hops: false`, which stays consistent: on a dwell tick
the real step also does not move because the hop clock, not the drive, is what admits it), or to
battle steering (frozen).
