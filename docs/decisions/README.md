# Architecture Decision Index

The current server is `mc-edge`/`solaris-v2` over `mc-core`: [ADR 0013](0013-core-model-v2.md).
Later explicit supersession takes priority over historical descriptions. See also
[architecture](../ARCHITECTURE.md). For a practical packet workflow, see
[protocol changes](../development/protocol-changes.md).

| ADR | Status | Read when… |
| --- | --- | --- |
| [0001](0001-vanilla-data-as-runtime-input.md) | Accepted; ownership updated | importing, licensing or selecting local vanilla content |
| [0002](0002-vanilla-protocol-metadata-as-reference.md) | Accepted | establishing protocol metadata provenance and packet layouts |
| [0003](0003-runtime-world-lock-architecture.md) | Superseded by 0013/0014 | tracing the retired v1 world-lock decision |
| [0004](0004-staged-single-writer-simulation.md) | Superseded by 0013/0014 | tracing the retired v1 simulation owner |
| [0005](0005-regional-simulation.md) | Superseded by 0013/0014 | tracing retired v1 regional ownership |
| [0006](0006-mc-net-module-boundaries.md) | Superseded by 0013/0014 | tracing retired v1 network module boundaries |
| [0007](0007-connection-liveness-under-outbound-pressure.md) | Liveness principle retained; v1 implementation historical | investigating outbound pressure and keepalive liveness |
| [0008](0008-overworld-density-router.md) | Accepted | changing worldgen authority or its revision fence |
| [0009](0009-regional-plugin-boundary.md) | Isolation/value/admission principles retained; v1 implementation superseded | understanding plugin isolation and admission rationale |
| [0010](0010-solaris-loader-contract.md) | Accepted; staged implementation | changing client bundles, Loader negotiation or trust |
| [0011](0011-core-crate-topology-and-threading.md) | V1 topology, threading and durability superseded by 0013/0014 | tracing retired v1 crate and thread ownership |
| [0012](0012-earth-worldplan-and-native-overlay.md) | Proposed for native v2; not ported | evaluating the Earth WorldPlan proposal, not a current serving mode |
| [0013](0013-core-model-v2.md) | Accepted; sole current core model | changing native state authority, tick, persistence, plugins or tickets |
| [0014](0014-v1-core-retirement.md) | Accepted; Phase 5 source retirement complete | understanding the v1 retirement and current crate cutover |
| [0015](0015-multiversion-protocol-translation.md) | Accepted; implemented | changing per-session translation: canonical 26.3, translated 26.1.2 |
| [0016](0016-mob-capabilities.md) | Accepted; layers 1 and 2 and sensors step 1 implemented | changing mob blueprints, capabilities or sensors |
| [0017](0017-tick-system-graph.md) | Proposed graph; bounded one-system prototype implemented | evaluating tick system declarations and row-phase scheduling |
| [0018](0018-in-game-computer.md) | Accepted design; draft implementation, real-client QA pending | changing the opt-in in-game computer |
| [0019](0019-voice-chat.md) | Accepted design; draft implementation, real-client QA pending | changing proximity voice relay |
| [0020](0020-entity-checkpoints-at-one-million.md) | Accepted; steps 1–2 implemented; strict latency criterion unmet | changing entity checkpoints or replay recovery |
| [0021](0021-build-time-and-crate-split.md) | Step 1 accepted and implemented; split pilot designed, not started | changing build profiles or proposing a core crate split |

Update the owning live ADR when authority, threading, persistence ordering or
module policy changes. Superseded ADR tombstones are not current recipes.

Migration state of an ADR whose status says partial, staged or proposed lives
in [open transitions](../STATUS.md#open-transitions), not in this table. A new
external crate, toolchain or component, language or runtime, persistent format,
daemon or service, or required prerequisite binary needs a short decision
record first: [new dependencies and components](../development/adding-a-feature.md#new-dependencies-and-components).
