# ADR 0004 - Staged single-writer simulation boundary

**Status:** Superseded by [ADR 0013](0013-core-model-v2.md) and [ADR 0014](0014-v1-core-retirement.md).

The v1 `SimulationOwner` ordered player, world and cross-domain mutations through
a staged single-writer boundary. V1 was retired; ADR 0013 defines the native
snapshot, intent and merge authority that replaces it.
