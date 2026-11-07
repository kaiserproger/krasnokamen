# Solaris core architecture

Solaris serves Minecraft Java **26.3** (protocol 777); 26.1.2 clients
(protocol 775) connect through the retained legacy cache and edge translation.
`solaris-v2` is the `mc-edge` binary over native `mc-core`; the v1 server is
retired. [ADR 0013](decisions/0013-core-model-v2.md) governs the core model.

## Composition

- `crates/mc-edge`: CLI, configuration, connections, authentication, packet
  decode and per-viewer presentation; it does not own gameplay state.
- `crates/mc-core`: native tick, gameplay, intents/merge, journal and checkpoint.
- `crates/mc-protocol`, `mc-data`, `mc-world`, `mc-physics` and `mc-worldgen`:
  protocol, content and shared world/physics implementation.
- `mc-plugin-host` and `mc-script`: isolated component execution and bounded
  admission; `mc-loader` owns the Loader wire contract.

Inputs enter native authority; entity workers read immutable snapshots and
emit intents, the owner merges them, and the edge projects the result. World,
player and plugin effects share a native recovery boundary. The edge may
reconcile world delivery to a newer snapshot; it cannot make client prediction
authoritative.

## Find the contract

- [Runtime and thread boundaries](architecture/runtime.md)
- [State ownership](architecture/state-ownership.md)
- [Mutation routes](architecture/mutations.md)
- [Native store, WAL, recovery and backup](architecture/persistence.md)
- [Settlement authority and code locations](architecture/settlements.md)
- [Chunk residency, activation and tickets](architecture/residency.md)
- [Adding a vanilla gameplay feature](development/adding-a-feature.md)
- [Plugins](PLUGINS.md), [Loader](SOLARIS_LOADER.md),
  [operations](OPERATING.md), [decisions](decisions/README.md)

Generated indexes navigate code; they are not a second source of semantics.
