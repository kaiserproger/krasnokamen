# ADR 0023 - Off-screen settlement simulation

**Date:** 2026-11-16  
**Status:** Accepted design; migration transit, migration flows, game-time economy
clocks, and deferred production/consumption are implemented and integrated.
Construction labour and birth credit (slices 4–5) remain planned, not implemented.  
**Related:** [0013](0013-core-model-v2.md) (native authority, one tick WAL),
[0020](0020-entity-checkpoints-at-one-million.md) (entity tombstones),
[0021](0021-build-time-and-crate-split.md) (module cycle),
[settlement jobs](../settlements/jobs.md), [persistence](../architecture/persistence.md).
The retired v1 regional simulation (0005, 0009) is not a precedent.

## Problem

Owner decision 2026-11-16: settlements whose chunks are not simulated advance
abstractly on a coarse deterministic cadence — production and consumption,
construction progress, births and migration — with an explicit, tested
reconciliation when they are simulated again. Hard constraints: native store
plus tick WAL only, fail closed; no invented activity during server downtime;
no dupes; one native owner per accrual with a mandatory double-count test;
an explicit decision on block placement in unloaded chunks.

## What already advances off-screen

Current runtime boundaries (see [settlements](../architecture/settlements.md)):
*simulated* is `ChunkWorld::is_active`; *resident* is `has_chunk` and is wider.
Entity rows remain in memory unless native transit takes custody.

- Wages and caravan deadlines use game time. Payroll accrues elapsed periods
  off-screen and pays through real treasury storage when it is resident;
  caravan deadlines can expire without moving the carrier.
- Production outside the station's simulated area accrues bounded deferred
  portions, including when the chunk is resident. It never changes a worker's
  carry or physical storage there. An active owner visit reconciles through the
  ordinary recipe planner and fenced storage commit when all storage is resident.
  Unaffordable portions or output that cannot fit are dropped in that visit.
- Daily consumption runs for every settlement, with or without production jobs.
  One shared game-time watermark charges living body-backed members; unsimulated
  demand becomes a deferred debit, paid from real food on an active visit.
- Transit keeps complete native member images while bodies are absent. Due
  members enter only when their destination is simulated. Migration decisions
  and resident-death safety observations belong to the runtime policy.
- Construction, recruitment, births and villager brains still require simulated
  chunks. No construction labour or birth credit is implemented yet.
- Freeze and downtime add no elapsed game-time periods. The one-time legacy
  clock conversion and its removal conditions live in
  [settlements](../architecture/settlements.md).

