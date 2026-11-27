# Decisions

Choices made so far, with reasons. Each one can be revisited: change the row in
the same change that changes the behaviour, and say so in the commit. A row
links a long record in [docs/decisions/](docs/decisions/) only where the design
needs pages. *Proposed* and *not implemented* rows describe an accepted
direction, not what the server does; what works today is in
[docs/STATUS.md](docs/STATUS.md). Rows were checked against the code on
2026-11-27 (`973e6e60e`). Owner decisions are cited by their number (D…); the
full log is private history.

## Product

| # | Decision | Reason |
| --- | --- | --- |
| P1 | A Rust Minecraft Java server written from scratch. Canonical release 26.3; every release from 26.1.2 through 26.3 joins. A version bump adds a translation step and never drops a supported release. | Players keep the client they have (owner, 2026-11-07; [ADR 0015](docs/decisions/0015-multiversion-protocol-translation.md)). |
| P2 | Gameplay and worldgen aim at minecraft.wiki-level similarity, not bit-exact RNG, AI cadence, `Mth` or terrain. | Exact sequential vanilla internals would forbid parallel ticks; players notice mechanics and rates, not RNG streams. |
| P3 | Performance and the absence of crashes, dupes and corrupt saves outrank AI parity. | A server that loses items or worlds is unusable whatever its mobs do. |
| P4 | Work order: ordinary vanilla multiplayer and gameplay; the production plugin API; measured optimization, ownership and autoscaling; rare edges. Lower work moves up only when it blocks play, saves or plugins. | Owner priority. |
| P5 | Make the most of one machine: about 1,000 players and hundreds of thousands of mobs, crowded or spread, and about one million of whatever is hot. The hot set is chosen by access, measured or declared, never by a list of vanilla types, and plugin state gets the same treatment. | Owner D32 addendum (2026-11-18). Targets, not demonstrated capacity. |
| P6 | No released Krasnokamen compatibility surface: migrate the callers and delete old APIs, schemas, authorities, flags and fallbacks. Compatibility targets are the vanilla protocol and Anvil, not our own old saves. | Two live authorities make every later change twice as hard. |
| P7 | An intended difference from vanilla is named with its reason, per case, and its differential check fails once it stops differing (`known_divergence` in `tools/harness/quirk_parity`). | Separates policy from bugs; an unnamed difference is a bug (D129). |
| P8 | The native server (`mc-edge`'s `krasnokamen` over `mc-core`) is the only server. v1 (`mc-server`, `mc-net`, `mc-simulation`, `mc-entity`) was deleted on 2026-10-15 without shims; `mc-loader` and `mc-vanilla` took its shared pieces, every harness lifecycle launches `krasnokamen`, and no old-binary A/B comparator is kept (owner, 2026-10-16). | Keeping v1 duplicated plugin, Loader, harness and merge work. Full record: `git show 973e6e60e:docs/decisions/0014-v1-core-retirement.md`. |

## Content and protocol

| # | Decision | Reason |
| --- | --- | --- |
| C1 | Vanilla facts (registries, tags, recipes, loot, components, block properties, protocol and world format) come from Mojang's distribution on the operator's machine, never from memory or Git. `content import` derives the cache from server jars the operator supplies or downloads with Mojang's pinned size and SHA-1, stages and validates before publishing, and keeps the previous cache on failure; a first start with no cache runs the same import before any world or listener opens. An existing cache is only discovered and validated, never replaced; an invalid configured override fails startup. The cache loads once into `CoreRegistry`: no process-global, compiled-in or second discovered copy; `crates/mc-data/data` fixtures are test facts. No Mojang bytes in Git, crates or bundles (`data/vanilla*` and `.analysis/` stay ignored). | Licensing and correctness. Commands and search order: [operating](docs/OPERATING.md#vanilla-content-cache); code: `crates/mc-edge/src/content_cache.rs` (`prepare_startup`). |
| C2 | Extraction, wire probes, side-by-side runs and `javap`-level metadata of the unobfuscated 26.1+ jars (class and field names and types, record order, enum members, constants, packet registration tables) establish observable behaviour and are a permitted protocol reference. Decompiled source is never translated and algorithms (lighting, RNG, worldgen formulas) are never copied; Rust names are our own. | Since 26.1 the jar is public documentation of the same identifiers the wiki names; a short capture cannot observe rare packets and one wrong id disconnects a client. How to apply: [protocol changes](docs/development/protocol-changes.md#reference-boundary). |
| C3 | Protocol and Anvil layouts are taken from local wire-probe or `javap` evidence, never from memory. | One wrong packet id disconnects a real client. |
| C4 | Older and newer clients go through a per-session adjacent translation chain in the edge (777 → 776 → 775); core authority never branches on protocol; each step owns name-paired mappings and fails closed on what it cannot map. | ViaVersion's proven shape; core stays version-free. [ADR 0015](docs/decisions/0015-multiversion-protocol-translation.md). |
| C5 | Rich client content uses one untrusted, content-addressed bundle contract for Fabric, NeoForge and Forge (Loader wire 3, artifact index schema 2). No guest code runs on the client; a server without client content needs no modded client. Implemented in stages. | Three loader schemas would drift apart; server bytes are not trusted by the client. [ADR 0010](docs/decisions/0010-krasnokamen-loader-contract.md). |
| C6 | *Proposed:* derive content without a jar or JDK on the user's machine. The native `RegistryData` encoder exists but only its tests call it; the owner's questions are open. | Owner goal 2026-11-07: weak machines, no manual jar. [ADR 0022](docs/decisions/0022-content-without-java.md). |

## Core model

[ADR 0013](docs/decisions/0013-core-model-v2.md) is the governing record; the
architecture pages carry it out.

| # | Decision | Reason |
| --- | --- | --- |
| K1 | One native authoritative tick and one cross-domain recovery boundary. Entity work is parallel by data, not by spatial ownership; a serial deterministic merge refuses conflicting or stale work whole. | Spatial regions collapse onto one owner in a crowd and complicate custody; one boundary keeps recovery consistent. |
| K2 | The edge owns connections and presentation, never gameplay authority. Published snapshots are immutable observations; holding one never locks the tick. | One writer; slow readers cannot stall gameplay. |
| K3 | Every cross-thread queue is bounded in bytes or items, or has a written reason and a counter. Reliable results are ordered and bounded and disconnect on exhaustion; they are never dropped silently. Visual-only effects may drop. | Silent loss looks like a gameplay bug and cannot be diagnosed. |
| K4 | Plugin guests run outside the tick, one callback at a time, with owned values and opaque handles. Observing is not committing; the native owner revalidates identity, revision, custody and capability at apply. | A guest must not stall or corrupt gameplay. |
| K5 | A new feature is its own module, or a crate without core internals, owning its state, systems, tests, narrow interface and one host registration point. Roots orchestrate and hold no domain policy. Never grow the largest top-level core module cycle. | Owner 2026-11-16; keeps the core splittable (B1). |
| K6 | No operator worker-thread percentages: capacity is derived once, and measured autoscaling changes bounded budgets. | Hand-tuned percentages go stale with every host. |
| K7 | A narrow measured optimization hack documents its bottleneck, boundary, correctness fence, measured effect and removal path. | Otherwise nobody can tell later whether it still pays. |
| K8 | *Proposed:* tick work as declared systems over native columns. Only a one-system prototype (`EntityStep`) exists; each further step needs owner approval and a measured benefit. | [ADR 0017](docs/decisions/0017-tick-system-graph.md). |

## Persistence

How it works: [persistence](docs/architecture/persistence.md).

| # | Decision | Reason |
| --- | --- | --- |
| W1 | The native redb store plus the tick WAL are the only serving authority. Anvil is explicit stopped-world import/export; no dual write, no fallback. | One recovery boundary; competing authorities break custody. |
| W2 | An append acknowledgement is not an fsync: a crash may roll back an unflushed tail, consistently across domains. No per-tick fsync and no DB write on the tick. | Per-decision fsync before a reply blocks the tick. |
| W3 | Corrupt, incomplete, inconsistent or unsupported required state refuses startup. The one exception is a quarantined chunk, whose bytes stay untouched. | Refusing to start is recoverable; silently lost custody is not. |
| W4 | Our formats have no compatibility read path and no downgrade path. The only conversions are named stopped-world commands (`world upgrade-worldgen`, `world upgrade-game-clock`); otherwise restore a matching backup. | Follows P6. |
| W5 | Live entity checkpoints write only rows that may differ from the store, with explicit tombstones. Accepted with its strict latency criterion unmet. | Full captures cost O(population) per checkpoint. [ADR 0020](docs/decisions/0020-entity-checkpoints-at-one-million.md). |
| W6 | *Not implemented (owner D45):* the journal redesign may reset worlds without conversion; plugins may declare state durable only at checkpoints; a warm standby's lag is accepted per durability class. | Owner answers 2026-11-19, design input for the committee. |

## Residency and simulation

| # | Decision | Reason |
| --- | --- | --- |
| R1 | Residency (stored and legible) differs from activation. Player view, simulation demand, tickets and dirty checkpoint pins decide residency; durable tickets survive a restart, pearl tickets do not; nothing catches up for downtime. | [Residency](docs/architecture/residency.md); ADR 0013. |
| R2 | *Not implemented (owner D45):* every creature far from players is simulated, wild mobs included, with births and deaths. | Owner prefers a living world over vanilla's "what you left is what you find". |
| R3 | Settlements advance off-screen on game time with one native owner per accrual; nothing accrues during downtime or `/tick freeze`. Transit, migration flows, game-time clocks and deferred production are implemented; construction labour and births are not. | [ADR 0023](docs/decisions/0023-offscreen-settlement-simulation.md). |

## World generation

| # | Decision | Reason |
| --- | --- | --- |
| G1 | Generation is deterministic and independent of chunk order. Landforms, caves and biome routing are separate stages; structures precede vegetation; tree support uses the planned surface; spawn is chosen from generated terrain. | Same world from any generation order; an analogue of vanilla's pipeline, not a copy. |
| G2 | World identity fences generator revision (`WORLDGEN_REVISION` in `crates/mc-worldgen/src/lib.rs`), seed, mode, dimension geometry and effective worldgen inputs; a mismatch refuses to open. An older revision upgrades only through stopped-world `world upgrade-worldgen`, which keeps existing chunks and accepts a seam. | Silently mixed terrain cannot be undone. |
| G3 | Datapack worldgen changes apply at restart and never hot-swap; worldgen the generators cannot apply refuses the whole pack. | Generators compile immutable inputs; a half-applied pack is worse than none. |
| G4 | *Proposed, not wired:* an Earth-like immutable base with native overlays. The v1 prototype library sits unused in `crates/mc-worldgen/src/earth/` (used only by `worldgen_mosaic`); adopt or delete is the owner's call. | Full proposal: `git show 973e6e60e:docs/decisions/0012-earth-worldplan-and-native-overlay.md`. |
| G5 | Far terrain for Horizon clients (`horizon:lod`) is served by our server, off by default (`[horizon_lod] enabled`), because far tiles help recover the seed. | Owner D64. |

## Mobs

| # | Decision | Reason |
| --- | --- | --- |
| M1 | A mob type is data: movement class, goals, sensors, capabilities and parameters. Plugins register blueprints; there is no per-type path a plugin type cannot use. Layers 1–2 and batched sensors step 1 are implemented; an operator data directory, Wasm capabilities and persistent sensor subscriptions are not. | [ADR 0016](docs/decisions/0016-mob-capabilities.md). |
| M2 | Data-defined mobs never spawn naturally, and each defining plugin namespace may keep at most 1,024 live custom mobs (revisable after measurement, owner D45). | Native capabilities run on the shared tick and guest fuel does not meter them. |

## Plugins and the engine

| # | Decision | Reason |
| --- | --- | --- |
| X1 | Plugins are independently deployed Wasmtime components (`krasnokamen:plugin@0.9.0`) loaded from `[plugins].directory`; there is no compiled-in plugin list. Production core builds need no sibling repository. | Isolation and independent release; [plugins](docs/PLUGINS.md). |
| X2 | *Accepted, not implemented (owner D66):* the core becomes an engine and our own gameplay (settlements, battles, realms, economy, computer) moves into versioned packages behind named engine services. | [ADR 0024](docs/decisions/0024-engine-and-gameplay-packages.md). |
| X3 | Realms, vassals, tribute and treaties are a plugin over native settlements, towns and economy (owner D62). | Owner D62; not scheduled. |

## Edge services

| # | Decision | Reason |
| --- | --- | --- |
| E1 | The in-game computer and screen sharing are an opt-in edge service, off by default and operator-only; world authority stays in the core. Draft implementation; screen sharing passed its real-client gate (2026-11-26), the terminal has none. | [ADR 0018](docs/decisions/0018-in-game-computer.md). |
| E2 | Proximity voice is relayed by the edge on a Play channel without decoding audio; the Simple Voice Chat bridge is opt-in. Draft implementation; passed its real-client gate with Fabric SVC and native clients (2026-11-26). | [ADR 0019](docs/decisions/0019-voice-chat.md). |
| E3 | Edge limits an operator needs to tune are configuration, not constants: per-address pre-play connections and the per-player chunk send rate. Memory and protocol guards and vanilla timeouts stay in code. | Owner D52, D53. |
| E4 | Play keepalive follows vanilla: one outstanding challenge, a strict 30 s echo deadline, keepalive written before other frames. | [Runtime](docs/architecture/runtime.md#connection-liveness). |

## Build and crates

| # | Decision | Reason |
| --- | --- | --- |
| B1 | Split `mc-core` into real crates along cut dependency cycles, leaves first, callers and tests moved with the code, no facade crates; measure the incremental rebuild before and after. | Owner D124, D128: one 566k-line crate dominates compile time. [ADR 0021](docs/decisions/0021-build-time-and-crate-split.md). |
| B2 | `workspace-features` unifies third-party features for the server build and core tests only, owns no runtime state and depends on no Krasnokamen crate. | Avoids rebuilding dependencies per feature set (ADR 0021 step 1). |
| B3 | Debug builds by default; release builds only for performance, release and real-client gates. | Faster edit loop; a debug server can fall to about 10 TPS while generating chunks, so client gates use a pinned release server. |

## Evidence

| # | Decision | Reason |
| --- | --- | --- |
| V1 | STATUS labels: **Verified** = a real vanilla client gate or a differential test against real vanilla; **Tested** = unit or fixture tests only; **Not run**. *Proposed, owner decision pending:* **Native-gated** = end to end with our native client only (otherwise counted as Tested). | Owner D129; fixtures written by the author prove less than real input. Our own client proves our server and client together, not vanilla compatibility, and is neither of the owner's first two labels. |
| V2 | Gameplay coverage comes from differential tests against a real local vanilla server, not from piles of unit tests. | Owner D60, D107, D128: unit tests written by the implementer kept passing while the feature was broken. |
| V3 | A unit test cannot close a known real-client failure; client surfaces need a real-client gate. | A green unit test has repeatedly coexisted with a broken client. |

## Settlements

Binding owner text: [settlement owner decisions](docs/settlements/OWNER_DECISIONS.md).
Architecture: [settlements](docs/architecture/settlements.md); off-screen
progress: R3.

## Rejected

| # | Alternative | Reason |
| --- | --- | --- |
| N1 | Mutable spatial regions as the unit of parallel ownership (v1). | A crowd collapses onto one owner; cross-region custody complicates correctness. |
| N2 | Sequential vanilla entity execution as the only model. | Not enough parallelism. |
| N3 | Separate domain journals, or fsync per decision before replying (v1). | Inconsistent or blocking recovery. |
| N4 | Guest AI on every entity tick, or synchronous in-tick plugin calls. | A guest stalls gameplay and cost scales with population. |
| N5 | Long-lived v1/v2 dual serving and runtime Anvil fallback. | Competing authorities undermine recovery. |
| N6 | One crate per domain without an enforceable boundary (v1 topology). | Crates that still form one cycle buy nothing; B1 cuts the cycle first. |
