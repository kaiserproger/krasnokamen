# ADR 0013 — Native core model v2

**Status:** Accepted. This is the governing core model; [ADR 0014](0014-v1-core-retirement.md) retires v1.

This page keeps the decision and its binding rules; domain contracts live in
[docs/architecture](../architecture/). The previous 319 KB version, with
per-feature implementation notes and their history, is in Git:
`git show b24ee531:docs/decisions/0013-core-model-v2.md`. Code and tests win
over that text; use it only to recover why a feature was built a certain way.

## Decision

Use one native authoritative tick and a single cross-domain recovery boundary.
Parallelize entity work by data, not by spatial ownership. The edge owns
connections and presentation, never gameplay authority. Published snapshots
are immutable observations; retaining one does not lock or stall the tick.
See [state ownership](../architecture/state-ownership.md) and
[mutation routes](../architecture/mutations.md).

The design goal is to make the most of one machine, not distribute a world
across machines, under both crowded and spread loads of roughly 1,000 players
and hundreds of thousands of mobs. These are targets, not demonstrated capacity.
Parallelism is by data; space is used for locality. Gameplay is vanilla-like,
not a promise of exact vanilla sequential entity order.
Within simulated areas, retain mob goals rather than cutting behaviours by
activation range; cadence may vary. Plugin mob designs should compose generic
native goals, sensors and parameters, not require a distinct per-type tick
path. [ADR 0016](0016-mob-capabilities.md) distinguishes the implemented
capabilities from remaining proposals; this is a design requirement, not a
claim that every generic capability has already been implemented.

## Tick and merge

The tick drains decoded player inputs, admitted plugin operations and ticket
changes. Parallel entity workers read the prior snapshot, update their own
next state and emit intents. Each intent's footprint includes every target,
source and dependent effect. The native merge validates against accumulated
state in deterministic actor/sequence order; conflicting or stale work is
refused whole. Merge is serial today. Block, random and scheduled work settles
under native tick ownership. A tick change set and immutable next snapshot
then feed persistence and presentation. Not every direct input mutation is an
entity intent: each path still has to preserve its complete custody boundary.
Within a block-update chain, preserve vanilla order. Independent-zone parallel
execution is a model constraint, not a claim that a general scheduler exists.

## Threads and backpressure

Long-lived core workers perform parallel gameplay work; edge socket and fanout
workers are separate. Guest callbacks execute outside the tick. Journal and
native writer workers handle disk I/O and checkpoint commits. Bounded FIFO
journal admission waits under pressure; accepted batches are neither dropped
nor reordered. A dead journal stops the core. Reliable per-player results are
ordered and bounded; exhaustion disconnects rather than silently discarding
results. Bulk producers such as the scoreboard join snapshot and one scoreboard
publication send one ordered batch per connection, and the edge queues its
frames as one item, so a large scoreboard does not hit the bound. Chat and
system-message lines are bounded at their source (the sender's spam budget, the
command sequence budget), so a connection's trailing run of them compacts into
one `TextBurst` entry at the cap instead of disconnecting a recipient, and the
edge queues the burst's frames as one item. World fanout
may reconcile to a newer snapshot. See
[runtime](../architecture/runtime.md) and [persistence](../architecture/persistence.md).

Operator changes reserve the whole authoritative input batch before changing
edge-owned operator state. An async mutex serializes these changes, and the
reserved snapshot and live permission update are sent without an intervening
await. Queue saturation therefore waits for capacity rather than polling or
publishing an older operator snapshot after a newer one. If the core stops
while waiting, the operator mutation is not applied.

## Persistence and recovery

Native redb state and the tick WAL are the only serving authority; Anvil is
explicit offline interchange, not a fallback or dual write. All dimensions and
player, world and plugin changes share the same checkpoint and replay boundary.
An append acknowledgement is not an fsync guarantee: a crash may roll back an
unflushed tail consistently. Some non-player motion is checkpoint-only.
Checkpoints require durable WAL coverage before the atomic database commit;
only a committed checkpoint permits pruning. Corrupt, incomplete, inconsistent
or unsupported required state refuses startup, rather than silently losing
custody, except a quarantined chunk (never loaded or rewritten, stored record
byte-identical): one without unsaved state is skipped and the other chunks keep
checkpointing (only its chunk-local scheduled-tick projection is skipped and
counted; the manifest scheduler images keep the queues), one with unsaved
state (block writes, container records and removal marks) declines the
checkpoint (WAL retained), because committing the rest of the world without
those edits could duplicate custody. Each checkpoint logs the quarantined chunks with their counts. The
complete framing, first-start,
lease and backup contract is in
[persistence](../architecture/persistence.md).

## Plugin admission

Independently deployed `solaris:plugin@0.8.0` components execute outside the
tick. Guests receive owned values and opaque handles, not native pointers,
locks, slots or world ownership. The native owner validates current identities,
revisions, complete custody and capabilities when applying operations. Plugin
KV and native effects share the tick journal; completion follows append ACK,
not fsync. Unsupported declarations and invalid packages fail startup.
See [plugin contract](../PLUGINS.md) and [settlements](../architecture/settlements.md).

## Residency and tickets

Residency (storage and legibility) differs from gameplay activation. Player
view, simulation demand, explicit tickets and dirty checkpoint pins determine
residency; dirty pins do not by themselves activate gameplay. Active chunks
must be resident and legible. Durable tickets restore after restart; ephemeral
pearl tickets do not. Activation clocks and debt have native persisted custody;
no downtime catch-up is inferred. See [residency](../architecture/residency.md).

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