The remaining construction/birth work is tracked in
[open transitions](../STATUS.md#open-transitions).

## Decisions

### 1. Clock and cadence

Every off-screen accrual uses game time (`tick_control.game_ticks(tick)`), never
`world.tick`. Game time stops during downtime and under `/tick freeze`, so
neither invents activity. An off-screen system runs at its own fixed period at
a phase derived from the settlement id, only while its subject is not simulated,
and accounts whole elapsed periods since its persisted watermark. It does no
work between periods and no catch-up beyond its watermark.

### 2. One owner per accrual

Each accrual has exactly one persisted counter or watermark (wage `next_due`,
job progress, a consumption or labour watermark). The on-screen system and its
off-screen counterpart are two code paths of **one owner module** that advance
the same counter, selected by one predicate evaluated per period (for example
"station chunk simulated"). A period consumed by either path is no longer
available to the other, before or after load, so reconciliation never needs to
subtract. The owners:

| Accrual | Owner | Off-screen rule |
|---|---|---|
| Wages | `commerce` (today) | unchanged authority; its clocks use game time (slice 2, merged) |
| Caravan deadline | `caravan` (today) | `game_deadline` in game time (slice 2, merged) |
| Production portions | `production` | stays its only writer; off-screen periods become deferred portions in the job (slice 3) |
| Consumption | `production`'s consumption owner (slice 3) | food demand per member per game day as a deferred debit |
| Construction labour | `construction` | labour credit in the project (slice 4) |
| Births | `households`/birth path | birth credit, materialised on load (slice 5) |
| Migration | `transit` (slice 1) | durable transit record, below |

The off-screen pass never writes a counter owned by another module and never
re-derives a period from wall time.

Slice 2 keeps this rule through the one-time legacy conversion: recovery
converts a store's world-tick clocks once, after the WAL fold and the global
game clock's restoration. Overdue wages keep their complete owed periods in a
one-time `legacy_owed` debit owned by `commerce` (not a second counter; `next_due`
stays the only watermark), caravans get a game-time deadline, recovery never pays
money and never forgives valid debt, and only inconsistent clocks or overflow
refuse startup. The rules and the shim's removal condition are in
[settlements](../architecture/settlements.md).

### 3. Block and item placement: deferred materialisation on load

No off-screen system writes blocks, block entities or container contents in a
chunk that is not resident. Off-screen effects are abstract state in the owning
record (deferred portions, debits, labour or birth credit). On the first active
owner visit with the required chunks resident, the owner applies them through
its ordinary physical path: inputs from real storage, outputs in real
containers, and (for planned slice 4) blocks through the structure executor.

Reasons: writing unloaded chunks would load them (cost scales with every
unloaded settlement), pin them resident through dirty-chunk eviction refusal,
and create a second, unobserved block writer next to the construction executor
and the physical inventory owner. Deferred application keeps one physical writer
per kind and lets the existing chunk WAL and checkpoint fences cover the result.
Consequence: off-screen progress is bounded by what the physical world allows
at load. A deferred effect that cannot be applied (inputs missing, no space)
applies the affordable part and drops the rest; nothing becomes a backlog that
outlives one reconciliation.

### 4. Construction progress (planned, not implemented)

Off-screen construction accrues labour credit in the project (assigned builders
× elapsed game-time periods, capped by the work left in the structure ledger).
No block is placed off-screen. On load the executor spends the credit at an
accelerated, bounded rate with physical materials from storage; credit without
materials stays in the project until materials arrive or the project ends.

### 5. Births (planned, not implemented)

No entity is created off-screen. Off-screen births accrue a bounded birth credit
for eligible households (free bed, food), evaluated with the existing birth
predicates on game-time days. On load the credit is realised as ordinary births
through the existing birth path at the home bed, in one tick. Transit custody is
not reused for births: it holds the complete records of entities that existed,
and a UUID that never had a body would have no source for the recovery check or
the census.

### 6. Migration transit (slice 1)

This restates the binding design (`receipts/SpainCulture/IMMIGRATION-DESIGN.md`,
"Durable transit record") as implemented in `settlements/transit.rs`.

- **Edge.** `edge_point` is the farthest standable surface cell on the bearing
  that is still entity-simulated, at least 16 blocks from the anchor and
  independent of the civil radius (owner 21:50). Departure and arrival happen
  there even when it lies inside the radius.
- **Departure tick.** Validation first, then in one tick: `households::leave`,
  capture each member's complete WAL entity image, remove the bodies without the
  death path, insert one record keyed by the lowest member UUID. The record
  arrives 10 game ticks per block from the exit to the destination anchor.
- **Refusals before mutation.** Founding household; a member in another
  dimension, converting, wandering or smelting; economic reservation or wages
  owed; construction work (a Builder or a borrowed builder, Main 2026-11-16:
  `leave` would turn a Builder into a mason or jobless villager, so the stored
  image would not be the body that left).
- **Arrival tick.** Due and the destination anchor simulated: restore the same
  entities at the edge on the reverse bearing, `join`, delete the record, one
  tick, at most 4 records per tick. Destination unsimulated: `awaiting_entry`.
  Destination deleted: retarget to the source, else the nearest settlement,
  else enter unbound at the old destination's edge (accepted deviation: the
  fallback does not check free beds; the destination policy owns admission).
  `transit::tick` returns each arrival that joined (`Arrival`: source,
  destination, household, member UUIDs) to the migration policy in the same
  tick; it is never persisted.
- **Entry retries.** If the return bearing has no entry, try the nearest
  standing foot cell within four horizontal and two vertical blocks of the
  anchor. If neither exists, or restoration refuses, retain custody and retry
  after 100 game ticks. The retry deadline is runtime-only and never changes
  the persisted record or its WAL image; the first refusal logs its reason.
  At most four entry searches/restores are attempted per tick, including failures.
- **Resident deaths.** The native damage owner records an actual resident
  death for the policy's seven-game-day safety window. Successful zombie
  infection is conversion, not death, and does not enter that window.
- **Automatic role holders.** Automatic migration also refuses an occupied
  certified native work point, not only commercial employment. The role must
  be released before selection or departure; an empty carry is not proof that
  a mine, lumber camp or workshop no longer needs its assigned worker.
