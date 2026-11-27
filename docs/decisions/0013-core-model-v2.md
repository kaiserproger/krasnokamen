# ADR 0013 — Native core model v2

**Status:** Accepted; implemented; the governing core model.
v1 is retired ([decision P8](../../DECISIONS.md#product)). Checked against the code on
2026-11-27: serial deterministic merge `crates/mc-core/src/tick/merge.rs:593`,
one stop order `crates/mc-core/src/core.rs:1828`, edge publish budget
`crates/mc-edge/src/server/pump.rs:55`.

This record keeps the decision and its binding rules. How each rule is carried
out lives in the architecture pages: [runtime](../architecture/runtime.md),
[state ownership](../architecture/state-ownership.md),
[mutation routes](../architecture/mutations.md),
[persistence](../architecture/persistence.md),
[residency](../architecture/residency.md) and
[settlements](../architecture/settlements.md). Earlier, longer versions with
per-feature notes are in Git (`git show b24ee531:docs/decisions/0013-core-model-v2.md`,
319 KB; `git show 973e6e60e:docs/decisions/0013-core-model-v2.md`, 44 KB).

## Decision

Use one native authoritative tick and a single cross-domain recovery boundary.
Parallelize entity work by data, not by spatial ownership. The edge owns
connections and presentation, never gameplay authority. Published snapshots
are immutable observations; retaining one does not lock or stall the tick.

The design goal is to make the most of one machine, not distribute a world
across machines, under both crowded and spread loads of roughly 1,000 players
and hundreds of thousands of mobs. These are targets, not demonstrated capacity.
Parallelism is by data; space is used for locality. Gameplay is vanilla-like,
not a promise of exact vanilla sequential entity order.
Within simulated areas, retain mob goals rather than cutting behaviours by
activation range; cadence may vary. Plugin mob designs compose generic native
goals, sensors and parameters, not a distinct per-type tick path.
[ADR 0016](0016-mob-capabilities.md) separates the implemented capabilities from
the remaining proposals.

## Tick and merge

The tick drains decoded player inputs, admitted plugin operations and ticket
changes. Parallel entity workers read the prior snapshot, update their own
next state and emit intents. Each intent's footprint includes every target,
source and dependent effect. The native merge validates against accumulated
state in deterministic actor/sequence order; conflicting or stale work is
refused whole. Merge is serial today. Block, random and scheduled work settles
under native tick ownership. A tick change set and immutable next snapshot
then feed persistence and presentation. Not every direct input mutation is an
entity intent: each path still preserves its complete custody boundary.
Within a block-update chain, preserve vanilla order. Independent-zone parallel
execution is a model constraint, not a claim that a general scheduler exists.

- Random ticks draw the requested `random_tick_speed` count per eligible active
  section; cost scales with that count, there is no fixed per-tick budget.
- Client game age follows simulated ticks, not journal frames; daylight time is
  independent ([tick and wire clocks](../architecture/state-ownership.md#tick-and-wire-clocks)).
- Transient bindings such as spectator cameras and derived relations such as
  mount passengers are native state with one owner each
  ([cameras](../architecture/state-ownership.md#spectator-cameras),
  [mounts](../architecture/state-ownership.md#mount-custody)).

## Threads and backpressure

Long-lived core workers do parallel gameplay work; edge socket and fanout
workers are separate; guest callbacks run outside the tick; journal and native
writer workers own disk I/O and checkpoint commits; datapack reloads and trace
writes run on a short-lived worker. Binding rules
([runtime](../architecture/runtime.md#reliable-delivery-and-backpressure)):

- Every cross-thread queue is bounded in bytes or items, or has a written
  reason and a counter ([state ownership rule 8](../architecture/state-ownership.md#rules)).
- Journal admission waits under pressure; accepted batches are never dropped or
  reordered. A dead journal stops the core.
- Reliable per-player results are ordered, delivered once and bounded;
  exhaustion disconnects, it never silently discards. Visual-only effects are
  best effort and may drop.
- World fanout may reconcile a viewer to a newer snapshot. The edge pump skips
  outputs that waited past `PUBLISH_QUEUE_BUDGET` (1.6 s) while a newer one is
  queued, never the newest; reliable changes still drain only through the
  published output. A lost knockback impulse closes only its recipient
  (`skipped-explosion-knockback`); a pump that stops for good halts the core.
- A slow plugin cannot stall the tick: plugin admission refuses new commands
  while its event queue is a quarter full, and halts fail closed only when
  unrefusable events overflow it.

## Persistence and recovery

Native redb state and the tick WAL are the only serving authority; Anvil is
explicit offline interchange, not a fallback or dual write. Binding rules
([persistence](../architecture/persistence.md)):

- All dimensions and player, world, entity and plugin changes share one
  checkpoint and replay boundary. Some non-player motion is checkpoint-only.
- An append acknowledgement is not an fsync guarantee: a crash may roll back an
  unflushed tail, consistently across domains. No per-tick fsync and no
  synchronous DB write on the tick.
- A checkpoint commits only after durable WAL coverage of its tick; only a
  committed checkpoint permits pruning; a failed WAL write, fsync or rename is
  fatal and never retried.
- Corrupt, incomplete, inconsistent or unsupported required state refuses
  startup instead of losing custody. The one exception is a quarantined chunk,
  whose bytes are never rewritten: without unsaved state it is skipped; with
  unsaved state the whole checkpoint is declined and the WAL kept.
- Our own formats have no compatibility read path and no downgrade path: a
  world written by a newer schema refuses older binaries and vice versa; restore
  a matching backup instead. The only conversions are explicit stopped-world
  commands for a named case (`world upgrade-worldgen`, `world upgrade-game-clock`).
  Compatibility targets are vanilla protocol and Anvil.
- Generated chunk baselines become durable before dependent WAL publishes;
  generators do not hot-swap. World identity fences generator revision, seed and
  effective worldgen inputs ([decisions G2–G3](../../DECISIONS.md#world-generation)).
- Explicit shutdown and a plain drop share one stop order; the world lease is
  released only after the tick and journal owners are joined.

## Plugin admission

Independently deployed `krasnokamen:plugin@0.9.0` components execute outside the
tick, one callback at a time. Guests receive owned values and opaque handles,
not native pointers, locks, slots or world ownership; they never own world state
or a region. Observing an event is not a commit: a guest submits
capability-gated commands, and the native owner validates current identities,
revisions, complete custody and capabilities when applying them. Plugin KV and
native effects share the tick journal; completion follows append ACK, not
fsync. Unsupported declarations and invalid packages fail startup.
See [plugins](../PLUGINS.md) and [settlements](../architecture/settlements.md).

## Residency and tickets

Residency (storage and legibility) differs from gameplay activation. Player
view, simulation demand, explicit tickets and dirty checkpoint pins determine
residency; dirty pins do not by themselves activate gameplay. Active chunks
must be resident and legible. Durable tickets restore after restart; ephemeral
pearl tickets do not. Activation clocks and debt have native persisted custody;
no downtime catch-up is inferred. See [residency](../architecture/residency.md).

## Scale target and the hot set (owner decision, 2026-11-18)

By default the core is sized for about one million of whatever is hot, in any
scenario: plugins can make creatures or data hot that vanilla never does. The hot
set is chosen by access, measured or declared, never by a list of vanilla types.
Plugin components and entities get the same hot/cold split, pages, ordering,
residency and collective representation as built-in ones; a plugin keeps its
state across offload through materialization. A layout or fast path keyed on
vanilla types or on one workload is rejected in review. The architecture
committee works out the full design after the freeze.

## Far-away simulation and journal policy (owner decision D45, 2026-11-19)

Owner answers to the residency and journal design questions. They are design
input, not implemented; implementation follows the committee designs after GO.

- Beyond what is already accepted (game-time settlement accrual in
  [ADR 0023](0023-offscreen-settlement-simulation.md), catch-up of resident
  inactive chunks, nothing accrued during downtime or `/tick freeze`), all
  creatures far from players are simulated, wild mobs included, and births and
  deaths may happen far away. This changes vanilla farms, spawn limits and
  "what you left is what you find".
- The per-namespace custom-mob credit of [ADR 0016](0016-mob-capabilities.md#cost-accounting)
  may be revised after measurements, keeping the shared-tick protection.
- The journal redesign's store format change may reset the world again; no
  offline conversion is required.
- A warm standby's generation lag is accepted per durability class (custody
  from `wal_tick`, checkpoint-only state from the last verified generation's
  `checkpoint_tick`) without a fixed maximum, to be measured later. The host and
  date, service and re-seed times, and warning age remain open.
- Plugins may declare new state durable only at checkpoints: after a crash it
  rolls back to the last successful checkpoint. Existing custody and plugin
  storage guarantees do not weaken.

## Rejected alternatives

- Mutable spatial regions as the unit of parallel ownership: a crowd collapses
  onto one owner and cross-region custody complicates correctness.
- Sequential vanilla entity execution as the sole model: insufficient parallelism.
- Separate domain journals or per-decision fsync before reply: replace one
  recovery boundary with inconsistent or blocking decisions.
- Guest AI on each entity tick or synchronous in-tick plugin calls: a guest
  can stall gameplay and cost scales with the population.
- Long-lived v1/v2 dual serving and runtime Anvil fallback: competing authorities
  undermine the cutover and recovery contract.

[Architecture overview](../ARCHITECTURE.md) · [Decisions index](README.md)
