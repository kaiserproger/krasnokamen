# PHYSICS-READER-MEMO-REBUILT-PER-ENTITY-STEP — fix-hunk map

The production-only patch `PHYSICS-READER-MEMO-REBUILT-PER-ENTITY-STEP.patch` is the forward-ported fix at
HEAD, SHA-256 of its non-test hunks
`9e7c598e039d53e127a84e944965658f39fb20c5bee090de288a256a2121e20c`.

| Original production change | Current patch equivalent |
|---|---|
| `c393b924068f4211e75f8198a295b5d1058c47d3`: `crates/mc-core/src/tick/system_graph.rs` `execute` — `map_init(|| ctx.world.physics_reader(), …)` per rayon job and one reader for the serial lane, replacing a closure that called `step_entity` (which then built `ctx.world.physics_reader()` itself) per row | Patch hunk 1: `execute` with the pre-fix closure `let step = |row: &ReadRow| row.alive.then(\|\| step_entity(row, ctx, &ctx.world.physics_reader()))` on the a/-side and HEAD's `map_init` + single serial reader on the b/-side. |
| same commit: the `use super::{… ReadRow …}` import was dropped once the closure type no longer needed it | Patch hunk 2: the import line, so the a/-side closure compiles. |

Original non-test exclusions, reason (b) — API the kept ref calls: `crates/mc-core/src/world.rs`
`PHYSICS_READERS`, `take_physics_reader_count()` and the `#[cfg(test)] fetch_add` inside
`physics_reader()`. `tick::system_graph::tests::entity_step_reuses_one_physics_reader_per_lane` reads the
counter, so reversing it stops the control from compiling; the counter stays and counts one reader per row
under the reversed `execute`. No other non-test hunk exists in the commit for this path.

Proving ref: `cargo:mc-core:tick::system_graph::tests::entity_step_reuses_one_physics_reader_per_lane` —
1024 admitted rows must construct fewer than 256 readers; the reversed `execute` constructs one per row.
Also registered and green on both sides:
`cargo:mc-core:world_tests::physics_reader_matches_direct_reads_across_columns_and_overlay` (the reader's
reads stay exact; it does not discriminate the fix on its own).

`step_entity`'s borrowed row and third parameter are `ENTITY-READ-ROW-248-BYTES-COPIED-PER-ENTITY-TICK`'s
hunks and stay on both sides of this patch (reason (a)).
