# ADR 0024 - The core as an engine, our gameplay as packages

**Date:** 2026-11-19  
**Status:** Proposed (owner D66). Revised three times after the independent
reviews (committee-sol and committee-astra, "changes" in each round,
`receipts/EngineADR/REVIEW-{sol,astra}{,-r2,-r3}.md`);
see [Review resolution](#review-resolution).
Nothing here is implemented. The D65 settlements crate (ADR 0021 Step 2, design
`receipts/CoreUnload/DESIGN.md`, revised after its reviews `REVIEW-sol.md` and `REVIEW-sol-r2.md` in that directory,
both outside Git) is already running and is this ADR's first step; this ADR
builds on that design and does not repeat it.  
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
   scheduling, networking. It also keeps the fast **executors** our gameplay
   needs every tick (allocation, worker actions, construction cells, formation
   and combat steps, transit custody). An executor keeps the mechanism; the
   game rules inside it today (tactics, fatigue, lethality, casualty policy,
   eligibility, weights, rates) move to the owning package as data and
   decisions (§1.1).
2. Our own gameplay (settlements, armies and battles, realms, economy,
   computer) becomes WebAssembly packages. A package decides at settlement,
   regiment, treaty or command cadence and declares data the engine evaluates
   per tick. Its required records are native rows with a declared schema that
   recovery checks without calling the package (§1.3).
3. The plugin API becomes named, versioned **engine services** after Roblox's
   named services: one WIT package per service. The `[services]` table gates
   interfaces only; resource rights, subscriptions and phases are declared and
   checked separately (§2.5). The first cut matches versions exactly (§2.6).
4. A request is queued, then executed natively, then journaled, then answered
   (§2.2). Atomicity exists only inside one explicit native operation. The
   first cut keeps today's call count, order and refusal boundaries (§2.3).
5. No guest code runs per entity, per cell or per hit
   (`0013-core-model-v2.md:564-565`). Every move is measured A/B against an
   empty-engine control and with real plugin traffic, and its exact TPS/MSPT
   change is reported (§3.4).
6. Order: settlements crate (D65, running) → **service cut of the API (the
   first move)** → computer → realms (new) → economy → settlements onto WIT →
   armies and battles (§4).

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

The invariants that stay in the engine in every layout: item custody, saves
(native store plus tick WAL, `0013-core-model-v2.md:210-229`) and tick
authority (`0013-core-model-v2.md:504-512`).

## 1. Boundary

### 1.1 Mechanism in the engine, game rules in the package

Per-entity, per-cell and per-hit **execution** stays native: motion, paths,
reach, collision, damage application, custody-conserving item and block
changes, per-tick budgets, and the executor's own persisted images. This keeps
the original ADR 0013 Q7 ("Plugins compose mobs from native core building
blocks … as data. Core executes them natively each tick. Plugin code runs only
on rare decisions.", `git show 1ada2ceb9:docs/decisions/0013-core-model-v2.md`
lines 21-22), the current rejection of "guest AI on each entity tick or
synchronous in-tick plugin calls" (`0013-core-model-v2.md:564-565`) and ADR
0017's "no plugin scheduling API or second mutable authority"
(`0017-tick-system-graph.md:11-15`). ADR 0016 blueprints are the precedent: a
plugin declares a mob as data and the engine runs it.

The **game rules** inside these executors move with their domain into its
package: constants, shares, weights, thresholds, eligibility and the decisions
they drive. The package supplies them as data (a table the executor reads) or
as a decision at its own cadence (an order, a plan, a policy change). Moving
only rosters and orders is not enough. Each such move is measured A/B like any
other (§3.4): a value folded at compile time becomes a table value read at run
time.

| Executor | Mechanism that stays native | Game rules that move to the package |
|---|---|---|
| formation, steering, targeting, combat, archery, cavalry (`battle/`, 13,025 prod lines) | steering cells, target grid, reach, damage and removal application, the battle image | tactics and fatigue: decision cadence, mesh interval and depth, surge probability and length, fatigue and recovery, exhaustion, morale floor (`battle/formation.rs:11-44`); cinematic lethality and casualty policy: target removals, press exposure, fatal share, pursuit lethality (`battle/combat.rs:40-80`) |
| bed, job and meeting-point allocation (`settlements/allocator.rs`, 2,280 lines; budget `tick_budget.rs:8-9`) | exclusive claims, POI index, path budget | which beds and job slots count (certified houses and certified work points only, `allocator.rs:1243,1298-1306`), staffing priority |
| worker actions (`villager/brain.rs`, 1,577, and the villager AI submodules; generic work orders `npcs.wit:190-271`) | digging, felling, harvesting, placing and hauling with conserved loot, doors, reach | job vocabulary and eligibility (for example which trees or zones a worker may cut), schedules |
| construction cells (generic executor in `plugin_structures`, 1,732 + 3,329 lines; settlement policy in `plugin_structures/settlement_builds.rs` and `settlements/construction*.rs`, 9,592 lines) | per-cell placement paid from storage, repair-stage derivation, the structure ledger | bills, project and order caps (`construction.rs:58,60`), builder stall and shun times (`construction.rs:2144,2146`), borrowing rules; the code split is D65 S7 (`DESIGN.md` §2.5: the reserve check stays on the generic path for every owner) |
| transit custody (`settlements/transit.rs`, 1,097) | departure and arrival custody, transit records | migration weights, thresholds and arrival chances (`migration.rs:15-33`) |
| economy accrual (`settlements/production.rs`, `commerce.rs`, `caravan.rs`) | game-time watermarks, atomic transfers, retry timing | food points per day (`offscreen_production.rs:6`), wages and periods, caravan deadlines, recipes |
| zones and faction relations (`plugin_player::protects_cell`, `battle::faction`) | per-edit and per-hit evaluation | who is allowed and when (war windows) |

**Rules bound to persisted executor state.** Some rules shape what an executor
persists. A battle section saves its decision phase as the remainder of the
decision period (`battle/formation.rs:205-211,419-427`) and fires when
`tick % DECISION_TICKS` equals it (`formation.rs:748-760`); if a package changed
the period from 10 to 5 across a reopen, a section saved with phase 7 would
never fire again. So an active battle is bound to one immutable, validated
parameter set: the set is admitted once (non-zero periods, permissible
denominators, bounded shares), its derived coefficients (for example the
removal rolls of `combat.rs:58-80`) are computed once at admission and never
per hit, it is saved with the battle image (`battle/persistence.rs:134-142`) and
restored on reopen. A deployed package whose set differs from an active
battle's refuses before the first tick. New battles take the package's current
set. The gate is A11's SIGKILL continuation (`docs/DEFINITION_OF_DONE.md:37`):
the battle continues with the same rules after the kill, and reopening with
changed rules is refused. The same rule applies to any executor whose saved
state depends on a package parameter.

Owner D66 asked for a small slowdown at most, not a fixed split per constant:
where a rule's table read costs more than the §3.4 limit on a hot path, the
move reports it and goes to the owner rather than keeping the rule native
silently.

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
  (binding, membership, work orders, paged NPC reads), the allocation executor,
  Inventory (storage endpoints, carry, equipment), Scheduler (game time, daily
  phases), Storage, Presentation (dialogs, menus, map items, labels),
  Commands, Content (blueprints, recipes, items).
- State today: `settlement/<id>` and `settlement_sequence` checkpoint rows plus
  `JournalEntry::Settlement` WAL images, `transit/<uuid>` rows (`DESIGN.md`
  §1.3, [settlements architecture](../architecture/settlements.md) "Durability
  and plugin boundary"); the structure ledger as plugin-domain KV keys
  (`0013-core-model-v2.md:326-331`); membership and work cursors in the
  villager entity column (`DESIGN.md` §1.2). Target (§1.3): records become
  package-owned native rows with a declared schema, only after D65 C3 and in a
  separate D66 format cutover; membership and cursors stay in the entity image;
  the structure ledger stays with the generic executor.
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
- Measured: `villager-heavy` (20,000 villagers + 100,000 mobs in pens, 100
  bots) MSPT p50 140.1 / p95 157.0 ms, 7.12 TPS, laps not instrumented
  (`docs/performance/PROFILES.md:23,83`); `native-100` without villagers
  `commit.settlements` p50 0.03 ms (`receipts/PerfAfterRename/phases-c.md:109`).

**Armies and battles.**
- Today: `crates/mc-core/src/battle/` 13,025 prod / 12,742 test lines
  (39 files); `settlements/army.rs` 430; `villager/soldier.rs` 273.
- Engine services needed: Entities, the formation and combat executors, World
  (ground), Presentation (spectator bindings, views), Commands, Content
  (weapons as items, `lifecycle.wit:69-83`; mob blueprints).
- State: the battle owner image (phase, battle clock, regiment and member
  identity, orders, morale and weapon timers) in the entities' checkpoint and
  WAL frame, full image first after a checkpoint and patches after
  (`0013-core-model-v2.md:231-247`); `Soldier` in the villager column
  (`DESIGN.md` §1.2). Target: the battle image stays executor state in the
  engine; rosters, orders, war policy and the tactics, fatigue, lethality and
  casualty rules of §1.1 become package data and decisions, and each active
  battle keeps the validated parameter set it started with (§1.1).
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
  lines 33-69; fit with D65, lines 73-77).
- Engine services needed: Settlements (query, steward, war declaration and
  outcome, muster), Players, Storage, Inventory (tribute is a real transfer),
  World zones with a war-window exception as data, Presentation (map item,
  dialogs), Commands, Scheduler.
- State (as in `DESIGN.md` §2.7): the treaty terms (vassalage, tribute amount
  and due dates), ransoms and the chronicle are owned by the realms package as
  bookkeeping (step 3), never `SettlementRecord` fields; the settlement records
  hold the native war state, the steward and the peace or surrender outcome with
  a reference to the treaty; from step 4 the material obligation (tribute owed)
  is a native row with one owner that changes only in the same typed operation
  as the payment that settles it (§4).
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
- Engine services needed: Inventory, Entities (work orders and caravan
  movement), Scheduler, Storage, Presentation (menus), Commands, and typed
  native economy operations with durable receipts (§1.3).
- State: economy policy in each settlement record's post-image, goods in native
  containers and villager carry (Decision 006 S4, `git show
  1ada2ceb9:docs/decisions/0013-core-model-v2.md` lines 101-158); money in plugin
  KV ledgers. Target: rules and ledgers in package records; goods stay engine
  custody; every payment, wage or tribute that moves goods is one typed native
  operation.
- Hot paths: the production and payroll cursors above, daily consumption once
  per 24,000 game ticks per settlement; production portions and caravan
  walking are executors (craft receipts `npcs.wit:591`, orders `npcs.wit:374`).

**Computer.**
- Today: `crates/mc-edge/src/computer.rs` 459, `screen.rs` 1,091,
  `screen_service.rs` 248 lines; the `computer` package is a client content
  bundle only (`computer/plugin.toml`, 36 lines of guest source).
- Engine services needed: World (block-use events), Presentation (UI flow),
  Players. Authorization of every open and input (operator level 4, local-only,
  online auth), revocation, the process bridge and the screen frame stream stay
  in `mc-edge` (`0018-in-game-computer.md:14-20,29-48,118-141`).
- State: none persisted (`0018-in-game-computer.md:47-48`).
- Hot paths: none in the tick.

### 1.3 Records, commit and recovery

**Custody** (items, bodies, blocks) and its revisions belong to the engine in
every layout.

**Commit path.** A package request is queued, then applied by its native
owner, then written in one tick WAL frame with the post-images it touched,
then, after that frame's append acknowledgement, answered to the package
(`core/tick_loop.rs:86-89,293-332`, `core/journal.rs:763-781`). The
acknowledgement is not an fsync of the journal directory
(`journal.rs:727-745`; `0013-core-model-v2.md:215-216`), and ordinary world
publication may follow as soon as the frame is queued; only the package's
answer waits for the acknowledgement.

**Atomicity lives in one explicit native operation.** Sharing a WAL frame
does not make two requests one transaction, and a request id does not make a
purchase idempotent. Two contracts exist today:

- The **inventory-storage transaction** is atomic over one live session's
  inventory and the package's own KV, and has no durable operation id
  (`crates/mc-core/src/plugin_player.rs:257-272`). Its contract forbids a blind
  retry: "A missing answer does not establish refusal and must not trigger a
  blind retry" (`commands.wit:392-394`). Its gate is that inventory and KV are
  consistent after reopen, and that a resubmission with the KV version the first
  attempt already spent debits nothing a second time. That control runs after
  reopen from a new live session of the same player with the old KV version, so
  the absence of a second debit cannot come from a dead session. An API-only cut
  adds no receipt to it.
- A **domain operation** with an operation id has a durable receipt. After a
  lost answer the package resubmits the same id and gets the stored outcome
  (`plugin.rs:836-847`); a different payload under that id is
  `operation-conflict`. `operation-status` reads these receipts only
  (`plugin.rs:823-835`). The storage version guard is checked at apply
  (`plugin.rs:848-857`) and is part of that resubmission comparison; it is not a
  WAL replay check and writes nothing.

Economy and settlement actions (wages, purchases, tribute payment, founding,
hiring) each become a **typed native operation of the second kind**: one apply
validates and commits the records it touches, the custody it moves, the
membership it changes and its durable receipt. Its gate covers refusal, a lost
answer, reopen and resubmission through the real host path, not by calling
`execute` directly as `plugin_player_tests.rs:92-136` does today. Never a
custody transfer followed by a separate KV write. The checkpoint-only class of
owner D45 (`0013-core-model-v2.md:553-555`) is for new independent state only,
never for obligations that affect custody or resubmission.

**Storage.** The string KV is not widened: it is folded into one `plugins`
checkpoint row (`core/journal.rs:1271-1279`) and its values stay at 4,096 bytes
(`crates/mc-script/src/limits.rs:66`). Required package records (records an
engine operation reads to admit or execute: custody, obligations, permissions,
membership) are separate native rows with touched-only WAL images, like
`settlement/<id>` today. Package-local bookkeeping that no engine operation
reads may stay in the bounded KV. The
16 MiB envelope (`crates/mc-world/src/native/checkpoint.rs:30`) is a storage
ceiling, not a record size and not an ABI budget: each record kind's bound is
derived from real records and checked before any custody changes. WIT carries
call-owned values, pages, domain changes, opaque ids and revisions, never native
rows, slots, pointers or `TickState`. Membership and work cursors stay in the
entity image; transit and reservations stay in native custody.

**Schema owner and fail closed.** Every required record kind has, separately
from the WIT service version, an owner (the package id), a schema version and a
validator that is declared data in the world contract (a typed record layout
with bounds), so live recovery, offline tools and WAL verification check each
image without calling the package. A missing owner or codec, an unknown schema
version or a corrupt image refuses the world before serving and before the
image is folded, so a later replacement or tombstone cannot hide it; this is
the D65 C1 rule (per-image schema decode in the shared batch decoder before
routing and fold, `receipts/CoreUnload/DESIGN.md:39-43` and §2.8, Main rulings
Q2 and Q7 there). Referential and custody
checks stay at their existing recovery stages. Today's KV is validated only as
bounded strings (`crates/mc-script/src/validation.rs:143-171`) and the startup
world identity does not record package versions
(`crates/mc-core/src/plugin_startup.rs:190-195`); neither is schema validation.

**D65 alignment.** No `SettlementRecord` field leaves the native settlement
record (to a package, KV or another owner) before D65 C3 is accepted, and C3 is
accepted only through the D65 gates listed in §4 step 0. A record format change
happens only in a separate D66 cutover with one writer, never inside a D65
slice and never dual-written (`receipts/CoreUnload/DESIGN.md:23-27`). The
converter-or-reset choice is owner question 2.

## 2. API: named, versioned engine services

### 2.1 Today: `krasnokamen:plugin@0.9.0`

- 18 WIT files, 3,835 lines in `crates/mc-script/wit/`. The world imports
  `host` and `command-permissions` and exports `lifecycle`, `events`,
  `precommit` (`plugin.wit:11-19`).
- Effects are return values: `on-events` returns `list<command>`
  (`events.wit:640`); `command` is one variant of 21 cases
  (`commands.wit:526-574`), `event` one variant of 30 (`events.wit:586-635`);
  domain families go through one `operation` envelope that also carries the
  storage version guards (`domain-operations.wit:10-25`). Imports are
  deliberately tiny: "no import here may take a world lock, wait for a chunk,
  or block on I/O" (`host.wit:1-6`).
- One version for everything, matched exactly
  (`crates/mc-script/src/version.rs:41-48`, checked at `manifest.rs:872`): any
  WIT change rebuilds every package ([plugin contract](../plugins/CONTRACT.md),
  "The contract is"). Capabilities are 19 strings plus `custom_payload:<name>`
  channels (`manifest.rs:301-323`).
- Production use: the first-party packages send messages, use storage (get,
  CAS, batch CAS, operation status), zones, teleport, timers, inventory menus
  and inventory-storage transactions, and read player, command, zone, timer and
  world observation events. No production package uses `npcs`, `npc-routes`,
  `settlements`, `world-structures`, inventory operations or
  `client-presentation`; the host fixture `sdk/rust/examples/hello` exercises
  them. `spawn-entity` is used by the supported example
  `sdk/rust/examples/sky-creeper/src/lib.rs:36-45`. `before-damage` exists in
  WIT (`precommit.wit:146`) but the runtime admits only build hooks
  (`crates/mc-core/src/plugin.rs:636`).
- Gaps our gameplay needs: dialog clicks ([dialogs](../DIALOGS.md), line 88),
  block-use callbacks (`0018-in-game-computer.md:14-17`), war, army, steward
  and map points (`D62-REALMS-PLAN.md:33-69`).

### 2.2 Request path: admission, execution, result

Today's path, which the first cut keeps phase for phase:

1. **Enqueue.** In the new shape a request import returns only an
   enqueue-local correlation id. The host keeps one bounded buffer per
   callback: at most 32 requests, timer changes included (`limits.rs:29`; the
   shared count in `crates/mc-plugin-host/src/host/clock.rs:105-115`), and a
   byte budget counted as each request is enqueued, before staging. An import
   past either bound fails the callback.
2. **Admission at return.** The host validates every DTO and capability and
   reserves channel space for the whole batch at once
   (`crates/mc-script/src/route_authority.rs:1323-1383`, `try_reserve_many`).
   A trap, an exhausted budget, a `plugin-error`, a failed late import or a
   refused batch drops the whole request buffer and the timers staged in that
   callback (`delivery.rs:28-35,66-84`). The guarantee covers exactly those
   two. Synchronous host-local imports (§2.4) take effect when called and are
   not undone: a permission publication replaces the grant table at once
   (`crates/mc-script/src/command_permissions.rs:145-152`).
3. **Native execution.** `plugin::pump` runs before the dimension tick
   (`crates/mc-core/src/tick.rs:753`), takes the queued requests one at a time
   (`plugin.rs:1270-1291`) and re-validates live sessions, owners, revisions
   and reservations at apply. Plugin damage enters at the merge
   (`tick/dimension.rs:273-276`). Two imports from one handler are two
   operations; only one explicit composite operation is atomic (§1.3).
4. **Results as the owner reports them.** The enqueue-local id is the
   package's correlation id, not a promise of an answer. Owners answer as they
   do today: a storage or inventory-storage answer and a domain-operation
   answer carry the request id (a domain operation also its durable operation
   id, `host/conversion.rs:192-202`); a zone answer names the zone, not a
   request, so two commands on one zone in flight cannot be told apart
   (`commands.wit:207-212`, `conversion.rs:203-211`); menu open and close are
   fire-and-forget (`commands.wit:552-559`). A result of a committed change is
   delivered after its frame's append acknowledgement (§1.3), to the package
   that asked, never broadcast. No import gets a new callback of its own.

### 2.3 Delivery order

- **FIFO per lane.** There is no global order; the contract says so
  (`events.wit:20-23`): `sequence` is a type, and `first-sequence` is 0 today
  (`crates/mc-plugin-host/src/host/conversion.rs:290-296`, `clock.rs:79-83`).
  The host endpoint reads independent lanes (observations, the coalesced
  server tick that drives timers, ordinary events and answers) and interleaves
  them by its existing arbitration (`crates/mc-script/src/route_authority.rs:1197-1232`).
  Order holds within a lane, not across lanes; the first cut keeps that
  arbitration unchanged.
- The first cut keeps the number and order of callbacks: today one server
  event becomes one call per interested package (`host/lifecycle.rs:458-474`),
  and the new per-service handler receives exactly that call. Targeted answers
  go only to the requesting package. All due timers of one pushed tick keep
  their staging and shared budget (at most 8 timer callbacks per tick and 256
  pending timers per instance, `crates/mc-plugin-host/src/timers.rs:20,27`).
- **Two kinds of transient refusal.** For an ordinary delivery, a transient
  refusal (backpressure, a full admission ledger, a player who left) drops the
  batch and sends the package a `command-batch-rejected` notice
  (`delivery.rs:92-102`); the notice gives no right to send a new request and
  reaches a package that imports no `commands` interface too. For a timer
  delivery, the same refusal drops the request batch and the staged timer
  schedule of that pushed tick, keeps the previous schedule and delivers the
  due timers again later; no rejection notice is sent (`clock.rs:150-166`).
  Synchronous host-local imports the timer callback already made (a permission
  publication, for example) stay applied, as in §2.2.
- Nothing regroups A/B/A into A/A/B or merges independent batches. If
  aggregation is added later, it merges only adjacent events of one service in
  one lane, phase and tick, with seal order and batch boundaries set natively,
  independent of when the guest finishes.
- **Determinism** holds for identical per-tick input: the same requests
  admitted at the same ticks give the same native result and the same replay.
  The same request list admitted at different ticks (because host delay
  differs) may give a different result.
- Package deadlines use game time from a persisted watermark (ADR 0023,
  `0023-offscreen-settlement-simulation.md:54-61`), not the host's pushed tick:
  that clock follows the WAL tick (`crates/mc-core/src/plugin/events.rs:122-130`),
  which advances under `/tick freeze` (`tick/dimension.rs:514`).

### 2.4 Services

- **Service = one WIT package** `krasnokamen:<service>@<version>` with one
  interface per capability. Request functions are enqueue-only imports (§2.2);
  each service exports one handler interface for its events.
- **Synchronous host-local imports** (Argon2 `password-hash`/`password-verify`,
  `host.wit`; permission publication, `command-permissions.wit:8-21`; `log`;
  `plugin-id`) are not requests: they never touch the world and are not queued.
  Their effect is immediate and outside the batch-drop guarantee of §2.2.
  Which phase may call which is listed in §2.5.
- **Runtime service** `krasnokamen:runtime`: lifecycle, scheduler (game-tick
  timers, `commands.wit:340-369`), log and identity, the shared failure enum
  (`operation-types.wit`), the batch-rejected notice. There is no per-tick
  callback: Roblox's `RunService.Heartbeat` runs scripts every frame, which
  `0013:564-565` rejects.

| Service | Interfaces | From today's WIT | Roblox analogue |
|---|---|---|---|
| `runtime` | lifecycle, scheduler, log, handler | `lifecycle.wit` (minus the contribution), timers in `commands.wit`, `host.wit` log/id, `types.wit`, `operation-types.wit` | `RunService` (without per-frame scripts) |
| `content` | items, recipes, block tools, worldgen rules, mob blueprints, record schemas | `lifecycle.wit:13-110` startup contribution | packages and assets |
| `players` | roster, messaging, teleport, session, auth | send-message, broadcast, list-online-players, teleport, disconnect, unlock-session; `host.wit` password functions; join/leave/chat/death events | `Players`, `TextChatService` |
| `commands` | invocations, permissions | `command-permissions.wit`, command-invoked | chat commands |
| `storage` | kv, operation status | `storage.wit`, storage commands; record rows come with step 4 (§1.3) | `DataStoreService`, but WAL-durable and fenced with custody |
| `world` | blocks, zones, structures, observations, build rule, time | `world-structures.wit`, `world-events.wit`, zone commands, `precommit.before-build` | `Workspace` |
| `entities` | npcs, work, orders, routes, spawn | `npcs.wit`, `npc-routes.wit`, spawn-entity | `Humanoid`, `PathfindingService` |
| `inventory` | endpoints, transfers, reservations, transactions, menus | `inventories.wit`, inventory-storage transaction, inventory menus | — |
| `presentation` | Loader views, dialogs, sounds, map items, payload channels | `client-presentation.wit`, plus the gaps above | `ScreenGui`, `RemoteEvent` |
| `settlements` | query, actions, handler | `settlements.wit` | — (game-specific) |

We borrow Roblox's named services, not its dynamic `GetService`, per-frame
`Heartbeat`, cloud save semantics or permanent compatibility. References:
<https://create.roblox.com/docs/reference/engine/classes/DataModel>,
<https://create.roblox.com/docs/scripting/services>,
<https://create.roblox.com/docs/scripting/capabilities>.

### 2.5 Manifest, rights and the component world

`[services]` names the interfaces a package may link, nothing more. Everything
else stays a separate declaration with its own check, as in the current
contract ([plugin contract](../plugins/CONTRACT.md), "A minimal server-only
component", "Custom native mobs" and the permission sections):

| Declaration | Grants | Checked at |
|---|---|---|
| `[services]` | which interfaces are linked and which handlers the host may call | startup: the component's actual imports and exports equal the declared interfaces and versions; an undeclared import or export refuses |
| `events` | which events are delivered | startup: the event name exists (`manifest.rs:878-893`) and the service's handler is exported |
| `player_commands`, `command_permissions` | command roots and their nodes | startup: the `commands` handler is exported; admission: the grant table on the tick thread |
| `permission_provider` | the one live permission publisher | startup: at most one package; other importers are answered `false` (`command-permissions.wit:8-21`) |
| `custom_payload:<name>` | one Play channel | startup: one owner per channel (`manifest.rs:319-321`) |
| `spawn_entity_types` | exact entity types the package may spawn | admission, per request (`manifest.rs:407-408`) |
| `hooks` | precommit registration | startup; a hook answers a decision and nothing else (`precommit.wit:1-25`) |
| `death_graves` | a deployment surface that changes native death custody for every player (`docs/plugins/OPERATIONS.md:150-159`); kept by the audit for `graves` (`receipts/PluginDupAudit/AUDIT.md:45`) | startup: declared in the manifest, at most one declaring package (a second refuses the deployment before start, `crates/mc-core/src/plugin.rs:593-612`); deploying the package is the operator's authorization, never implied by a service |
| `[client]` bundles and their permissions | Loader content and rights | startup sha256 and Loader negotiation (ADR 0010) |
| `[worldgen]`, `required_features` | authored data the core must load | startup, as today |

**Phases.** One component is instantiated in two stores with one linker, as
today (`crates/mc-plugin-host/src/host/lifecycle.rs:193-214`). Wasmtime
resolves all of a component's imports before the first call (Wasmtime 36,
`runtime/component/linker.rs:197-213`), so the startup store links the same
declared functions; what each phase may do is enforced at call time, as
`allow_import` does today (`crates/mc-plugin-host/src/lib.rs:313-320`,
`instance.rs:574-640`). Enqueue imports are new, so their phase rule is new;
host-local calls keep today's rule (only hooks refuse them). The first cut
does not forbid the KDF in the startup store: that would be a tightening no
package needs.

| Phase | Enqueue imports | Host-local calls allowed (as today) |
|---|---|---|
| startup store: component initialization and `configure` (returns only a contribution, `lifecycle.wit:123-128`) | refused: the call fails the phase | `log`, `plugin-id`, `password-hash`/`password-verify`; `publish` answers `false` (the startup instance is handed no permission table, `host/lifecycle.rs:193-218`) |
| runtime store: component initialization, before `init` | refused: no callback batch exists yet | `log`, `plugin-id`, `password-hash`/`password-verify`; `publish` answers `false` (the table is handed over after instantiation, `host/lifecycle.rs:212-218`) |
| runtime store: `init` and ordinary handlers | allowed (§2.2) | `log`, `plugin-id`, `password-hash`/`password-verify` (auth packages), `publish` (the provider only) |
| hooks (`precommit`) | refused | `plugin-id` only; a hook answers its decision (`precommit.wit:12-17`) |
| `shutdown` (best effort, returns no commands, `lifecycle.wit:135-138`) | refused | `log`, `plugin-id`, `password-hash`/`password-verify`; an old generation cannot publish to the live table ([plugin contract](../plugins/CONTRACT.md), lines 229-230) |

Example: today's `krasnokamen-towns` (capabilities `storage`, `zones`,
`player_queries`, root `town`, timers) after the cut. Its current
`events = ["server.started"]` is dropped: no contract event carries it
(`events.wit:586-635`), and towns loads its state from `init`
(`solaris-default-plugins` `sources/krasnokamen-towns/src/lib.rs:986-1001`).

