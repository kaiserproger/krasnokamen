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

**A/-side adaptations** (everything the a/-side text adds or phrases differently from the pre-image):

1. the local `Admission` type is spelled with the kept `AdmissionPart` type (whose `Default` increments the
   test-only counter) — same three `Vec` fields, same order, same empty `Default`, so the allocation pattern
   and the admission behaviour are identical; the only difference is that the test can count constructions
   instead of comparing allocator addresses. This is the substitution class the reviewer accepted;
2. the pre-image concatenated with `kept.extend(part.kept)` / `out.specials.extend(part.specials)` /
   `out.resting.extend(part.resting)`; the a/-side keeps HEAD's destructured `specials`/`resting` bindings
   and uses `for mut part in parts { kept.append(&mut part.kept); specials.append(&mut part.specials);
   resting.append(&mut part.resting); }` — same order, same destinations, same consumption of the local
   vector;
3. one `// Pre-fix: …` comment above the local collect (new text, no behaviour).

Nothing else on the a/-side differs: the `par_chunks(PREP_CHUNK).enumerate().map(|…| { let mut part = …; …;
part }).collect()` shape, the `admit_slot` call and the untouched `out.parts` are the pre-image's.

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

## Omitted hunks

Reason (b) — API the kept refs use: `tick/entity_phase.rs`'s `EntityPhaseScratch.parts` field,
`tick/entity_phase/prepare.rs`'s `AdmissionPart` type (with its test-only counter) and `out.parts`'s
presence are read by `tick::entity_phase::entity_phase_tests::admission_buffers_are_pooled_across_prepares`;
reversing them stops the control from compiling.

Reason (a) — other rows' behaviour, hunks of `c393b9240` that are **not** in this patch:

- `crates/mc-physics/src/lib.rs`: `clip_y`'s `1.0E-7` face tolerance (`PHYSICS-CLIP-Y-PARITY`), the
  `COLLISION_EPSILON` early returns on x/z (`PHYSICS-CLIP-MISSING-VANILLA-EPSILON-EARLY-RETURN`), and the
  `ReadRow`-related physics changes;
- `crates/mc-core/src/tick/entity_phase.rs`: `ReadRow`'s field set and the step/tick wiring
  (`ENTITY-READ-ROW-248-BYTES-COPIED-PER-ENTITY-TICK`, `PHYSICS-READER-MEMO-REBUILT-PER-ENTITY-STEP`);
- `crates/mc-core/src/tick/entity_phase/step.rs`, `tick/system_graph.rs`, `world.rs`,
  `tick/passive_follow.rs`, `battle/formation.rs`, `battle/steering.rs`: the reader memo, the row-by-reference
  step and separation work (`PHYSICS-READER-…`, `ENTITY-READ-ROW-…`,
  `BATTLE-SEPARATION-CELL-CAP-DROPS-NEIGHBOURS-UNREPORTED`).

Reversing any of them would turn this control red on another row's assertion and stop that row's kept tests
from compiling.

## Proving ref

`cargo:mc-core:tick::entity_phase::entity_phase_tests::admission_buffers_are_pooled_across_prepares` — the
control reverses the allocation shape and reds at the `== 0` count assertion with `left: 9`; the restore is
green on the same tree.
