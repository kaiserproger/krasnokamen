# ENTITY-ADMISSION-SCRATCH-ALLOCATED-PER-CHUNK-EACH-TICK — fix-hunk map

The production-only patch `ENTITY-ADMISSION-SCRATCH-ALLOCATED-PER-CHUNK-EACH-TICK.patch` is the
forward-ported fix at HEAD, SHA-256 of its non-test hunks
`835007acecb930cc524689db7a634fdfcc582a1ddedd0cdd6d61261258e57a37`.

| Original production change | Current patch equivalent |
|---|---|
| `c393b924068f4211e75f8198a295b5d1058c47d3`: `crates/mc-core/src/tick/entity_phase/prepare.rs` `prepare_rows` reuses the pooled `AdmissionPart` buffers (`parts.resize_with(…, AdmissionPart::default)` over the existing vector, then clears each part) instead of allocating a fresh `Vec<Admission>` set per chunk per tick | Patch's only hunk: a/-side takes the previous set out of `out` and rebuilds a fresh set every prepare (`let _stale_parts = std::mem::take(parts);` then `resize_with`), b/-side is HEAD's reuse. |

a/-side adaptation: the single added `let _stale_parts = std::mem::take(parts);` line is the pre-fix
allocation shape — it makes every prepare construct a fresh set of `AdmissionPart`s while the previous set
stays alive for the duration of the pass. It changes no behaviour other than allocating.

Original non-test exclusions, reason (b) — API the kept ref calls: `tick/entity_phase.rs`
`EntityPhaseScratch.parts` and `tick/entity_phase/prepare.rs`'s `AdmissionPart` type (both added by the same
commit, and the test-only `take_admission_parts_constructed` counter counts its `Default`). The kept ref
`tick::entity_phase::entity_phase_tests::admission_buffers_are_pooled_across_prepares` reads `out.parts` and
the counter, so reversing the field or the type stops the control from compiling; they stay.

Proving ref: `cargo:mc-core:tick::entity_phase::entity_phase_tests::admission_buffers_are_pooled_across_prepares`
— over a nine-chunk slot range (`8 * PREP_CHUNK + 64`) the first prepare must construct every pooled
`AdmissionPart` and a repeated prepare of the same population must construct **zero**: it reuses the pool.
The reversed prepare constructs nine fresh parts on the second prepare and fails at
`a repeated prepare must not construct admission buffers: the pool is reused`. The count is exact, so the
assertion does not depend on the allocator handing back the same address (the pointer-identity form of the
check, which the round-6 review rightly called flaky, is gone).

This row's control is assertion-level only after this patch: the original commit's blanket reversal removed
`parts` and made every kept test of the row fail to compile, which `ledger promote` refuses (README
"the `fixed` rule", rule 4). Nothing is promoted on a compile-level control.