```toml
id = "krasnokamen-towns"
name = "Krasnokamen Towns"
version = "0.2.0"
entry = "plugin.wasm"
player_commands = ["town"]

[services.runtime]
version = "0.1.0"
interfaces = ["lifecycle", "scheduler", "log", "handler"]

[services.players]
version = "0.1.0"
interfaces = ["messaging", "roster", "handler"]

[services.commands]
version = "0.1.0"
interfaces = ["invocations"]

[services.storage]
version = "0.1.0"
interfaces = ["kv", "handler"]

[services.world]
version = "0.1.0"
interfaces = ["zones", "handler"]
```

```wit
package krasnokamen-towns:guest@0.2.0;

world towns {
    import krasnokamen:runtime/log@0.1.0;
    import krasnokamen:runtime/scheduler@0.1.0;
    import krasnokamen:players/messaging@0.1.0;
    import krasnokamen:players/roster@0.1.0;
    import krasnokamen:storage/kv@0.1.0;
    import krasnokamen:world/zones@0.1.0;

    export krasnokamen:runtime/lifecycle@0.1.0;
    // timer-fired, command-batch-rejected
    export krasnokamen:runtime/handler@0.1.0;
    // command-invoked for /town
    export krasnokamen:commands/invocations@0.1.0;
    // online-players answers
    export krasnokamen:players/handler@0.1.0;
    // storage get and CAS answers
    export krasnokamen:storage/handler@0.1.0;
    // zone command answers
    export krasnokamen:world/handler@0.1.0;
}
```

