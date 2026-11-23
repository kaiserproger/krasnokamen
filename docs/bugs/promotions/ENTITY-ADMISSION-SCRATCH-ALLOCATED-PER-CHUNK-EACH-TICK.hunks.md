# ENTITY-ADMISSION-SCRATCH-ALLOCATED-PER-CHUNK-EACH-TICK — fix-hunk map

The production-only patch `ENTITY-ADMISSION-SCRATCH-ALLOCATED-PER-CHUNK-EACH-TICK.patch` is the
forward-ported fix at HEAD, SHA-256 of its non-test hunks
`835007acecb930cc524689db7a634fdfcc582a1ddedd0cdd6d61261258e57a37`.

| Original production change | Current patch equivalent |
|---|---|
| `c393b924068f4211e75f8198a295b5d1058c47d3`: `crates/mc-core/src/tick/entity_phase/prepare.rs` `prepare_rows` reuses the pooled `AdmissionPart` buffers (`parts.resize_with(…, AdmissionPart::default)` over the existing vector, then clears each part) instead of allocating a fresh `Vec<Admission>` set per chunk per tick | Patch's only hunk: a/-side takes the previous set out of `out` and rebuilds a fresh set every prepare (`let _stale_parts = std::mem::take(parts);` then `resize_with`), b/-side is HEAD's reuse. |

a/-side adaptation: the single added `let _stale_parts = std::mem::take(parts);` line is the pre-fix
allocation shape — it keeps the previous tick's buffers alive for the duration of the pass, so the control
really allocates per prepare (no allocator address reuse makes the assertion flaky). It changes no
behaviour other than allocating.

Original non-test exclusions, reason (b) — API the kept ref calls: `tick/entity_phase.rs`
`EntityPhaseScratch.parts` and `tick/entity_phase/prepare.rs`'s `AdmissionPart` type (both added by the same
commit). `entity_phase_tests::admission_buffers_are_pooled_across_prepares` reads `out.parts`, so reversing
the field or the type stops the control from compiling; they stay.

Proving ref: `cargo:mc-core:entity_phase_tests::admission_buffers_are_pooled_across_prepares` — two
prepares of an unchanged 64-row population must report the same `kept` data pointers and capacities; the
reversed prepare allocates a new set and the pointers differ.

This row's control is assertion-level only after this patch: the original commit's blanket reversal removed
`parts` and made every kept test of the row fail to compile, which `ledger promote` refuses (README
"the `fixed` rule", rule 4). Nothing is promoted on a compile-level control.
