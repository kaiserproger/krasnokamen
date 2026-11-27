# Architecture decisions

The server is `mc-edge`'s `krasnokamen` over the native `mc-core`
([ADR 0013](0013-core-model-v2.md)). Code, tests and runtime evidence beat these
records; a record that disagrees with the code is fixed, not followed. See also
the [architecture overview](../ARCHITECTURE.md) and, for packet work,
[protocol changes](../development/protocol-changes.md).

Every record was revisited against the code on 2026-11-27 (base `973e6e60e`).
"Current truth" is what the tree does, not what the record intends.

| ADR | Decision | Current truth | Evidence |
| --- | --- | --- | --- |
| [0001](0001-vanilla-data-as-runtime-input.md) | vanilla facts come from the operator's own local distribution; explicit import, discovery never downloads | implemented | `mc-edge/src/content_cache.rs:105-176,243-280`, `server.rs:1906` |
| [0002](0002-vanilla-protocol-metadata-as-reference.md) | `javap`-level metadata of the unobfuscated jars is a permitted reference; no copied source or algorithms | in force | `tools/dump-vanilla-protocol.sh`, `mc-protocol/src/packets/configuration.rs:140,579` |
| [0008](0008-overworld-density-router.md) | deterministic, order-independent overworld pipeline; revision fenced by world identity; explicit offline upgrade | implemented | `mc-worldgen/src/lib.rs:146` (`WORLDGEN_REVISION`), `mc-world-tools/src/worldgen_upgrade.rs` |
| [0010](0010-krasnokamen-loader-contract.md) | one untrusted, content-addressed client-bundle contract for Fabric, NeoForge and Forge | implemented in stages: Loader wire 3, artifact index schema 2; player use of generic item declarations not implemented | `mc-loader/src/lib.rs:11,614` |
| [0012](0012-earth-worldplan-and-native-overlay.md) | Earth-like immutable base with native overlays | proposed, not wired; v1 library left in `mc-worldgen/src/earth/`, used only by `worldgen_mosaic` | `mc-worldgen/src/bin/worldgen_mosaic.rs:47` |
| [0013](0013-core-model-v2.md) | one native tick, data-parallel entity work, serial deterministic merge, one WAL/checkpoint recovery boundary, edge owns presentation only | implemented; the governing model | `mc-core/src/tick/merge.rs:593`, `core.rs:1828`, `mc-edge/src/server/pump.rs:55` |
| [0014](0014-v1-core-retirement.md) | retire v1; `mc-loader` and `mc-vanilla` take the shared pieces | complete | no v1 crate under `crates/` |
| [0015](0015-multiversion-protocol-translation.md) | per-session adjacent translation chain in the edge; canonical 26.3, window 26.1.2–26.3 | implemented | `mc-protocol/src/lib.rs:35`, `mc-data/src/protocol_versions.rs:11`, `mc-edge/src/translation/adjacent/` |
| [0016](0016-mob-capabilities.md) | mob types are data: capabilities, plugin blueprints, batched sensors | layers 1–2 and sensors step 1 implemented; operator data directory, Wasm capabilities (layer 3) and persistent sensor subscriptions not | `mc-core/src/blueprint.rs:189`, `mc-script/wit/lifecycle.wit:109`, `mc-core/src/plugin.rs:1613`, `mc-core/src/target_grid.rs:135` |
| [0017](0017-tick-system-graph.md) | tick work as declared systems over native columns | proposed; one-system prototype (`EntityStep`) only | `mc-core/src/tick/system_graph.rs` |
| [0018](0018-in-game-computer.md) | opt-in in-game computer and screen sharing as an edge service | draft implementation, off by default; real-client QA pending | `mc-edge/src/computer.rs:36`, `screen.rs`, `screen_service.rs` |
| [0019](0019-voice-chat.md) | proximity voice relay; Simple Voice Chat bridge | draft implementation; native relay on by default, SVC bridge opt-in; real-client QA pending | `mc-edge/src/voice.rs:53`, `svc_service.rs:65` |
| [0020](0020-entity-checkpoints-at-one-million.md) | entity checkpoints capture only possibly stale rows; explicit tombstones | steps 1–2 implemented; accepted with the strict latency criterion unmet | `mc-core/src/core/capture.rs:246`, `native_checkpoint.rs:1206` |
| [0021](0021-build-time-and-crate-split.md) | build profiles; split the oversized core along cut dependency cycles | owned by the crate-split work (D124); not revisited here | — |
| [0022](0022-content-without-java.md) | derive content without Java on the user's machine | proposed; native `RegistryData` encoder exists but only its tests call it; owner questions open | `mc-data/src/registry_network.rs:287` |
| [0023](0023-offscreen-settlement-simulation.md) | settlements advance off-screen on game time with one owner per accrual | transit, migration flows, game-time clocks, deferred production/consumption implemented; construction labour and births (slices 4–5) not | `mc-core/src/settlements/transit.rs`, `migration.rs`, `offscreen_production.rs` |
| [0024](0024-engine-and-gameplay-packages.md) | the core as an engine, our gameplay as versioned packages | accepted (owner D66); not implemented: the plugin API is still `krasnokamen:plugin@0.9.0` | `mc-script/wit/plugin.wit:11` |

Retired and removed (text in Git history, e.g. `git show 973e6e60e:docs/decisions/0005-regional-simulation.md`):
0003 runtime world lock, 0004 staged single-writer simulation, 0005 regional
simulation, 0006 `mc-net` module boundaries, 0007 v1 connection liveness,
0009 v1 regional plugin boundary and 0011 v1 crate topology. Their retained
principles live in ADR 0013 and the
[runtime map](../architecture/runtime.md#connection-liveness).

## Rules for these records

- A change to authority, threading, waiting, persistence ordering or module
  policy updates the owning record in the same change; an intention is not
  runtime truth.
- A record keeps its decision and binding rules. Implementation detail lives in
  the code and the [architecture pages](../ARCHITECTURE.md#find-the-contract);
  history lives in Git.
- Migration state of a partial, staged or proposed record lives in
  [open transitions](../STATUS.md#open-transitions).
- A new external crate, toolchain, language or runtime, persistent format,
  daemon or service, or required prerequisite binary needs a short record first:
  [new dependencies and components](../development/adding-a-feature.md#new-dependencies-and-components).