### 2.6 Versions and breaking changes

Three versions are distinct: the one the manifest declares, the one the
component's imports and exports actually carry, and the one the host provides.

- **First cut:** all three must match exactly for every service; anything else
  refuses before the world opens. Wasmtime 36 (`Cargo.lock:4055-4056`) can
  resolve a versioned import to a semver-compatible host name and then
  type-checks it exactly
  (<https://docs.rs/wasmtime/latest/wasmtime/component/struct.Linker.html>),
  but that lookup proves neither type equality nor an unchanged import after an
  interface grows, so the first cut does not use it.
- **Before 1.0** every change to a service is a minor bump and incompatible;
  an incompatible 0.x minor is refused, not only a different major.
- **From 1.0:** a new optional import interface is additive and must never be
  required by an older binary; any change to an existing function, record or
  variant (including a new event case) and any new required export (a new
  handler the host calls) is breaking. Semver lookup may then be enabled for
  imports only, with a test that an unchanged, unrelated component still loads.
- **One bundle per breaking change:** WIT, host, SDK, every affected package
  (importers, exporters and transitive users of shared types) and the live pack
  and matrix pins, in one change set; release notes name the service and both
  versions. The host serves one version of each service: no adapters, no second
  version, no deprecation window (AGENTS.md: no compatibility surface of our
  own). Here we depart from Roblox, which keeps deprecated APIs callable.
- **Acceptance of a version change.** The first cut (monolith to services) has
  no unaffected old binary: every kept guest is rebuilt against the split API,
  and the old 0.9.0 monolith component is the negative control, refused before
  the world opens. From the second change of the split API on, the positive
  control is an unchanged service component that loads while another service
  changes; an incompatible component is refused before the world opens; there
  is never a second host version.
- An API-only cut changes no serialized DTO: operation receipts persist the
  request they answered (`plugin.rs:942-955`), and their bytes stay identical.

### 2.7 What the cut deletes

- `package krasnokamen:plugin@0.9.0` and its world (`plugin.wit:11-19`), the
  manifest `api` key and the single-version check (`version.rs:41-48`), replaced
  by per-service exact checks (§2.6).
- The `command` and `event` variants and `on-events`
  (`commands.wit:526-574`, `events.wit:586-640`), replaced by per-service
  request imports and handler exports with the same call count and order
  (§2.3).
- The WIT `domain-operations.wit` envelope (lines 10-25). Its storage version
  guards stay on the native request itself: they are checked at apply and are
  part of the resubmission comparison of an operation id (`plugin.rs:836-857`);
  a separate storage CAS would not replace them.
- The 19 capability strings (`manifest.rs:301-319`); `custom_payload:<name>`
  becomes its own declaration (§2.5).
- Dead contract: `precommit.before-damage` (refused at runtime,
  `plugin.rs:636`) and `hook-actor.environment`, which no path produces
  (`D62-REALMS-PLAN.md:55-61`). Damage legality for realms and war comes back
  as engine data rules (§3.3), not a per-hit hook.
- Families without a production consumer (§2.1) stay where a move in §4 or a
  supported SDK example uses them (`spawn-entity`: sky-creeper); the cut lists
  each with its consumer and deletes the rest.

### 2.8 Services provided by a package

Built only at step 5 (§4), when settlements leave the engine; until then the
engine serves `settlements` (`crates/mc-core/src/plugin_settlements.rs`) and the
realms package uses that. At step 5 the settlements package becomes the single
provider of the `settlements` service:

Two parties are distinct on every routed request: the **originating consumer**
(its package id and the actor it acts for) and the **provider**, which owns
the records the request touches. Both identities come from host provenance,
never from fields a guest returns.

- **Attestation.** Today the host attaches provenance (plugin id and route
  generation) to every command it admits from a callback
  (`crates/mc-script/src/route_authority.rs:1368-1382`), and the server accepts
  only host-attached commands, which it turns into a non-cloneable admitted
  command carrying that plugin id (`route_authority.rs:1042-1059`,
  `command_admission.rs:247-257`). Routing extends that attestation: when the
  host delivers a consumer's request to the provider it records the routed
  context (consumer package and generation, the actor taken from the consumer's
  own attested input, the correlation id) under a host-issued route handle.
  The provider's typed native operation names only that handle; the host
  attaches consumer, actor and the registered provider from its own record at
  admission. A handle the host did not issue, or one that names another
  provider, is refused. Handles are runtime-only: after reopen the consumer
  resubmits with the same operation id, the host routes it again with a new
  handle carrying the same parties, and the receipt below matches on the
  parties, not on the handle.
- The route is asynchronous: the host delivers the request to the provider as
  an event; the provider's answer comes back after the provider decided,
  bounded in size like any answer.
- **Authorization.** The native owner checks the actor's current rights for the
  action and the registered provider's right to change exactly its own records.
  No identity replaces the other, and none of the provider's rights pass to the
  consumer.
- **Receipt.** The durable receipt is bound to the provider, the consumer
  package, the actor's stable identity (the player UUID, not the session) and
  the operation id, so a resubmission after reopen matches only the same
  parties and payload and returns the stored outcome (§1.3). A new mutation
  is never authorized by an old receipt: it needs the actor's current rights.
- A message between packages is not a WAL transaction. Success that involves
  custody exists only after that operation committed and its frame was
  acknowledged; the provider's message alone commits nothing.

## 3. Performance guard

### 3.1 Cost model of the boundary

For one move, per tick:

- **Tick thread (adds to MSPT):** building the events the package subscribed
  to, converting and applying its admitted requests in the pump, and native
  evaluation of the data rules it declared. These are O(events + requests +
  rule evaluations), never a guest call per entity.
- **Host thread (off the tick, competes for cores):** callbacks × fixed call
  cost (fuel refill and epoch re-arm,
  `crates/mc-plugin-host/src/instance.rs:12-14,123-132`; canonical-ABI entry
  and exit) + imports × enqueue cost + bytes lifted and lowered × per-byte
  cost + guest work × the Wasm-to-native factor. One thread serves every
  package, one callback at a time (`crates/mc-plugin-host/src/host.rs:1-10`,
  [plugins](../PLUGINS.md) "Slow callbacks"). On the reference box the server
  gets four physical cores (`PROFILES.md:42-44`), shared with the parallel
  entity phase, so host CPU is not free.
- **Native fan-out:** group members touched per request and native checks per
  hit or cell driven by package data. These, not the request count, measure
  the work.
- **Latency:** at least one tick from event to effect; a request can be
  refused as stale ([plugins](../PLUGINS.md) lines 12-13).
- **Hard bounds:** 32 requests per callback, 256 queued commands, 1,024 queued
  events (`crates/mc-script/src/limits.rs:29,31,33`); 8 MiB per host call,
  50 M fuel per call, a 30 s wall guard
  (`crates/mc-plugin-host/src/limits.rs:80-100`); 256 pending timers per
  instance and 8 timer callbacks per pushed tick (`timers.rs:20,27`). 256
  queued commands is an outstanding limit, not a per-tick throughput. Past a
  quarter of the event bound the core refuses new plugin commands, and
  unrefusable events past the bound halt the world ([plugins](../PLUGINS.md)
  "Slow callbacks").
- **Unmeasured:** the fixed call cost, the per-import and per-byte costs and
  the Wasm factor. The host's limits are "deliberately conservative starting
  points, not measured budgets" (`crates/mc-plugin-host/src/limits.rs:9-12`);
  no boundary benchmark exists. The first move measures them (§4.1).

### 3.2 Against A3: peaks, not means

A3 is 100+ players and 100k+ freely roaming mobs at 20 TPS
(`docs/DEFINITION_OF_DONE.md:29`): 50 ms per tick. Today `native-100` (mobs in
roofed pens, `PROFILES.md:82`) runs at p50 63.45 ms, 15.17 TPS and
`villager-heavy` at p50 140.1 ms, 7.12 TPS (`PROFILES.md:22-23`). Those are pen
profiles, not A3; A3 claims come only from the free-roam `horizon-100-roam`
scenario the definition of done names (`DEFINITION_OF_DONE.md:27,29`). The tick
is already over budget, so a move may add no per-entity work to the tick
thread, and its host-thread work must fit in spare core time at the **peak**,
not on average.

- Per-entity guest decisions cannot fit: 20,000 soldiers make about 3,000
  target and attack decisions per tick (§1.2, [INFERENCE]).
- A general order to 20,000 fighters needs at least 313 requests at 64 members
  each (`npcs.wit:473-483`), more than the 256-command outstanding capacity
  (`limits.rs:33`, a bound on commands waiting at once, not on commands per
  tick). A regiment order therefore names the regiment id and revision and the
  engine resolves membership natively, or it goes in bounded portions whose
  added delay is measured.
- 1,000 settlements cannot each hold a package timer (256 per instance, 8
  callbacks per tick). One package timer drives a native due queue that hands
  the package a bounded page of due settlements per call. A once-per-day
  evaluation of 1,000 settlements averages 0.04 per tick, but its peak is
  whatever the due queue releases at once; the page bound sets it.

### 3.3 Rules for gameplay packages

1. No guest call per entity, per cell or per hit; executors stay native.
2. **Rules as data.** Tactics, fatigue, lethality and casualty parameters,
   protection zones with war-window exceptions, faction relations, recipes,
   blueprints, migration weights and wage parameters are tables a package
   pushes; the engine evaluates them per hit, edit or decision (§1.1).
   Precedents: zone allow-lists enforced natively for everyone
   (`solaris-default-plugins` `krasnokamen-towns/README.md:5`), ADR 0016
   blueprints, the zone gate in `D62-REALMS-PLAN.md:63-69`.
3. **Group requests by handle.** A regiment, household or settlement is named
   by id and revision; per-member lists are bounded portions with per-member
   outcomes (`npcs.wit:665`).
4. **Delivery as today** in the first cut (§2.3); later aggregation only under
   the rules there.
5. **Due-driven, paged.** Policy runs on scheduler timers and native due
   queues at its own cadence, never by polling every tick.
6. **Filtered and aggregated observations.** The engine builds only events a
   package subscribed to, sheds observations per package as now
   ([plugins](../PLUGINS.md) "Slow callbacks"), and reports aggregates (for
   example casualties per regiment per interval) instead of per-hit events.
7. **Paged snapshot reads** (`settlement-page`, `npc-page`) instead of
   per-entity queries.

### 3.4 A/B rule for every move

- **Pins and window:** B (before) and A (after) from
  `tools/dev/pin-release --no-incremental`, frozen trees named in the receipt,
  the same box, a quiet window (`tools/dev/mem-run.sh -x -p` inside
  `tools/dev/measurement-windows`).
- **Overhead control, a separate and stricter gate:** `native-100` (pens, no
  plugins) and `explore-20` without plugins, B against A. With no plugin
  traffic the move may not make the engine slower: a neutral result or a
  confirmed speedup passes; a reproducible worsening beyond the noise, of any
  size, goes to the owner with its cause; an inconclusive result does not
  allow the merge (rule 3 below).
- **Boundary under load:** a paired B/A with the same post-audit package set,
  the same world and the same scripted command stream, which must show non-zero
  callbacks, requests and completed effects in both runs; a run with zero plugin
  traffic proves nothing about the boundary.
- **Per move:** computer → `explore-20` with an active terminal and a viewed
  screen; realms → a 200-player profile (`D62-REALMS-PLAN.md:25-26`), with the
  feature's own cost (feature off against on, same pin) reported apart from the
  ABI cost; economy and settlements → `villager-heavy`, `core_bench
  peaceful100` and one Spain trial (`DESIGN.md` §4); battles → the 10k and 22k
  battle takes (`tools/battle/run-battle.py`) and the battle tick micro test;
  any A3 claim → `horizon-100-roam`.
- **Report:** three alternating pairs B, A, B, A, B, A. TPS, MSPT p50/p95/p99
  and max, the laps of the moved systems (`MC_CORE_PHASE_LOG`), host-thread CPU,
  callback and import counts, callback p50/p99, time from request to effect,
  peak backlog, refusal and lost-event counts; per-pair deltas, the B/B noise
  (spread of the three B runs) and peak as well as mean; never a cross-pair
  mean (`DESIGN.md` §4).
- **Verdict, applied in this order** to MSPT p50 and p95 over three
  reproducible pairs, each pair's full delta (A against its paired B, never the
  delta minus the noise):
  1. A new loss of required events fails the move, whatever the timings.
  2. TPS falling below 20 on a profile where B holds 20 goes to the owner
     first.
  3. If the B/B noise (the spread of the three B runs) is too wide to tell a
     5% slowdown from spread (the noise is 5% or more, or the pairs disagree in
     sign beyond the noise), the result is **inconclusive**: it does not allow
     the merge and is re-measured (longer runs or a quieter window).
  4. All pairs inside the noise: **neutral**. A reproducible improvement beyond
     the noise passes and is reported.
  5. A reproducible worsening beyond the noise in all three pairs: the full
     paired delta decides; under 5% on both p50 and p95 in every pair is
     **small** and lands with its numbers reported; 5% or more on either in any
     pair is **large** and goes to the owner first.

  These are the proposal of owner question 1; until the owner answers, Main
  applies them and reports every move's exact numbers.

