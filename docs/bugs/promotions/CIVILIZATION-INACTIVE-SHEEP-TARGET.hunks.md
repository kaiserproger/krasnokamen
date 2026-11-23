# CIVILIZATION-INACTIVE-SHEEP-TARGET — HEAD re-break control patch

Patch: `docs/bugs/promotions/CIVILIZATION-INACTIVE-SHEEP-TARGET.patch`
(`patch:e9e14515da90d5c1…`), the row's `fix_commit`. It reverses on the
committed tree (`git apply -R --check`).

## What the fix is (origin `898af4725`, `settlements/construction_availability.rs` only)

The wool-source accounting only counts sheep that are inside the simulated
window: `sheep_colors`/`caps` take the active set, an inactive sheep is no live
wool source, and `needs_shears`/`farmer_raw` follow that set. A shepherd is not
sent to a closer inactive sheep.

## Why only this file

The commit's other production files are **omitted**: `villager/shepherd.rs`
(`find_sheep`/`work` rewrites are the EMPLOYED-SHEARING row's behaviour, reason
(a)), `settlements/layout.rs` (STREET-VERGE), `settlements/construction.rs`
(STREET-BUILDER-RETENTION), `villager/brain.rs` (worked shearing). The row's
cargo ref `settlements::construction::availability::tests::inactive_sheep_is_not_a_live_wool_source`
lives in this file.

## Reverse-apply evidence

`git apply -R --check` on the committed tree succeeds; the real-client red is the
integrated sweep's control run, not claimed here.
