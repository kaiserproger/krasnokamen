# ENTITY-ADMISSION-SCRATCH-ALLOCATED-PER-CHUNK-EACH-TICK — fix-hunk map

The production-only patch `ENTITY-ADMISSION-SCRATCH-ALLOCATED-PER-CHUNK-EACH-TICK.patch` is the
forward-ported fix at HEAD, SHA-256 of its non-test hunks
`patch:651c4479aecc97d85ee6581ffb6d7b49d7b8b453d20b1d5d817972a70b0d0c2b`. It reverse-applies on the
candidate.

## The reversal is the real pre-image

`c393b924068`'s `prepare_rows` parallel path is one hunk, and its pre-image (the file at `c393b9240^`,
`crates/mc-core/src/tick/entity_phase/prepare.rs`) is:

- a local `struct Admission { kept, specials, resting }` inside `prepare_rows`;
- `let parts: Vec<Admission> = periods.par_chunks(PREP_CHUNK).enumerate().map(|…| { let mut part =
  Admission::default(); …; part }).collect();` — one fresh set of three `Vec`s per chunk, on the rayon
  workers, dropped at the end of the tick;
- concatenation from that local vector into `kept`/`out.specials`/`out.resting`;
- `out.parts` is never touched.

The a/-side of the patch is exactly that shape at HEAD: no `parts.resize_with`, no clearing loop, a local
`Vec<AdmissionPart>` collected from `par_chunks`, concatenation from the local vector, and `out.parts`
left alone.

**Single a/-side adaptation:** the local `Admission` type is spelled with the kept `AdmissionPart` type
(whose `Default` increments the test-only counter). It is the same three `Vec` fields in the same order with
the same `Default` (empty) state, so the allocation pattern and the admission behaviour are identical; the
only difference is that the test can count the constructions instead of comparing allocator addresses.

## Why this control is honest

- The kept ref's first assertion is the count after the first prepare (`>= 9`: every chunk of the slot range
  built its buffer set); both the pre-fix and the fixed code satisfy it.
- The second prepare must construct **zero** buffers. The pre-fix tree constructs nine fresh sets here and
  fails `assert_eq!(take_admission_parts_constructed(), 0, "a repeated prepare must not construct admission
  buffers: the pool is reused")` with `left: 9`; the fixed tree reuses the pooled `AdmissionPart`s and
  constructs none.
- `out.parts.len() == 9` and the capacity check are the **API the fix adds**, so they run only after the
  count assertion: a pre-fix tree reds at the count, not at a preserved-API assert (README rule: a red on a
  preserved-API assertion is not evidence).
- The count is exact because nextest runs each test in its own process, so no other test can construct an
  `AdmissionPart` concurrently.

## Omitted hunk

Reason (b) — API the kept refs use: `tick/entity_phase.rs`'s `EntityPhaseScratch.parts` field,
`tick/entity_phase/prepare.rs`'s `AdmissionPart` type (with its test-only counter) and `out.parts`'s
presence are read by `tick::entity_phase::entity_phase_tests::admission_buffers_are_pooled_across_prepares`;
reversing them stops the control from compiling. Everything else in `c393b9240` is another row's behaviour
(reason (a): `entity_phase/step.rs`, `system_graph.rs`, `world.rs`, `battle/*`, `passive_follow.rs`).

## Proving ref

`cargo:mc-core:tick::entity_phase::entity_phase_tests::admission_buffers_are_pooled_across_prepares` — the
control reverses the allocation shape and reds at the `== 0` count assertion with `left: 9`; the restore is
green on the same tree.