## 4. Migration order

Decided with the reviews (both voted for this order): service cut → computer →
realms → economy → settlements onto WIT → armies and battles. Realms
preparation may run in parallel with the computer step after the owner's D62
GO.

| Step | What | Depends on |
|---|---|---|
| 0 (running) | D65 settlements crate: S0–S8, then C1–C3 (`DESIGN.md` §3.2-3.3), accepted only through its gates: the S0 byte oracle (same stages and ticks, fixed salt and UUIDs, base-vs-base in separate processes, three isolated controls; §2.9, lines 44-47); C1 decoding every image's schema in the shared batch decoder before routing and fold (§2.8, lines 39-43); the offline proof by `recover_offline` → `encode_checkpoint`, export and the custody and crash censuses, not `compact` (§2.9); S6 as a separate sequential cut of typed synchronous owner-thread calls (§2.4, lines 36-38); S7 keeping the reserve check on the generic `Begin` for every owner (§2.5, Main ruling Q4); the release-speed gate (§4). D65 rev. 2 is not accepted yet: the open items of `receipts/CoreUnload/REVIEW-sol-r2.md` stay part of the C3 gate and are not counted as done here. Duplicate-plugin deletions by the audit (D66.4). | — |
| **1** | **Service cut** (§4.1) | — (parallel with 0) |
| 2 | Computer: block identity and the terminal and screen UI flow as a package; every open/input authorization, revocation, the process bridge and the frame stream stay in `mc-edge`, never through the Wasm FIFO | 1 |
| 3 | Realms (new package, D62): treaties and their terms (vassalage, tribute amount and due dates), capitulation by treaty, chronicle, map; a separate package per D62 (`OWNER_DECISIONS.md:371-372`), using the engine-served `settlements` service. Treaty terms are owned by realms as package bookkeeping in the bounded KV (`DESIGN.md` §2.7); no engine operation reads them in this step, no tribute is paid yet and arrears give no casus belli yet | 1; owner GO for D62; D65 S2 owned state before any new settlement operation except the steward (`D62-REALMS-PLAN.md:73-77`); native war (D62 step 1); D63 map v2 for the map; a captive state for ransom |
| 4 | Economy: economy rules and ledgers into package records; payments, wages, and the actual tribute payment that settles a realms obligation, each as one typed native operation with a durable receipt (§1.3), never a transfer followed by a separate KV write; the tribute obligation becomes a native row here with one owner, and overdue tribute (the D62 casus belli) starts here; goods stay engine custody | 1; an **accepted** D65 C3 (C1 alone is not enough) under the step-0 gates; then the separate D66 format cutover with one writer (`DESIGN.md:23-27`, §1.3); native package rows with a declared schema. Tribute payment and arrears additionally need step 3 (the treaty and its terms exist); the rest of economy does not wait for step 3 |
| 5 | Settlements onto WIT (D66.3): founding, growth, certification rules, households, districts, migration rules, presentation, commands, and the executors' game rules (§1.1); the `settlements` provider switches from the engine to the package here (§2.8) | 4 |
| 6 | Armies and battles: rosters, staging, order vocabulary, war policy and the tactics, fatigue, lethality and casualty rules of `formation.rs` and `combat.rs` as package data and decisions; each active battle bound to its saved, validated parameter set (§1.1, a battle-image format change with its own persistence review); the executors stay | 5; independent battle-core review (AGENTS.md rule 5); A11 SIGKILL continuation with the same rules and refusal with changed ones |

