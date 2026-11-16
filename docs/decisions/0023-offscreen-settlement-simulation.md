# ADR 0023 - Off-screen settlement simulation

**Date:** 2026-11-16  
**Status:** Proposed (draft until Main accepts it after independent review).
Slice 1, migration transit custody, is implemented on `wt/MigrationTransit`;
nothing else in this ADR is implemented.  
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

The inventory (owner host, `receipts/OffscreenSim/INVENTORY.md`, with
`file:line` evidence) and the pinning test `settlements/offscreen_pin_tests.rs`
establish today's behaviour. Terms: *simulated* is `ChunkWorld::is_active`;
*resident* is `has_chunk` and is wider; entity rows never leave memory.

- `economy::tick` visits every economically active settlement with no chunk
  check. Payroll accrues every elapsed period at once (`projected_accrual`) and
  is paid off-screen whenever the treasury container is resident; otherwise the
  backlog is paid as one lump at the first resident tick, or never while it
  exceeds the budget.
- Production completes portions off-screen when the station chunk is resident
  and the frozen worker body stands at its post; otherwise the job parks
  `unloaded` and re-arms every 20 ticks.
- Caravan deadlines expire off-screen although the carrier never moves.
- Construction, recruitment, births and every villager brain branch wait for
  simulated chunks. Nothing creates population, buildings or recruits off-screen.
- Every economy clock uses `world.tick`, which advances under `/tick freeze`
  while game time and the settlement phase stop; downtime is safe because the
  tick restores from the checkpoint.

No row of [open transitions](../STATUS.md#open-transitions) covers settlements
or residency.

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
"station chunk resident"). A period consumed by either path is no longer
available to the other, before or after load, so reconciliation never needs to
subtract. The owners:

| Accrual | Owner | Off-screen rule |
|---|---|---|
| Wages | `commerce` (today) | unchanged authority; its clocks move to game time (slice 2) |
| Caravan deadline | `caravan` (today) | moves to game time (slice 2) |
| Production portions | `production` | stays its only writer; off-screen periods become deferred portions in the job (slice 3) |
| Consumption | `production`'s consumption owner (new, slice 3) | food demand per member per game day as a deferred debit |
| Construction labour | `construction` | labour credit in the project (slice 4) |
| Births | `households`/birth path | birth credit, materialised on load (slice 5) |
| Migration | `transit` (slice 1) | durable transit record, below |

The off-screen pass never writes a counter owned by another module and never
re-derives a period from wall time.

### 3. Block and item placement: deferred materialisation on load

No off-screen system writes blocks, block entities or container contents in a
chunk that is not resident. Off-screen effects are abstract state in the owning
record (deferred portions, debits, labour or birth credit). On the first tick
the chunk is resident and the owner runs, the owner applies them through its
ordinary physical path in one tick: inputs are taken from real storage, outputs
placed into real containers, blocks placed by the structure executor.

Reasons: writing unloaded chunks would load them (cost scales with every
unloaded settlement), pin them resident through dirty-chunk eviction refusal,
and create a second, unobserved block writer next to the construction executor
and the physical inventory owner. Deferred application keeps one physical writer
per kind and lets the existing chunk WAL and checkpoint fences cover the result.
Consequence: off-screen progress is bounded by what the physical world allows
at load. A deferred effect that cannot be applied (inputs missing, no space)
applies the affordable part and drops the rest; nothing becomes a backlog that
outlives one reconciliation.

### 4. Construction progress

Off-screen construction accrues labour credit in the project (assigned builders
× elapsed game-time periods, capped by the work left in the structure ledger).
No block is placed off-screen. On load the executor spends the credit at an
accelerated, bounded rate with physical materials from storage; credit without
materials stays in the project until materials arrive or the project ends.

### 5. Births

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
- **Record.** Source, destination and its anchor, departure and arrival game
  time, bearing, exit, status, the household record `leave` removed, and each
  member's image in the WAL entity codec. Member images name no settlement or
  household; the record's household is the unit's only household identity in
  transit (the villager codec refuses a household without a settlement), and
  arrival restores it when the unit joins.
- **Persistence.** `transit/<uuid>` checkpoint rows and `Transit` tick-WAL
  images (`None` deletes); `deny_unknown_fields`; old stores decode as no
  records; binaries without the row kind refuse the store (unknown global
  domain), so downgrade means restoring the pre-upgrade backup.
- **Native store.** Settlement and transit rows are complete per checkpoint:
  the store deletes a stored row the image omits (before this, a deleted
  settlement left a stale row that the next read refused with a capture-tick
  mismatch). A member's entity row is tombstoned at the checkpoint after
  departure; the store accepts that identity's live row again only in the
  checkpoint that drops it from every transit row. Every other tombstone stays
  final (ADR 0020).
- **Invariant and recovery.** At every committed tick each member UUID is in
  exactly one place: a world entity or one record. Recovery refuses a UUID that
  is an entity and a member, a member of two records, or a record household
  that is also a settlement's, before membership validation. The offline census
  reports `transit_records` and `transit_members` and refuses the same cases.
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

Slice 1 has the disk tests (`transit_disk_tests.rs`: departure published and
unpublished, in transit from the checkpoint row, arrival and its later
checkpoint, and the body-plus-record negative control) and the census and
native-store negative controls. Its SIGKILL cuts run through the operator
command once the command registration lands.

### 8. Module boundaries

Each slice is a submodule of `settlements` (or a top-level module nothing in the
module cycle refers to) with its own state, systems, tests and row codec, a
narrow interface, and one registration point in `settlements::tick` or the
settlement command host. Placement does not bound the module cycle: code-health
attributes a submodule's `crate::`/`super::` references to its top-level host,
so the largest module cycle is measured with `cargo run -p xtask --
code-health` on the integrated tree for every slice (ceiling 187, measured 187
on 2026-11-16) and reported with it. The [ADR 0017](0017-tick-system-graph.md)
system graph is not a registration path, and a registration point does not
break a cycle.

## Slices

Write sets are disjoint; each slice owns its tests and docs paragraph.

| Slice | Writes | Delivers |
|---|---|---|
| 1 Migration transit (done on branch) | `settlements/transit.rs`, `transit_tests.rs`, `transit_disk_tests.rs`, `households.rs` leave/join, `registry.rs` field, `settlements/persistence.rs` hookups, `journal.rs`/`journal/replay.rs` `Transit` entry, `mc-world` native checkpoint rows, `mc-world-conversion` census | this ADR's section 6 |
| 2 Game-time economy clocks | `settlements/commerce.rs`, `settlements/caravan.rs` (+ tests) | wages and caravan deadlines stop under `/tick freeze`; double-count test across freeze |
| 3 Production and consumption | `settlements/production.rs`, new `settlements/offscreen_production.rs` (+ tests) | deferred portions and food debit with load reconciliation |
| 4 Construction labour | new `settlements/offscreen_construction.rs`, one call in `construction.rs` | labour credit and accelerated application on load |
| 5 Births | new `settlements/offscreen_births.rs`, one call in the birth path | birth credit realised through the existing path |

Slices 2–5 depend only on sections 1–3; slice 2 goes first because every later
watermark is game time.

## Consequences

- Off-screen settlements progress without loading chunks; their physical world
  changes only when simulated again, bounded by what is physically available.
- New persisted authority in slices 2–5 lives in the owning settlement record
  (serde defaults for old stores, no downgrade).
- Today's residency-gated off-screen behaviour (payroll paid and portions
  completed while resident but not simulated) stays until slice 3 replaces the
  production predicate; slice 2 removes the freeze accrual.
