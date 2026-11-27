# Krasnokamen core architecture

Krasnokamen serves every Minecraft Java release from 26.1.2 through 26.3
(protocols 775–777). The core speaks 26.3; an older session runs through the
edge's adjacent translation chain over the endpoint caches `content import`
installs ([ADR 0015](decisions/0015-multiversion-protocol-translation.md)).
`krasnokamen` is the `mc-edge` binary over native `mc-core`;
[ADR 0013](decisions/0013-core-model-v2.md) governs the core model.

## Composition

- `crates/mc-edge`: CLI, configuration, connections, authentication, packet
  decode and per-viewer presentation; it does not own gameplay state.
- `crates/mc-core`: native tick, gameplay, intents/merge, journal and checkpoint.
- `crates/mc-world-conversion`: offline Anvil import/export, native census and
  probes; `crates/mc-world-tools`: stopped-world pregeneration, prune/compact
  and generation-contract upgrades. Both are above `mc-core`; the edge routes
  operator commands to them. Neither is a core dependency, even for tests.
  Native codecs and recovery stay with the core's `native_checkpoint` owner.
  Live and offline generation share `chunk_loader`'s immutable source and worker
  pool, not a callback into a shared tick state.
- `crates/mc-protocol`, `mc-data`, `mc-world`, `mc-physics` and `mc-worldgen`:
  protocol, content and shared world/physics implementation. `mc-nbt` is the
  shared NBT codec; `mc-domain` holds protocol-independent gameplay
  value types (no transport, persistence or runtime dependency);
  `mc-melee-range` is vanilla melee-reach geometry, dependency-free so the
  server and the replay scanner judge contact with the same code.
- `mc-plugin-host` and `mc-script`: isolated component execution and bounded
  admission; `mc-loader` owns the Loader wire contract.
- `mc-vanilla`: vanilla-jar process control and registry capture for content
  import and the harness; `mc-test-harness`: wire-probe, replay validation and
  other differential test tools; `xtask`: code-health and generated navigation.
- `workspace-features`: compile-time third-party feature unification for the
  server build and core tests ([ADR 0021](decisions/0021-build-time-and-crate-split.md)).
  Only mc-edge's normal dependencies and mc-core's dev dependencies activate it;
  standalone lower crates remain independent. It owns no runtime state and
  depends on no Krasnokamen crate.

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
- [Battles: tick order, facing, steering and combat](architecture/battles.md)
- [Chunk residency, activation and tickets](architecture/residency.md)
- [Adding a vanilla gameplay feature](development/adding-a-feature.md)
- [Horizon `horizon:lod` far-terrain acceptance](development/horizon-lod-acceptance.md)
- [Plugins](PLUGINS.md), [Loader](KRASNOKAMEN_LOADER.md),
  [operations](OPERATING.md), [decisions](../DECISIONS.md)

Generated indexes navigate code; they are not a second source of semantics.