Why this order: the computer is the smallest move, migrates no data, has no
tick cost and proves the presentation and block-event services. Realms has no
code to move and is the first real consumer of `settlements`. Economy is the
lowest-cadence, custody-heavy part of settlements and proves typed operations
with receipts. Settlements need all of that. Battles are the hottest code and
go last.

### 4.1 First move: the service cut

- **Change.** Split `crates/mc-script/wit/` into the service packages of §2.4,
  each at `0.1.0`. Records move verbatim; only the envelopes change. The request
  path (§2.2), the delivery order and call count (§2.3) and the pump phase stay
  as they are. `mc-plugin-host` links per declared interface, checks versions
  exactly (§2.6) and keeps the staging of `delivery.rs:28-35`. `mc-script`
  replaces `api`, the single-version check and the capability strings with the
  `[services]` table and keeps every other declaration of §2.5. The SDK
  (`sdk/rust/krasnokamen-plugin-sdk`), its examples, [plugins](../PLUGINS.md),
  [plugin contract](../plugins/CONTRACT.md) and [SDK](../plugins/SDK.md)
  follow. Every package the audit keeps in `solaris-default-plugins` is rebuilt
  and the live pack and matrix pins are re-pinned in the same bundle. The dead
  contract of §2.7 is deleted. Persisted DTOs stay byte-identical.
