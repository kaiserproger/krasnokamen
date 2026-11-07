# Residency, activation and tickets

`crates/mc-core/src/tick/residency.rs`, `world/residency.rs`,
`chunk_tickets.rs` and `forceload.rs` implement the native boundary described
in [ADR 0013](../decisions/0013-core-model-v2.md).

Residency makes a chunk available for storage, hydration and collision reads;
activation permits gameplay. Demand combines player view plus its margin,
active chunks, explicit tickets and dirty checkpoint pins. Simulation distance
does not cap the client's view square. Dirty pins retain state but do not by
themselves activate it. A chunk must be resident and legible to activate.
Previously active chunks have a one-chunk exit band around players, clipped
when simulation distance shrinks. Activation changes are not block-content or
wire revision changes.

Durable remote tickets name owner, area, reason and budget; excess requests are
refused. `/forceload` persists per-dimension tickets and their removal. Forced
and ender-pearl tickets load a two-chunk ring and block-tick a one-chunk ring;
only the center admits entity simulation absent player demand. Pearl tickets
are derived from live pearls with online owners, expire after 40 ticks outside
a chunk, and are not journaled. The per-owner cap is 32 distinct chunks.

Activity clocks and nonzero catch-up debt are native authority, with WAL and
checkpoint fences. They do not accrue merely because a chunk is resident but
inactive, nor from server downtime. Catch-up consumes prior debt before the
live step; checkpoint acknowledgements cannot release newer dirty state.
Malformed or partial persisted debt pairs refuse admission rather than being
normalized. See [persistence](persistence.md) for the recovery boundary.
