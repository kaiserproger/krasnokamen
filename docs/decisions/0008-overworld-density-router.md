# ADR 0008 - Overworld generation pipeline

**Status:** Accepted. The revision is owned by `WORLDGEN_REVISION` in
`crates/mc-worldgen/src/lib.rs`; do not pin a number here.

## Decision

Generation is deterministic and coordinate-derived: generating the same or
neighbouring chunks in different orders must not change the result. Landforms,
caves and biome routing are separate stages consumed by chunk assembly, ores,
structures and decoration. Structures precede vegetation; tree support uses the
planned surface rather than a structure or an earlier decoration. Spawn is
selected from generated terrain rather than deforming it.

World identity fences generator revision, seed, mode, dimension geometry and
worldgen profiles. Native core checks the contract before recovery or
generation. A mismatch fails closed rather than silently combining terrain.
Native storage and the tick WAL are the serving authority; Anvil import/export
is explicit offline conversion ([ADR 0013](0013-core-model-v2.md)).

## Explicit operator revision upgrade

With the server stopped, `krasnokamen --config server.toml world upgrade-worldgen`
permits an older revision only when the remaining identity, including seed,
format and dimension geometry, matches. It updates the header and generation
and dimension records in one durable transaction. Existing chunks and gameplay
rows are not rewritten; newly generated chunks use the new revision, so the
operator explicitly accepts a seam. A newer revision, a different identity, a
running server or missing native store is refused; a repeat is a no-op.

## Boundaries and proof

The pipeline is an analogue, not a claim of exact vanilla NoiseRouter or complete
feature parity. See [world generation](../VILLAGE_GENERATION.md),
[architecture](../ARCHITECTURE.md), `crates/mc-worldgen` and its tests for
current details. Changes must preserve deterministic generation and chunk-order
independence, revision/profile mismatch admission, tree support and explicit
Anvil conversion. Visual claims require the applicable
[real-client gate](../development/real-client-gates.md).