- **Measurement in the same slice.** A boundary bench in `mc-plugin-host`, on
  the reference box in the `-p` lane, before (0.9.0) and after the cut: a no-op
  callback; callbacks with 0, 1 and 32 imports; interleaved services; 1/32/256
  events of fixed size; a byte sweep up to the per-callback budget; a peak of
  due timers; time from request to effect on a busy host. It yields the fixed
  call cost, the per-import and per-byte costs and a Wasm factor for a fixed
  guest loop, which every later move's budget uses.
- **Acceptance.** L2 green; every guest the audit keeps is rebuilt against the
  split API, and the plugin regressions and the standard-pack load test pass
  with that package set; the old 0.9.0 monolith component is refused before the
  world opens (§2.6 negative control). The inventory-storage transaction keeps
  inventory and KV consistent after reopen, and a resubmission from a new live
  session of the same player with the KV version an earlier attempt already
  spent debits nothing twice; a domain operation resubmitted with its operation
  id after a lost answer returns the stored outcome and a different payload is
  refused (§1.3); both through the real host path. The §3.4 overhead control
  shows no worsening, the paired B/A with plugin traffic is measured and its
  TPS/MSPT change reported under the §3.4 verdict (a small slowdown lands, a
  large one goes to the owner first, an inconclusive one does not merge); bench
  receipts before and after. The cut changes the admission path of inventory
  transactions, so it gets an independent review before Main commits it
  (AGENTS.md rule 5, custody).
