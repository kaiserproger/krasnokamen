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

Datapack filesystem operations use the existing bounded worker slot. Admission
is not command success: `CommandResult::Deferred` identifies one pending result
in `command_context::Runtime`. The worker prepares data; only the native tick
publishes it and completes the matching result. The shared dimension boundary
applies execute-store callbacks and resumes execute branches and function frames
in order. A continuation retains its source context, fork accounting and command
sequence budget; resumption does not begin a new sequence. Other commands wait
in the existing bounded job queue rather than running a second interpreter.
Duplicate completion identities have no effect. Deferred continuations and
queued root commands retain the initiating connection's entity identity and
permission, independently of an `/execute as` target. Both single-world and
shared-world drains cancel stale sessions or revoked permissions before executing
commands, stores, continuations or feedback. Cancellation does not undo filesystem
work already performed. Plain `/reload` uses this same awaited result: following
function lines and trace reports wait for publication or explicit failure.

Debug function traces run those same function frames and publish their actual
`ChangeSet`; they neither dispatch a second interpreter nor invent per-command
results. A traced child may await I/O before the enclosing trace report is
written through that same worker slot. Trace recording is limited to the active
source's command feedback, including its own completion event, never unrelated
messages received while suspended. Output has a 16 MiB bound; exhausting either
that bound or the shared command budget produces a failed command result and
an explicitly incomplete report. A successful profiler/trace result is published
only after its report has been written.

Recipe and block-loot reloads prepare one immutable native content snapshot on
the existing datapack worker. Startup tables are shared, not cloned. The native
shared-world boundary publishes both catalogs and the pack selection together;
invalid preparation leaves all three unchanged. Open crafting/station menus
recompute their projections without moving inputs or the carried stack.
Reliable recipe-book events retain the exact native catalog that produced them.
The edge memoizes only derived wire packets, weakly keyed by that catalog, and
sends its station metadata before book displays and refreshed menu contents.
Neither a lagging pump nor a join racing reload may substitute the latest
catalog for an older queued event. Advancement definitions remain startup-only.

`/data` resolves one native accessor before preparing its NBT edit. Storage,
entity and block sources share the same path operations. Entity and block
destinations preflight the complete candidate through their existing codecs
and custody guards before the first mutation. They apply typed live changes,
not summon/placement initialization: entity UUIDs, dense slots, unrelated
cooldowns and resident block clocks remain owned by their existing runtimes.
Player destinations are refused. Entity post-images enter the existing
appearance journal fence; containers and block metadata use their custody
and opaque-image journals and normal viewer publication. NBT is a command
projection, not a second entity authority.
Sculk sensors, catalysts and shriekers use their resident codecs for both
projection and admission. Unrelated edits preserve phase/bloom/shriek deadlines
and the exact pending vibration; an authored listener delay replaces its arrival
relative to the current tick. Internal deadline fields are not command-writable.
Jukebox data edits publish playback transitions and `has_record` through the
resident owner; unrelated metadata does not restart a song. Lectern page and
brewing-property edits update existing viewers without replacing their menus or
moving their carried stacks.

Unrelated metadata edits on unopened loot chests preserve their typed loot-table
and seed owner rather than installing an empty live inventory. NPC equipment
edits prepare the exact prospective custody hashes and script-range revisions
before health, equipment or journal mutation; unchanged images need no revision
increment. This is not NPC reservation support: inventory `Reserve` admits only
player inventory and containers, rejecting NPC equipment/carry with
`InvalidRequest`. `/loot replace` admits the complete destination range and its
final aggregate reservation footprint before writing any slot.

Operator changes reserve the whole authoritative input batch before changing
edge-owned operator state. An async mutex serializes admission and changes;
the initiator's live permission and lock status are rechecked after reservation.
The operator snapshot and a permission update for every registered session of
the target UUID are sent without an intervening await. Queue saturation waits
for capacity rather than polling or publishing out of order. If the core stops
while waiting, the operator mutation is not applied.

Pickup publishes changed slots in the active menu. Player inventory slots hidden
by that menu use the dedicated player-inventory packet rather than a window-0
contents image, so another menu's cursor and crafting slots are not overwritten.

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

Independently deployed `krasnokamen:plugin@0.9.0` components execute outside the
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
