# ADR 0024 - The core as an engine, our gameplay as packages

**Date:** 2026-11-19  
**Status:** Proposed (owner D66). Independent reviews (committee-sol,
committee-astra) come before acceptance. Nothing here is implemented. The D65
settlements crate (ADR 0021 Step 2, design `receipts/CoreUnload/DESIGN.md`,
outside Git) is already running and is this ADR's first step; this ADR builds
on that design and does not repeat it.  
**Related:** [0009](0009-regional-plugin-boundary.md) (isolation and admission
principles), [0013](0013-core-model-v2.md) (native authority, plugin admission,
hot set), [0016](0016-mob-capabilities.md) (mob blueprints as data),
[0017](0017-tick-system-graph.md) (declared systems),
[0018](0018-in-game-computer.md) (computer), [0021](0021-build-time-and-crate-split.md)
(crate split), [0023](0023-offscreen-settlement-simulation.md) (off-screen
settlements); [plugins](../PLUGINS.md), [plugin contract](../plugins/CONTRACT.md),
[settlements architecture](../architecture/settlements.md).

Citations: `file:line` on base `96e99cfe3` unless another tree is named.
`<work>` is `/run/media/kaiserroman/games/solaris-work`; `receipts/…` and
`.analysis/…` are outside Git on the owner host.

## Decision in brief

1. The core is an engine: vanilla mechanics, world and chunks, entities and
   physics, item custody, persistence (native store + tick WAL), tick
   scheduling, networking. It also keeps a small set of **kernels**: generic
   per-entity or per-cell executors our gameplay needs every tick (bed and
   job allocation, worker execution, construction cell placement, formation
   and combat steps, transit custody). Kernels take data and carry no
   settlement, war or economy policy.
2. Our own gameplay (settlements, armies and battles, realms, economy,
   computer) becomes WebAssembly packages. A package decides at settlement,
   regiment, treaty or command cadence, declares data the engine evaluates per
   tick, and keeps its records in engine-owned storage.
3. The plugin API becomes named, versioned **engine services**, after Roblox's
   `GetService` model: one WIT package per service, semver per service, and
   the service interfaces a package imports are its capabilities. There is one
   current version of each service; a breaking change migrates every caller in
   the same change.
4. No guest code runs per entity, per cell or per hit (ADR 0013 rejected
   alternative, `0013-core-model-v2.md:564-565`). Every move is measured A/B and
   its exact TPS/MSPT change is reported; a large regression goes to the owner.
5. Order: settlements crate (D65, running) → **service cut of the API (the
   first move of this ADR)** → computer → realms (new) → economy → settlements
   onto WIT → armies and battles.

## Context: owner D66

`<work>/receipts/Stage3/OWNER-DECISIONS-20261117-1000.md:567-577`, asked
2026-11-19 ~21:32, answered ~21:50. Owner verbatim: «заодно устраните
дублирующие ядро плагины, и приведите апи в порядок. можно ориентироваться на
такие штуки как платформа Roblox-а например» / «ну и из самого ядра бы
геймплейное повыносить в WIT-файлы, а ядро оставить движком». Form answers:

1. Scope: «Только наш собственный: поселения, армии и бои, державы, экономика,
   компьютер. Ванильные механики (мобы, редстоун, крафт, фермы) остаются в
   движке.»
2. Speed: «Небольшое замедление допустимо ради чистой архитектуры».
3. Settlements: «Сначала Rust-крейт, как в D65, потом перевести его на
   WIT-плагин, когда API будет готово».
4. Duplicate plugins: «Удалять по итогам аудита без твоего просмотра».
   The audit runs separately (`receipts/PluginDupAudit/`); this ADR does not
   decide deletions.
5. When: «Сейчас, параллельно, как D65».

Not stated: how much slowdown is "small", the API shape, the order after
settlements. This ADR proposes them.

The invariants that stay in the engine in every layout: item custody, saves
(native store plus tick WAL, `0013-core-model-v2.md:210-229`) and tick
authority (`0013-core-model-v2.md:504-512`).

## 1. Boundary

### 1.1 The kernel rule