- **Brain observation boundary.** Allocator → settlement policy → brain; see
  [migration policy](../architecture/settlements.md#migration-policy-and-physical-return).
- **Record.** Source, destination and its anchor, departure and arrival game
  time, bearing, exit, status, the household record `leave` removed, and each
  member's image in the WAL entity codec. Member images name no settlement or
  household; the record's household is the unit's only household identity in
  transit (the villager codec refuses a household without a settlement), and
  arrival restores it when the unit joins.
- **Persistence and recovery.** `transit/<uuid>` checkpoint rows and `Transit`
  tick-WAL images (`deny_unknown_fields`; binaries without the row kind refuse
  the store, so downgrade means restoring the pre-upgrade backup). At every
  committed tick each member UUID is in exactly one place, a world entity or one
  record; recovery, the offline census and `world custody-census` refuse every
  violation ([transit custody](../architecture/persistence.md#migration-transit-custody)).
  A member's entity tombstone stays final except for that identity's return
  from transit (ADR 0020).
- **Operator commands** (Main option A): `settlement transit depart <settlement>
  <household> <destination>` (exit = the household's current position, full
  validation) and `settlement transit list`. Handlers live in `transit.rs`; the
  parser and command tree belong to the settlement command owner.

### 7. Reconciliation tests

Each slice lands with native tests on real terrain and a negative control:

- a **double-count test** per accrual: one job, wage or project crosses
  simulated → unsimulated → simulated, with a checkpoint and reopen between,
  and the total equals the elapsed game-time periods exactly; the control
  disables the shared watermark and must count twice;
- WAL-replay disk tests at each new cut (commit, in progress, apply);
- for new custody, one real-process SIGKILL scenario per cut followed by the
  offline census, at least three runs per cut on the laptop lane.

Slice 1's evidence is `transit_disk_tests.rs` (departure published and
unpublished, in transit from the checkpoint row, arrival and its later
checkpoint, and a body-plus-record negative control) and the `transit-crash`
harness profile ([transit custody](../architecture/persistence.md#migration-transit-custody)).
Its controls: a forged published batch that re-adds a member body beside its
record (census FAIL, recovery refuses) and a binary whose departure leaves the
record out of the departure tick's WAL (the departure cut turns red).

### 8. Module boundaries

Each slice is a submodule of `settlements` (or a top-level module nothing in the
module cycle refers to) with its own state, systems, tests and row codec, a
narrow interface, and one registration point in `settlements::tick` or the
settlement command host. Placement does not bound the module cycle: code-health
attributes a submodule's `crate::`/`super::` references to its top-level host,
so every slice reports the largest module cycle from `cargo run -p xtask --
code-health` on the integrated tree and must not grow it (decision K5). The
[ADR 0017](0017-tick-system-graph.md)
system graph is not a registration path, and a registration point does not
break a cycle.

## Slices

Write sets are disjoint; each slice owns its tests and docs paragraph.

| Slice | State | Delivers |
|---|---|---|
| 1 Migration transit | integrated | section 6 |
| Migration flows | integrated | runtime policy owns membership relocation and immigrant spawn; weak physical return goals; [migration policy and physical return](../architecture/settlements.md#migration-policy-and-physical-return) |
| 2 Game-time economy clocks | integrated | wages and caravan deadlines stop under `/tick freeze`; double-count test across freeze; one-time legacy conversion (section 2) |
| 3 Production and consumption | integrated | deferred portions and daily food debit; shared game-time watermarks, physical load reconciliation and WAL/recovery tests |
| 4 Construction labour | planned: `settlements/offscreen_construction.rs`, one call in `construction.rs` | labour credit and accelerated application on load |
| 5 Births | planned: `settlements/offscreen_births.rs`, one call in the birth path | birth credit realised through the existing path |

The remaining slices 4–5 depend on sections 1–3 and use the integrated
game-time clock and native reconciliation boundaries.

## Consequences

- Implemented off-screen production and consumption advance without loading
  chunks; physical effects reconcile only during active visits with resident
  storage, bounded by available inputs and output capacity.
- Job deferred portions and consumption debit/credit share their owner's
  persisted game-time watermark, settlement WAL image and checkpoint boundary.
  Idle/retry scheduling is runtime-only and grants no additional accrual.
- The old resident-but-unsimulated production path is replaced, not retained
  alongside deferred production. Payroll retains its physical treasury rules.
- Construction and births remain simulation-gated until slices 4–5 implement
  their credits; the accepted design is not a claim that those features exist.