- **Not in it:** no new capability, no behaviour change in the tick, no move
  of gameplay, no delivery aggregation. New D62 settlement operations land after
  it, as `krasnokamen:settlements` 0.x bumps that rebuild their importers.

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
  (`<work>/solaris/.analysis/committee/q006-settlements-into-core.md:6`).
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
  7fe8efa; `7fe8efa:README.md:39-51`).

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

Reasons 2, 4 and 5 come from the October committee study of the Rust package,
not from a measured failure of the September Lua package under load. The v2
instantiation failure was the moment of deletion, not a reason.

### 5.3 What changed

- D66 reverses reason 1 for our gameplay, with the crate first (D66.3).
- Reasons 2, 5 and 6 do not recur by construction: this split keeps bodies,
  allocation and worker execution in native executors and moves rules, records,
  decisions and presentation. The September package put housing, work policy
  and body control in the guest.
- Reason 3 is closed only by typed native operations with durable receipts
  (§1.3): today's inventory-storage transaction covers one live session and the
  package's own KV with no durable operation id, and membership changes are in
  no transaction a package can request.
- The D65 design, with C1 corrected, gives the settlement codec to the domain
  and checks every image before fold (`DESIGN.md` §2.8 and lines 39-43): the
  basis for package-owned rows with a declared schema.

### 5.4 What still blocks the move back

- Reason 4: no native package rows with a declared schema yet (§1.3).
- One serial host thread with 256/1,024 bounds and a halt on unrefusable
  backlog; the boundary cost is unmeasured (§3.1).
- API gaps: dialog clicks, block use, war, army, steward, map, typed economy
  operations, package-provided services (§2.1, §2.8).
- The exact-version monolith rebuilds every package on any change (§2.1).

Step 1 removes the last of these; steps 4 and 5 remove the rest before any
settlement record leaves the engine.

## Consequences and risks

- **Executor creep:** executors can grow hidden game rules again. Review rule:
  an executor takes data and names no settlement, realm or war concept and no
  tuning constant of our design; a new executor or engine point needs review,
  as the D65 S1 ratchet does for inbound references (`DESIGN.md` §3.2).
- **One host thread** may saturate as packages grow; per-package threads are
  added only after a measurement shows it.
- **Persistence format:** moving settlement records into package rows changes
  their keys and owners; it gets its own persistence review and follows owner
  question 2.
- **Churn:** step 1 rebuilds every package once; afterwards a change rebuilds
  only a service's importers and exporters.