A behaviour that runs per entity per tick, per block cell or per hit is an
engine kernel. Policy that decides at settlement, regiment, treaty or player
command cadence belongs to a package. The rule is the original ADR 0013 Q7
("Plugins compose mobs from native core building blocks … as data. Core
executes them natively each tick. Plugin code runs only on rare decisions.
Missing behaviour is added to the core library.", `git show
1ada2ceb9:docs/decisions/0013-core-model-v2.md` lines 21-22) applied to our
own gameplay. It keeps the current rejection of "guest AI on each entity tick
or synchronous in-tick plugin calls" (`0013-core-model-v2.md:564-565`) and ADR
0017's "no plugin scheduling API or second mutable authority"
(`0017-tick-system-graph.md:11-15`). ADR 0016 blueprints are the existing
example: a plugin declares a mob as data and the engine runs it.

Engine kernels today (they exist; the moves below strip policy out of them,
they add no new runtime):

| Kernel | Code today | Policy that leaves it |
|---|---|---|
| bed/job/meeting-point allocation | `settlements/allocator.rs` (2,280 lines), shared tick budget `tick_budget.rs:8-9` | which settlement owns which building, admission rules |
| worker execution (mining, lumber, harvest, smelting, building, hauling) | `villager/brain.rs` (1,577) and the villager AI submodules; generic work orders already in WIT (`npcs.wit:190-271`) | what work a settlement orders and when |
| construction cell placement with custody | generic executor in `plugin_structures` (1,732 + 3,329 lines); settlement policy in `plugin_structures/settlement_builds.rs` and `settlements/construction*.rs` (9,592 lines) | the split is CoreUnload S7 (`DESIGN.md` §2.5) |
| formation, steering, targeting, combat, archery, cavalry | `battle/` (13,025 prod lines) | rosters, staging, order vocabulary, war rules |
| transit custody | `settlements/transit.rs` (1,097) | migration attractiveness and timing |
| rules as data: protected zones, faction relations | `plugin_player::protects_cell` (zone allow-lists), `battle::faction` | who is allowed, when (war windows) |

### 1.2 Our systems

Lines are production lines (`*_tests.rs` excluded) on the base.

**Settlements, including villager work AI.**
- Today: `crates/mc-core/src/settlements/` 39,669 prod / 59,614 test lines
  (126 files); of the prod lines, presentation 8,193 (`ui_menus.rs`, `ui.rs`,
  `ui_map.rs`, `ui_text.rs`, `presentation.rs`), construction 9,592, economy
  6,297 (below). The D65 move set (with villager AI, `plugin_structures`,
  `plugin_settlements`, the command handler) is 54,940 prod lines
  (`DESIGN.md` §1.4).
- Engine services needed: World (blocks, structure executor, zones), Entities
  (binding, membership, work orders, paged NPC reads), the allocation kernel,
  Inventory (storage endpoints, carry, equipment), Scheduler (game time, daily
  phases), Storage, Presentation (dialogs, menus, map items, labels),
  Commands, Content (blueprints, recipes, items).
- State and where it persists today: `settlement/<id>` and
  `settlement_sequence` checkpoint rows plus `JournalEntry::Settlement` WAL
  images, `transit/<uuid>` rows (`DESIGN.md` §1.3,
  [settlements architecture](../architecture/settlements.md) "Durability and
  plugin boundary"); the structure ledger as plugin-domain KV keys
  (`0013-core-model-v2.md:326-331`); membership and work cursors in the
  villager entity column (`DESIGN.md` §1.2). Target: records become
  package-owned rows in engine storage (§1.3); the CoreUnload C1 opaque
  payload with a domain-owned codec (`DESIGN.md` §2.8, P2) is the bridge.
  Membership and cursors stay in the engine column as a per-entity package
  attachment. The structure ledger stays with the generic executor.
- Hot paths per tick: `settlements::tick` once per dimension at the commit
  tail (`tick/dimension.rs:465-478`): the allocator under a global budget of
  256 decisions and 4,096 path nodes shared with the brain
  (`tick_budget.rs:8-9`), 8 POI chunks (`allocator.rs:30`), at most 8 projects
  and 8 orders per settlement (`construction.rs:58,60`), army, transit and
  migration (one evaluation per settlement per game day,
  [settlements architecture](../architecture/settlements.md) "Migration policy
  and physical return"); autonomy at chunk environment
  (`tick/dimension.rs:191`: 4 villages, 8 + 2 region queries, 32 binds,
  `autonomy.rs:18-21`); the villager brain right after the allocator
  (`tick/dimension.rs:470`). Production dispatches at most 16 jobs per tick,
  payroll at most 16 contracts per settlement per tick
  ([settlements architecture](../architecture/settlements.md)).
- Measured: `villager-heavy` (20,000 villagers + 100,000 mobs, 100 bots) MSPT
  p50 140.1 / p95 157.0 ms, 7.12 TPS, laps not instrumented
  (`docs/performance/PROFILES.md:23`); `native-100` without villagers
  `commit.settlements` p50 0.03 ms (`receipts/PerfAfterRename/phases-c.md:109`).

**Armies and battles.**
- Today: `crates/mc-core/src/battle/` 13,025 prod / 12,742 test lines
  (39 files); `settlements/army.rs` 430; `villager/soldier.rs` 273.
- Engine services needed: Entities, the formation and combat kernels, World
  (ground), Presentation (spectator bindings, views), Commands, Content
  (weapons as items, `lifecycle.wit:69-83`; mob blueprints).
- State: the battle owner image (phase, battle clock, regiment and member
  identity, orders, morale and weapon timers) in the entities' checkpoint and
  WAL frame, full image first after a checkpoint and patches after
  (`0013-core-model-v2.md:231-247`); `Soldier` in the villager column
  (`DESIGN.md` §1.2). Target: battle images stay kernel state in the engine;
  rosters, orders and war policy become package records.
- Hot paths: `battle::step` every tick in the entity phase
  (`tick/entity_phase.rs:186-188`) over at most 20,000 soldiers in 128
  regiments (`battle/formation.rs:45,48`); retarget every 10 ticks
  (`battle/targeting.rs:256`), formation decisions every 10
  (`battle/formation.rs:11`), attack decisions every 20 (`battle/combat.rs:16`),
  so about 2,000 retargets and 1,000 attack decisions per tick at 20,000
  soldiers [INFERENCE: soldiers / cadence, assuming staggering];
  `soldier::attacks` at the entity tail (`tick/dimension.rs:268`).
- Measured: 22,000 units MSPT p50 25.1 / p95 35.0 ms, entity phase mean
  15.5 ms, `battle::steering::step` 3.3 ms per tick
  (`receipts/BattleMsptRnD/OWNER-REQUEST.md:20-34`); a 10k battle p50 38.1 /
  p95 85.9 ms (`receipts/BvBattleLethality/TENK-COST.md:29`).

**Realms (new, nothing to move).**
- Rules: owner D62 ([owner decisions](../settlements/OWNER_DECISIONS.md)
  "Realms, vassals, tribute and sieges", lines 363-389); plan and core API gaps:
  `receipts/Stage3/D62-REALMS-PLAN.md` (steps 0-7, lines 12-22; API points,
  lines 33-69).
- Engine services needed: Settlements (query, steward, war declaration and
  outcome, muster), Players, Storage, Inventory (tribute is a real transfer),
  World zones with a war-window exception as data, Presentation (map item,
  dialogs), Commands, Scheduler.
- State: treaties, tribute schedules, ransoms, the chronicle as package
  records; war state in the settlement records (`DESIGN.md` §2.7).
- Hot paths: none per tick; command cadence, 10–200 players
  (`OWNER_DECISIONS.md:382`). Damage legality and protection are evaluated
  natively from package data, never by a guest call per hit (§3.3).

**Economy.**
- Today native, in `settlements/`: `production.rs` 1,695, `caravan.rs` 1,173,
  `commerce.rs` 1,144, `economy.rs` 508, `autocraft.rs` 457,
  `economy_customization.rs` 416, `offscreen_production.rs` 362,
  `economy_commands.rs` 298, `production_delivery.rs` 244 (6,297 lines).
  Packages: `krasnokamen-economy` (633 source lines, the virtual-money ledger)
  and `basic-economy` (1,127, shops through inventory menus and
  inventory-storage transactions), in `solaris-default-plugins` 38a9c30.
- Engine services needed: Inventory (atomic inventory plus storage
  transaction, `commands.wit:395-402`), Entities (work orders and caravan
  movement), Scheduler, Storage, Presentation (menus), Commands.
- State: economy policy in each settlement record's post-image, goods in native
  containers and villager carry (Decision 006 S4, `git show
  1ada2ceb9:docs/decisions/0013-core-model-v2.md` lines 101-158); money in plugin
  KV ledgers. Target: policy and ledgers in package records; goods stay
  engine custody.
- Hot paths: the production and payroll cursors above, daily consumption once
  per 24,000 game ticks per settlement; production portions and caravan
  walking are kernels (craft receipts `npcs.wit:591`, orders `npcs.wit:374`).

**Computer.**
- Today: `crates/mc-edge/src/computer.rs` 459, `screen.rs` 1,091,
  `screen_service.rs` 248 lines; the `computer` package is a client content
  bundle only (`computer/plugin.toml`, 36 lines of guest source).
- Engine services needed: Net (a Play channel), World (block-use events),
  Players (operator level, reach), Presentation. The host-process bridge stays
  an edge service: ADR 0018 refuses guest process spawning
  (`0018-in-game-computer.md:14-20`).
- State: none persisted (`0018-in-game-computer.md:47-48`).
- Hot paths: none in the tick.

### 1.3 What persists where

- Custody (items, bodies, blocks) and its revisions: engine, in every layout.
- Package records: engine-owned storage per package, journaled in the same
  tick WAL frame as the custody effects of the same request ("Plugin KV and
  native effects share the tick journal", `0013-core-model-v2.md:509-511`), with
  the checkpoint-only class owner D45 allows (`0013-core-model-v2.md:553-555`).
  Today's plugin values are capped at 4,096 bytes and keys at 128
  (`crates/mc-script/src/limits.rs:63,66`); settlement rows go to the 16 MiB
  checkpoint envelope (`crates/mc-world/src/native/checkpoint.rs:30`). Package
  storage needs rows at that bound before any settlement record leaves the
  engine.
- Per-entity package data (membership, work cursors): an attachment in the
  engine's entity column, persisted with the entity image, never a package
  side table keyed by entity id (rejected in `DESIGN.md` §2.3).
- Kernel state (battle images, allocator claims, transit records): engine.

## 2. API: named, versioned engine services

### 2.1 Today: `krasnokamen:plugin@0.9.0`

- 18 WIT files, 3,835 lines in `crates/mc-script/wit/`. The world imports
  `host` and `command-permissions` and exports `lifecycle`, `events`,
  `precommit` (`plugin.wit:11-19`).
- Effects are return values: `on-events` returns `list<command>`
  (`events.wit:640`); `command` is one variant of 21 cases
  (`commands.wit:526-574`), `event` one variant of 30 (`events.wit:586-635`);
  domain families go through one `operation` envelope
  (`domain-operations.wit:10-25`). Imports are deliberately tiny: "no import
  here may take a world lock, wait for a chunk, or block on I/O"
  (`host.wit:1-6`).
- One version for everything, matched exactly
  (`crates/mc-script/src/version.rs:41-48`, checked at
  `manifest.rs:872`): any WIT change rebuilds every package
  ([plugin contract](../plugins/CONTRACT.md), "The contract is").
  Capabilities are 19 strings (`manifest.rs:301-319`).
- Production use: the first-party packages send messages, use storage
  (get, CAS, batch CAS, operation status), zones, teleport, timers, inventory
  menus and inventory-storage transactions, and read player, command, zone,
  timer and world observation events. No production package uses `npcs`,
  `npc-routes`, `settlements`, `world-structures`, inventory operations,
  `client-presentation`, `spawn-entity` or `precommit`; only the host fixture
  `sdk/rust/examples/hello/src/{npc_ops,settlement_ops,world_structure_ops,inventory_ops}.rs`
  does. `before-damage` exists in WIT (`precommit.wit:146`) but the runtime
  admits only build hooks (`crates/mc-core/src/plugin.rs:636`).
- Gaps our gameplay needs: dialog clicks ([dialogs](../DIALOGS.md), line 88),
  Play payload channels and block-use callbacks (`0018-in-game-computer.md:14-17`),
  war, army, steward and map points (`D62-REALMS-PLAN.md:33-69`).

### 2.2 Shape

- **Service = one WIT package** `krasnokamen:<service>@<semver>` with one
  interface per capability. Request functions are imports that only enqueue
  and return a request id; they keep the `host.wit:1-6` rule. Each service
  has one exported handler interface for its events.
- **One callback, one batch.** Whatever a callback enqueues, across services,
  is staged and admitted as one batch when it returns, exactly as today
  (`crates/mc-plugin-host/src/host/delivery.rs:28-35`), and applied at the next
  merge (`0013-core-model-v2.md:504-512`). Hook phases refuse request imports,
  as now (`precommit.wit:12-17`).
- **Order.** Events carry the global `sequence` (`types.wit:46`). The host
  delivers them in sequence order and coalesces consecutive events of one
  service into one handler call.
- **Capabilities.** The manifest's `[services]` table names each service, its
  version and its interfaces; the host links only those, so an undeclared
  import fails instantiation (the component linker defines nothing else).
  The operator reviews that list. It replaces `api` and the 19 strings.
- **Runtime service** `krasnokamen:runtime`: lifecycle, scheduler (game-tick
  timers, `commands.wit:340-369`), log and identity, the shared failure enum
  (`operation-types.wit`). There is no per-tick callback: Roblox's
  `RunService.Heartbeat` runs scripts every frame, which `0013:564-565` rejects.

| Service | Interfaces | From today's WIT | Roblox analogue |
|---|---|---|---|
| `runtime` | lifecycle, scheduler, log, types | `lifecycle.wit` (minus the contribution), timers in `commands.wit`, `host.wit` log/id, `types.wit`, `operation-types.wit` | `RunService` (without per-frame scripts) |
| `content` | items, recipes, block tools, worldgen rules, mob blueprints | `lifecycle.wit:13-110` startup contribution | packages and assets |
| `players` | roster, messaging, teleport, session, auth | send-message, broadcast, list-online-players, teleport, disconnect, unlock-session; `host.wit` password functions; join/leave/chat/death events | `Players`, `TextChatService` |
| `commands` | roots, permissions, invocations | `command-permissions.wit`, command-invoked and batch-rejected events | chat commands |
| `storage` | kv, transactions, operation status | `storage.wit`, storage commands, `types.wit:29` version guards | `DataStoreService`, but atomic with custody and WAL-durable |
| `world` | blocks, zones, structures, observations, build rule, time | `world-structures.wit`, `world-events.wit`, zone commands, `precommit.before-build` | `Workspace` |
| `entities` | npcs, work, orders, routes, spawn | `npcs.wit`, `npc-routes.wit`, spawn-entity | `Humanoid`, `PathfindingService` |
| `inventory` | endpoints, transfers, reservations, transactions, menus | `inventories.wit`, inventory-storage transaction, inventory menus | — |
| `presentation` | Loader views, dialogs, sounds, map items, payload channels | `client-presentation.wit`, plus the gaps above | `ScreenGui`, `RemoteEvent` |
| `settlements` | query, actions | `settlements.wit` | — (game-specific) |

References: Roblox services through `DataModel:GetService`
(<https://create.roblox.com/docs/reference/engine/classes/DataModel>,
<https://create.roblox.com/docs/scripting/services>), per-script capability
sandboxing (<https://create.roblox.com/docs/scripting/capabilities>).

### 2.3 Versions and breaking changes

- Before 1.0 every change to a service bumps its minor (0.x semver is
  breaking). From 1.0: a new function, interface or type is a minor; any change
  to an existing function, record or variant, including a new case in an event
  variant, is a major.
- The host serves one version of each service. A breaking change lands as one
  change set: WIT, host, SDK, and every first-party consumer rebuilt in
  `solaris-default-plugins`; release notes name the service and both
  versions. A package built against another major fails startup with the
  service name and both versions (fail closed, as the exact match does now).
  No adapters, no second version, no deprecation window (AGENTS.md: no
  compatibility surface of our own). This is where we depart from Roblox, which
  keeps deprecated APIs callable.
- Wasmtime 36 (`Cargo.lock:4055-4056`) links a guest's versioned import to a
  semver-compatible host interface and still type-checks it exactly
  (<https://docs.rs/wasmtime/latest/wasmtime/component/struct.Linker.html>).
  An additive minor therefore needs no rebuild; that is the component model's
  rule, not a layer we keep.
- Gain over today: a change to one service rebuilds only its importers.

### 2.4 What the cut deletes

- `package krasnokamen:plugin@0.9.0` and its world (`plugin.wit:11-19`), the
  manifest `api` key and the exact match (`version.rs:41-48`).
- The `command` and `event` variants and `on-events`
  (`commands.wit:526-574`, `events.wit:586-640`), replaced by per-service
  request imports and handler exports.
- The `domain-operations.wit` envelope (lines 10-25); its storage version guard
  becomes part of a storage transaction.
- The 19 capability strings (`manifest.rs:301-319`).
- Dead contract: `precommit.before-damage` (refused at runtime,
  `plugin.rs:636`) and `hook-actor.environment`, which no path produces
  (`D62-REALMS-PLAN.md:55-61`). Damage legality for realms and war comes back
  as engine data rules (§3.3), not a per-hit hook.
- Families without a production consumer (§2.1) stay only where a move in §4
  consumes them; the cut lists each with its consumer and deletes the rest.

### 2.5 Services provided by a package

When settlements leave the engine (§4 step 6), the `settlements` service is
provided by the settlements package: the host routes a consumer's request (for
example from realms) to the provider as an event and the provider's answer
back, under the same batch admission; custody effects still go through engine
services. This routing is built with that step, not before; until then the
engine serves `settlements` (`crates/mc-core/src/plugin_settlements.rs`).

## 3. Performance guard

### 3.1 Cost model of the boundary

For one move, per tick:

- **Tick thread (adds to MSPT):** building the events the package subscribed
  to from committed observations, plus converting and applying its admitted
  requests at the merge, plus native evaluation of the data rules it
  declared. All three are O(events + requests), never O(entities).
- **Host thread (off the tick, competes for cores):** calls × fixed call cost
  (fuel refill and epoch re-arm, `crates/mc-plugin-host/src/instance.rs:12-14,123-132`;
  canonical-ABI entry and exit) + bytes lifted and lowered × per-byte cost +
  guest work × the Wasm-to-native factor. One thread serves every package, one
  callback at a time (`crates/mc-plugin-host/src/host.rs:1-10`,
  [plugins](../PLUGINS.md) "Slow callbacks"). On the reference box the server
  gets four physical cores (`PROFILES.md:42-44`), shared with the parallel
  entity phase, so host CPU is not free.
- **Latency:** at least one tick from event to effect; a request can be
  refused as stale ([plugins](../PLUGINS.md) lines 12-13).
- **Hard bounds:** 32 commands per callback, 256 queued commands, 1,024 queued
  events (`crates/mc-script/src/limits.rs:29,31,33`); 8 MiB per host call,
  50 M fuel per call, a 30 s wall guard
  (`crates/mc-plugin-host/src/limits.rs:80-100`); past a quarter of the bound
  the core refuses new plugin commands, and unrefusable events past the bound
  halt the world ([plugins](../PLUGINS.md) "Slow callbacks").
- **Unmeasured:** the fixed call cost, the per-byte cost and the Wasm factor.
  The host's limits are "deliberately conservative starting points, not
  measured budgets" (`crates/mc-plugin-host/src/limits.rs:9-12`); no boundary
  benchmark exists. The first move measures them (§4.1).

### 3.2 Against A3

A3 is 100+ players and 100k+ freely roaming mobs at 20 TPS
(`docs/DEFINITION_OF_DONE.md:29`): 50 ms per tick. Today `native-100` runs at
p50 63.45 ms, 15.17 TPS and `villager-heavy` at p50 140.1 ms, 7.12 TPS
(`PROFILES.md:22-23`). The tick is already over budget, so a move may add no
per-entity work to the tick thread, and its host-thread work must fit in spare
core time on that box.

- Per-entity guest decisions cannot fit by construction: 20,000 soldiers make
  about 3,000 target and attack decisions per tick (§1.2, [INFERENCE]) and 20,000 villagers
  share 256 decisions per tick, against 256 queued commands in total.
- Package-cadence decisions fit: 1,000 settlements evaluated once per game
  day are 0.04 evaluations per tick; at most 128 regiments issue orders on
  events; realms act on player commands.

### 3.3 Rules for gameplay packages

1. No guest call per entity, per cell or per hit; kernels stay native.
2. **Rules as data.** Protection zones with war-window exceptions, faction
   relations, recipes, blueprints, morale and wage parameters are tables a
   package pushes; the engine evaluates them per hit or edit. Precedents:
   zone allow-lists enforced natively for everyone
   (`solaris-default-plugins` `krasnokamen-towns/README.md:5`), ADR 0016 blueprints, the zone gate in
   `D62-REALMS-PLAN.md:63-69`.
3. **Group requests.** One request per regiment, household or settlement with
   a member list and per-member outcomes (`npcs.wit:374`, `npcs.wit:665`).
4. One handler call per service per delivery round, events coalesced (§2.2).
5. **Due-driven.** Policy runs on scheduler timers at its own cadence, never
   by polling every tick.
6. **Filtered and aggregated observations.** The engine builds only events a
   package subscribed to, sheds observations per package as now
   ([plugins](../PLUGINS.md) "Slow callbacks"), and reports aggregates (for
   example casualties per regiment per interval) instead of per-hit events.
7. **Paged snapshot reads** (`settlement-page`, `npc-page`) instead of
   per-entity queries.

### 3.4 A/B rule for every move

- Pins B (before) and A (after) from `tools/dev/pin-release --no-incremental`,
  frozen trees named in the receipt, same box, a quiet window
  (`tools/dev/mem-run.sh -x -p` inside `tools/dev/measurement-windows`).
- Three alternating pairs B, A, B, A, B, A. Report TPS, MSPT p50/p95/p99, the
  laps of the moved systems (`MC_CORE_PHASE_LOG`), host-thread CPU, callback
  p50/p99, peak backlog and refusal counts; per-pair deltas and spread, never a
  cross-pair mean (`DESIGN.md` §4).
- Profiles per move: API cut → `native-100`, `explore-20`; computer →
  `explore-20` with open terminals; realms → a 200-player profile
  (`D62-REALMS-PLAN.md:25-26`); economy and settlements → `villager-heavy`,
  `core_bench peaceful100`, one Spain trial (`DESIGN.md` §4); battles → the
  10k and 22k battle takes (`tools/battle/run-battle.py`) and the battle tick
  micro test.
- Verdict (proposed; D66 did not size "small"): inside the pair spread is
  neutral; worse than the spread in all three pairs but under 5% on p50 and
  p95 is small, reported and landed; 5% or more, or TPS falling below 20 on a
  profile B holds at 20, is large and goes to the owner before it lands.

## 4. Migration order

| Step | What | Depends on |
|---|---|---|
| 0 (running) | D65 settlements crate: CoreUnload S0–S8, then C1–C3 (`DESIGN.md` §3.2-3.3); duplicate-plugin deletions by the audit (D66.4) | — |
| **1** | **Service cut of the plugin API** (§4.1) | — (parallel with 0) |
| 2 | Computer as a package: block identity, block-use events, the terminal and screen UX through `presentation` payload channels; the process bridge stays in `mc-edge` | 1 |
| 3 | Realms (new package, D62): treaties, tribute, ransom, chronicle, map; its core points (steward, native war, muster) are written against the owned settlement state (`DESIGN.md` §2.7) and exposed through `settlements` | 1; D62 plan steps 0-1; owner GO for D62 |
| 4 | Economy as a package: settlement economy policy (6,297 lines) and the money/shop packages after the audit; goods stay engine custody | 1; D65 C1 (opaque records); storage rows at the native bound; inventory transactions that also carry package records |
| 5 | Settlements policy onto WIT (D66.3): founding, growth, certification rules, households, districts, migration rules, presentation, commands; kernels stay | 1, 4; D65 C3 with P2; per-entity attachments; package-provided services (§2.5) |
| 6 | Armies and battles: rosters, staging, order vocabulary, war policy; the battle kernels and images stay | 5; independent battle-core review (AGENTS.md rule 5) |

Why this order: the computer is the smallest move, has no tick cost, and
proves the presentation and block-event services. Realms has no code to move
and becomes the first real consumer of `settlements`. Economy is the
lowest-cadence, custody-heavy part of settlements and proves package records
committed atomically with custody. Settlements need everything before them.
Battles are the hottest code and go last. Steps 2 and 3 can run in parallel
after step 1.

### 4.1 First move: the service cut

- **Change.** Split `crates/mc-script/wit/` into the service packages of §2.2,
  each at `0.1.0`. Records move verbatim; only the envelopes change (request
  imports and handler exports instead of the `command`/`event` variants).
  `mc-plugin-host` links per declared service, coalesces delivery by service
  in sequence order and keeps the batch staging of `delivery.rs:28-35`.
  `mc-script` replaces `api`, the exact match and the capability strings with
  the `[services]` table. The SDK (`sdk/rust/krasnokamen-plugin-sdk`), its
  examples, [plugins](../PLUGINS.md), [plugin contract](../plugins/CONTRACT.md)
  and [SDK](../plugins/SDK.md) follow. Every package the audit keeps in
  `solaris-default-plugins` is rebuilt and the live pack and matrix pins are
  re-pinned. The dead contract of §2.4 is deleted.
- **Measurement in the same slice.** A boundary bench in `mc-plugin-host`:
  a no-op callback, 1/32/256 events of fixed size, 32-request answers and a
  byte sweep, on the reference box in the `-p` lane, before (0.9.0) and after
  the cut. It yields the fixed call cost, the per-byte cost and a Wasm factor
  for a fixed guest loop, which every later move's budget uses.
- **Acceptance.** L2 green; the plugin regressions and the standard-pack load
  test pass with the same package set as before the cut (less the audit's
  deletions); the A/B of §3.4 on `native-100` and `explore-20` is measured and
  its TPS/MSPT change reported, and the §3.4 verdict applies (a small slowdown
  lands, a large one goes to the owner first); bench
  receipts before and after. The cut changes the admission path of
  inventory transactions, so it gets an independent review before Main
  commits it (AGENTS.md rule 5, custody).
- **Not in it:** no new capability, no behaviour change in the tick, no move
  of gameplay. New D62 settlement operations land after it, as
  `krasnokamen:settlements` minors, rebuilding only their importers.

## 5. Why settlements left the plugin in October, and what changed

### 5.1 History

- 2026-09-15: `solaris-settlements` replaced the prototype packages
  (`solaris-default-plugins` b01342b): one package owning settlement content,
  growth, economy, population, residents and squads, API 0.6.0, ~6k lines of
  Lua with "versioned, sharded records written with their derived indexes in
  one atomic `storage_batch_cas`". Its contract made the core a "generic engine
  + reusable API only, zero gameplay plugins" (`SETTLEMENT_OVERHAUL_CONTRACT.md:29`
  in that repository, edition 2026-10-01). By mid-October it was ~57k lines of
  Rust over `solaris:plugin@0.7.0`
  (`<work>/solaris/.analysis/committee/q006-settlements-into-core.md:5`).
- 2026-10-16: owner directive, «Forget vanilla compatibility here. Merge
  settlements straight into the core. We become a core WITH SETTLEMENTS,
  better — but keep the ability to customize.» (`q006-settlements-into-core.md:3`).
- Committee Decision 006: native modules in `mc-core`, cutover at S1 with no
  hybrid, slices S1 registry and allocator → S2 households → S3 districts → S4
  economy → S5 military (`.analysis/committee/decision-005-006-007.md:16-21`).
  S1–S4 landed 2026-10-16/17 (`git show 1ada2ceb9:docs/decisions/0013-core-model-v2.md`
  lines 101-338).
- 2026-10-18: v0.0.8 with core v2 (core 1ada2ceb9); the plugins repository
  retired the package as "now owned by core", noting that the committed
  components "failed to instantiate on the v2 host" (`solaris-default-plugins`
  7fe8efa).

### 5.2 The reasons

1. **Owner product decision** (above). Not a technical failure.
2. **Two controllers of one body.** Plugin adoption set `no_ai = true` and the
   native brain then skipped those bodies, so housing and work ran in the guest
   while movement ran in the engine; the 50k-villager budget (Q005) needed one
   allocator on the tick owner sharing the brain's budget
   (`.analysis/committee/q006/sol.md:31,82`; `decision-005-006-007.md:14,17`).
3. **Split custody steps.** The guest persisted a fenced intent, then ran the
   native transfer, then finalized after the receipt
   (`solaris-default-plugins` 7fe8efa^ `solaris-settlements/README.md:66-68`);
   native apply makes it one WAL batch (`.analysis/committee/q006/astra.md:85`).
4. **Storage limits.** 4,096-byte values forced sharded records and two-bank
   publication (`q006/astra.md:75`, citing the package's `registry_storage.rs`).
5. **Missing housing API.** Family and district pipelines stopped for lack of
   an occupancy or conditional-home commit (`q006/sol.md:58`).
6. **Out-of-tick latency** (ADR 0013 Q8 at 1ada2ceb9, line 22): fine for
   rare decisions, not for per-resident work.

The v2 instantiation failure was the moment of deletion, not a reason.

### 5.3 What changed

- D66 reverses reason 1 for our gameplay, with the crate first (D66.3).
- Reasons 2, 5 and 6 do not recur by construction: this split keeps bodies,
  allocation and worker execution in engine kernels and moves only policy,
  records and presentation. The September package put housing, work policy
  and body control in the guest.
- Reason 3 is half closed: the API already has an atomic inventory plus
  storage transaction (`commands.wit:395-402`), revision guards on domain
  operations (`domain-operations.wit:21-25`) and durable operation receipts
  (`commands.wit:188`). Membership changes are not in that transaction yet.
- The D65 design gives the settlement codec to the domain and makes the core
  carry the payload opaquely (`DESIGN.md` §2.8, P2): the same envelope can later
  belong to a package.

### 5.4 What still blocks the move back

- Reason 4 is unchanged: values of 4,096 bytes (`limits.rs:66`).
- One serial host thread with 256/1,024 bounds and a halt on unrefusable
  backlog; the boundary cost is unmeasured (§3.1).
- API gaps: dialog clicks, payload channels, block use, war, army, steward,
  map, per-entity attachments, package-provided services (§2.1, §2.5).
- The exact-version monolith rebuilds every package on any change (§2.1).

Step 1 removes the last of these; steps 4 and 5 remove the rest before any
settlement record leaves the engine.

## Consequences and risks

- **Kernel creep:** kernels can grow hidden policy. Review rule: a kernel takes
  data and names no settlement, realm or war concept; a new kernel or engine
  point needs review, as the CoreUnload S1 ratchet does for inbound references
  (`DESIGN.md` §3.2).
- **One host thread** may saturate as packages grow; per-package threads are
  added only after a measurement shows it.
- **Persistence format:** moving settlement records into package storage
  changes their keys; it gets its own persistence review and either a
  converter or a world reset (owner question 4).
- **Churn:** step 1 rebuilds every package once; afterwards a change rebuilds
  only a service's importers.
- On acceptance, add the D66 row to [open transitions](../STATUS.md#open-transitions).

## Rejected alternatives

- **Guest code per entity (settlement brains or battle AI in Wasm):**
  `0013-core-model-v2.md:564-565`; at A3 scale it cannot fit the queue bounds
  (§3.2).
- **One monolithic API version:** today's exact match rebuilds every package
  for any change (§2.1).
- **Roblox-style permanent deprecation:** a compatibility surface of our own.
- **A per-hit `before-damage` hook for war legality:** a guest round trip per
  hit at battle volume; rules as data instead (§3.3).

## Open questions for the owner

1. "Small slowdown": proposed under 5% on MSPT p50 and p95, and TPS stays 20
   where it was 20; larger goes to you (§3.4).
2. The kernel line: per-entity work and fighting stay engine kernels driven by
   package data (recommended), or must they become Wasm code (which overturns
   `0013:564-565` and needs its own measurement)?
3. Realms as a separate package over a settlements package needs host routing
   between packages (§2.5). Keep D62's separate plugin, or ship realms inside
   the settlements package?
4. When settlement records move into package storage: converter, or a world
   reset?
5. Order: computer before realms (recommended, smaller), or realms first?

[Architecture overview](../ARCHITECTURE.md) · [Decisions index](README.md)