- On acceptance, add the D66 row to [open transitions](../STATUS.md#open-transitions).

## Rejected alternatives

- **Guest code per entity, per cell or per hit** (settlement brains or battle
  AI in Wasm): `0013-core-model-v2.md:564-565`; at A3 scale it cannot fit the
  queue bounds (§3.2).
- **Keeping game rules inside the executors** and moving only rosters and
  orders: the engine would keep our gameplay (§1.1).
- **One monolithic API version:** today's exact match rebuilds every package
  for any change (§2.1).
- **Widening the string KV** to hold settlement records: one aggregated
  checkpoint row, no schema, no per-record WAL images (§1.3).
- **Roblox-style permanent deprecation:** a compatibility surface of our own.
- **A per-hit `before-damage` hook for war legality:** a guest round trip per
  hit at battle volume; rules as data instead (§3.3).

## Decided with the reviews

These were owner questions in the first draft; both reviewers voted the same
way and they follow from D62, D66 and ADR 0013, so they are decisions here:

1. **Execution line:** per-entity, per-hit, per-cell and custody execution
   stays native; packages give rules, data and decisions (§1.1).
2. **Realms stays a separate package** over the settlements service, as owner
   D62 chose (`OWNER_DECISIONS.md:371-372`); routing between packages is built
   only at step 5 (§2.8).
3. **Order:** service cut → computer → realms → economy → settlements onto WIT
   → armies and battles; realms preparation may run in parallel with the
   computer after the owner's D62 GO (§4).

## Open questions for the owner

1. **"Small slowdown."** Proposal, in this order, on the full paired delta of
   MSPT p50 and p95 over three reproducible pairs: a new loss of required
   events fails; TPS falling below 20 where B holds 20 comes to you; noise too
   wide to separate a 5% slowdown is inconclusive and blocks the merge; inside
   the noise is neutral (a confirmed speedup passes); a reproducible worsening
   under 5% on both is small and lands with its numbers, 5% or more on either
   is large and comes to you first. The plugin-free overhead control is a
   separate, stricter gate: no worsening at all (§3.4).
2. **Settlement records into package rows: converter or reset?** Proposal: keep
   the format where possible; otherwise a one-off offline converter of
   checkpoint plus WAL into a new directory with custody and high-water checks;
   a reset only for a world you have explicitly marked disposable, never
   automatically (§1.3).

## Review resolution

Reviews of 779ef6888: `receipts/EngineADR/REVIEW-sol.md` (S1–S7) and
`receipts/EngineADR/REVIEW-astra.md` (A1–A7); M1 is Main's battle-rules note;
D65-r2 is finding ADR-0024-6 of the D65 re-review
`receipts/CoreUnload/REVIEW-sol-r2.md`. All HIGH and MEDIUM findings:

| Finding | Severity | Section | Resolution |
|---|---|---|---|
| S1 batch admission is not atomic execution; "at the next merge" is wrong | high | §2.2 | Imports return a correlation id; one bounded per-callback buffer; whole batch reserved at return and dropped on trap or refusal; `plugin::pump` before the dimension tick re-validates; atomicity only inside one composite operation |
| S2 a shared WAL frame is not a transaction; a request id is not idempotence | high | §1.3, §4.1 | Queue → apply → one WAL frame → append ACK → result, ACK not a directory fsync; typed native operations with durable receipts for economy and settlements; D45 only for independent state; gate through the real host path with refusal, lost answer, reopen, retry |
| S3 4 KiB KV and the 16 MiB envelope are different limits | high | §1.3 | No wider string KV; separate native rows with touched-only WAL images; 16 MiB is a ceiling; bounds from real records checked before custody; WIT carries owned values and ids; membership in the entity image, transit and reservations native |
| S4 an opaque payload is not fail closed | high | §1.3 | Per record kind: owner, schema version and a declared-data validator used by live and offline recovery and WAL verification without a gameplay callback; refuse before serving and before fold; KV string checks and startup identity are not schema validation |
| S5 no global sequence; coalescing changes refusal and visibility | high | §2.3, §3.3 | Per-recipient FIFO; the first cut keeps call count, order, routing, single-batch refusal and the shared timer budget; no regrouping; later aggregation only with native seal order; apply tick may differ; deadlines from persisted game time, not the freeze-advancing WAL tick |
| S6 economy after C1 alone could break the byte-exact D65 cut | high | §1.3, §4 | No `SettlementRecord` field leaves before C3; the corrected D65 gates listed in step 0; format changes only in a separate D66 cutover with one writer |
| S7 versions and routing ambiguous | medium | §2.6, §2.8 | Exact manifest/import/host match in the first cut; persisted DTOs unchanged; provider switch at step 5; asynchronous route with the original actor, bounded answer, no delegated rights; custody success only after the append ACK |
| A1 order cannot be kept while calling each service once | high | §2.3 | Same as S5; refusal notice reaches packages without `commands`; nothing of a failed callback is published |
| A2 `[services]` does not define rights, subscriptions, phases | high | §2.5 | Declarations table with their checks (events and handlers, commands and permissions, one provider, payload channels, spawn allow-list, hooks, Loader rights); phases; full `krasnokamen-towns` manifest and component world |
| A3 a new required export breaks old guests | high | §2.6 | Three versions distinguished; incompatible 0.x refused; a new required export is breaking; optional imports never required by old binaries; one bundle with exporters and transitive users; acceptance tests |
| A4 the storage fence could be lost; joint admission is not joint commit | high | §1.3, §2.7, §2.8 | Guards stay on the native request (checked at apply, part of the resubmission comparison); economy and tribute actions are named typed operations with receipts and lost-answer recovery; one provider, original actor, correlation, answer after the owner's decision; a message is not a WAL transaction |
| A5 perf gates prove neither boundary cost nor A3 | high | §3.2, §3.4, §4.1 | Empty-engine `native-100`/`explore-20` as overhead control; paired B/A with the same post-audit pack, world and command stream and non-zero callbacks, requests and effects; A3 only via `horizon-100-roam`; realms feature cost apart from ABI cost |
| A6 a mean does not bound the peak; requests do not bound work | high | §3.1, §3.2, §3.3, §4.1 | Separate counts (callbacks, imports, bytes, members, native checks, fan-out); ≥ 313 requests for 20,000 fighters, 256 is outstanding not throughput; regiment id + revision or bounded portions; native due queue instead of 1,000 timers; bench with 0/1/32 imports, interleaving, peak due, time to effect; byte budget counted before staging |
| A7 dependency graph incomplete | medium | §1.2, §2.7, §4 | Computer moves UX only; realms after D65 owned state, D63 map v2 and a captive state; corrected C1 before settlements; provider at step 5; `spawn-entity` kept for sky-creeper |
| A-history: line 6, not 5; reasons 2, 4, 5 are from the October study | — | §5.1, §5.2 | Fixed and stated |
| M1 battle executors hold game rules (`formation.rs:11-44`, `combat.rs:40-80`) | — | §1.1, §1.2, §4 | Mechanism vs rules table; tactics, fatigue, lethality and casualty policy move to the battles package as data and decisions, measured A/B; the same split stated for allocation, worker actions, construction, transit and economy |
| D65-r2 ADR-0024-6 step 4 could start after C1 | high | §1.3, §4 | Step 4 depends on an accepted C3 under the D65 gates of `DESIGN.md` (S0 byte oracle §2.9, C1 per-image decode before fold §2.8, offline `recover_offline` → `encode_checkpoint`/export proof §2.9, S6 sequential cut §2.4, S7 generic reserve check §2.5, release gate §4); format changes only in a separate D66 cutover with one writer (`DESIGN.md:23-27`), so both plans allow the same order |

Second round, reviews of 6fce5d95e: `receipts/EngineADR/REVIEW-sol-r2.md`
(S3, S4, S6 resolved; S1, S2, S5, S7 reopened; three new) and
`receipts/EngineADR/REVIEW-astra-r2.md` (A3, A5, A6, history resolved; A1, A2,
A4, A7 reopened; three new). Every open item and new finding:

| Finding | Severity | Section | Resolution |
|---|---|---|---|
| S1-r2 "nothing of the callback is published" is wider than the guarantee | medium | §2.2, §2.4 | The batch-drop guarantee covers the request buffer and the timers staged in that callback only; synchronous host-local imports take effect at once (a permission publication replaces the table) and are outside it |
| S2-r2 / A4-r2 one lost-answer gate for two contracts; "replay" wording | high | §1.3, §2.7, §4.1 | Inventory-storage transaction (no receipt, no blind retry): inventory and KV consistent after reopen, and a resubmission with the already spent CAS version debits nothing twice; domain operation (durable operation id): the stored outcome returned on resubmission, a different payload refused; the guard comparison is called resubmission, not WAL replay; no new receipt in the API-only cut |
| S5-r2 / A1-r2 FIFO across lanes; timer refusal differs | medium | §2.3 | FIFO per lane with today's arbitration between lanes; ordinary transient refusal sends `command-batch-rejected`, timer-delivery refusal keeps the schedule, delivers later and sends no notice; determinism stated for identical per-tick input |
| A1-r2 `server.started` callback in the example | medium | §2.5 | Removed: no contract event carries it, and towns loads from `init` |
| S7-r2 provider routing does not say who may change whose records | high | §2.8 | Originating consumer (package and actor) and provider (record owner) distinguished; the native operation checks the consumer actor's rights and the provider's right over its own records; the receipt is bound to both parties and the operation id; no identity substitution, no provider rights passed to the consumer |
| Sol-new-1 / A7-r2 step 3 pays tribute before the mechanism exists | high | §1.2, §4 | Step 3 records treaty terms, amount and schedule only, with no payment; the payment that settles the obligation is one typed native operation with a receipt at step 4, where the obligation becomes a native row; never a transfer plus a separate KV write |
| Sol-new-2 / A2-r2 `configure` "with no runtime import linked" is impossible; `death_graves` missing | medium | §2.5 | One component, two stores, one linker; Wasmtime links all imports before the first call, so phases are enforced at call time; enqueue only in `init` and ordinary handlers, refused in the startup store (initialization included) and in hooks; host-local calls listed per phase; `death_graves` restored as its own deployment right |
| Sol-new-3 / A3-r2 step 1 cannot load an unchanged old component | medium | §2.6, §4.1 | Step 1 rebuilds every kept guest and uses the old 0.9.0 monolith as the negative control; the unchanged-component check runs only between two versions of the split API |
| Astra-new-1 battle rules not bound to the saved battle | high | §1.1, §1.2, §4 | An active battle keeps one immutable validated parameter set saved with its image and restored on reopen; a different deployed set refuses before the tick; periods and denominators validated at admission; derived lethality coefficients computed once; gate: A11 SIGKILL continuation with the same rules and refusal with changed ones |
| Astra-new-2 correlation and result promise wider than the API | medium | §2.2 | Enqueue-local correlation id distinguished from ids owners return; zone answers are keyed by zone, menus are fire-and-forget; append-ACK applies to commit results; no new callback per import |
| Astra-new-3 the 5% rule allowed two verdicts | medium | §3.4, owner question 1 | Threshold on the full paired delta; neutral, small, large and inconclusive defined; the plugin-free overhead control is a separate, stricter gate that must be neutral |
| A6-r2 "one tick's queue" | — | §3.2 | Now "the 256-command outstanding capacity", a bound on commands waiting at once |
| D65 status: rev. 2 not accepted | — | §4 step 0 | The open items of `receipts/CoreUnload/REVIEW-sol-r2.md` stay part of the C3 gate and are not counted as done |

Third round, reviews of 6dade3c21: `receipts/EngineADR/REVIEW-sol-r3.md`
(S2, S5, S7 and the three second-round new findings resolved; S1 open; three
new) and `receipts/EngineADR/REVIEW-astra-r3.md` (A1, A2, A4, A7, battle rules
and correlation resolved; threshold open; one new). Every open item, new
finding and note:

| Finding | Severity | Section | Resolution |
|---|---|---|---|
| R3-1 the actor is not attested on the provider → native call | high | §2.8 | The existing host attestation (`route_authority.rs:1042-1059,1368-1382`, `command_admission.rs:247-257`) extends to the routed context: the host records consumer package, actor and correlation under a host-issued route handle, and attaches consumer, actor and registered provider at admission; native never takes them from guest-returned fields; handles are runtime-only; the receipt binds the player UUID (not the session), both packages and the operation id, so resubmission after reopen matches; a new mutation still needs the actor's current rights |
| S1-r3 timer refusal still promised "nothing of that pushed tick" | medium | §2.3 | Limited to the request batch and the staged timer schedule; synchronous host-local imports the timer callback made stay applied, as in §2.2 |
| R3-2 tribute payment did not depend on step 3 | medium | §4 | Step 4's tribute payment and arrears also depend on step 3; the rest of economy does not wait for it |
| R3-3 phase table missed runtime initialization and shutdown; startup KDF rule presented as current | medium | §2.5 | Rows for runtime-store initialization and `shutdown` with enqueue refused and their host-local calls named; host-local calls keep today's rule (only hooks refuse them), so the KDF stays allowed in the startup store: no tightening |
| A2-r3 note: keep the single-owner check for `death_graves` | — | §2.5 | A second declaring package refuses the deployment before start (`plugin.rs:593-612`) |
| Threshold-r3 one receipt could be both neutral and large; a speedup failed the overhead control | medium | §3.4, owner question 1 | Precedence: lost required event fails; losing held 20 TPS goes to the owner; noise too wide to separate a 5% slowdown is inconclusive and blocks the merge; inside the noise is neutral; reproducible worsening is judged on the full paired delta (under 5% small, 5% or more large); the overhead control allows no worsening, a confirmed speedup passes |
| S2-r3 note: stale-CAS control after reopen | — | §1.3, §4.1 | Runs from a new live session of the same player with the old KV version |
| Astra-new-r3 treaty owner differs from D65 | medium | §1.2, §4 | Consistent with `DESIGN.md` §2.7 (rev. 4): treaty terms, ransoms and chronicle are owned by realms; settlement records hold the native war state, steward and outcome with a treaty reference; from step 4 the tribute obligation is one native row with one owner, changed only with its payment |

[Architecture overview](../ARCHITECTURE.md) · [Decisions index](README.md)
