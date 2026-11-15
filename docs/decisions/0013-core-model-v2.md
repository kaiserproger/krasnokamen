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

Client game age follows simulated ticks, not journal frame identifiers.
[State ownership](../architecture/state-ownership.md#tick-and-wire-clocks) defines the
freeze/step projection and how recovery restores that age; daylight time remains
independent.

Mount custody belongs to the vehicle's ordered, bounded rider UUID list. The
first rider with a live body controls motion; absent recovery claims are skipped.
Absent claims do not reserve attachment positions for live passengers.
A passenger's reverse link and published passenger entity IDs are derived,
never separate authorities. Removing one passenger preserves the other seats.
Native images retain the list; offline animal NBT uses `krasnokamen:Riders`.
The shared image reader validates duplicate UUIDs and the vehicle's seat capacity;
replay also rejects one UUID claimed by multiple vehicles. Before adding a rider,
live Ride removes that UUID from every other vehicle in the dimension and removes
absent claims (and an existing claim for that actor) from its target.
Only the camel (3, `Camel.canAddPassenger`) and happy ghast (4) seat more than one
rider. The earlier scalar `krasnokamen:Rider` NBT key and WAL `rider` field are
refused outright, so an old image fails the load instead of silently unseating
its rider. Deploy this build after a clean stop: an unclean WAL tail written by
an older build still carries `rider` and is refused at startup.

A recovered absent UUID is a pending claim, not an occupied seat: it survives idle
ticks until its passenger reconnects or another live Ride takes that vehicle.
No rider-kind is persisted, so an absent mob UUID follows the same rule.
`mounts::live_riders` resolves those UUIDs against live bodies and is the single
gameplay occupancy source for control, AI, leash follow, breeding, despawn,
portals and battle projections. There is no cached `EntityLifeState.mounted`
boolean to go stale on image load or removal of a passenger body; the durable
rider index intentionally continues to include absent claims.
Explicit leave/dismount, death, or a known dead/invalid passenger clears its
relation. Plain Join and successful plugin-login unlock both call `mounts::join`;
the active vehicle tick also rebuilds the reverse link and passenger projection.
Re-seating has no distance or chunk-activity check within the dimension: bringing
the returning player to the vehicle is accepted policy, not vanilla parity.
Dimension travel ejects seats and ends pending absent claims, but retains the
vehicle's chest and cargo. A later claimant joins at their saved position rather
than following a vehicle to another dimension. Death ejects seats and drops
that custody.
The WAL recovery regression is
`happy_ghast_admission_leave_and_reconnect_survive_wal_recovery`; native tests also
cover reclaimed seats, first-live steering, plugin unlock, competing claims and a
clean `Core::shutdown` with a still-seated player.

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
edge queues the burst's frames as one item. Only non-zero recipient explosion
knockback uses the reliable FIFO, summed into one packet per recipient/publish
independently of the visual spatial index. Visual-only explosions are
best-effort and may drop; bursts cannot fill the reliable FIFO with visuals.
No later snapshot restores a lost impulse. Each output therefore carries
transient cumulative per-player knockback-tick counts plus the last such tick.
Queue registration seals the latest native count under the registry lock.
The publisher aliases the output's canonical map Arc under a write guard held
through the synchronous broadcast; the scalar getter takes its read guard,
so registration is ordered wholly before or after a publication without
copying the map or introducing another count writer. The pump starts from that
sealed count, never lowers it while draining older outputs, and does not replay
pre-registration impulse publications into a newly registered world view.
Comparing retained outputs detects lost impulses across broadcast `Lagged`,
including a skipped impulse followed by another in the retained tick. Only
affected connections close (`skipped-explosion-knockback`); this observation
does not alter gameplay authority or persistence.
Entity ids remain stable through dimension travel: the observer resolves the
recipient across current worlds after travel commits. If the ordered dimension
reset moves a viewer away from that tick's explosion projection, the edge closes
only that recipient (`dimension-switch-explosion-knockback`) instead of silently
losing the impulse. It does not send an old-world explosion into the new world.
World snapshot deltas may reconcile to a newer snapshot; fanout plans within
each world queue's free frame slots and bytes, so a slow viewer's backlog is
bounded in bytes and its nearest, longest-waiting entity updates go first. See
[runtime](../architecture/runtime.md) and [persistence](../architecture/persistence.md).

Datapack reloads and `/debug function` trace-file writes run as jobs on a
short-lived `datapack-worker` thread (`crates/mc-core/src/datapacks.rs:1322-1326`),
handing the result back over a bounded `sync_channel(1)`, so they execute
outside the tick and never hold it.

Durable plugin events are bounded by their own queue, and one slow guest
callback stalls every plugin's delivery. The core therefore refuses new plugin
commands and drops plugin-menu clicks while a quarter of the queue capacity is
already waiting; the world halts fail closed only when events that admission
cannot refuse (graves, guest command results) pass the full capacity, and the
halt log counts the backlog by event kind.

Input-ordered chat proves native input handling, not geometry delivery: reliable
traffic can overtake the world queue. The operator-only `krasnokamen:setup_world_fence`
request keeps one transient nonce per connection. A later publication must have
the viewer's current dimension and complete geometry window, with no block or
chunk-repair debt, before the edge appends the nonce response to that same world
FIFO. The setup client dispatches all preceding frames before using the marker.
This changes neither gameplay authority nor durability; disconnect drops the fence.

Operator changes reserve the whole authoritative input batch before changing
edge-owned operator state. An async mutex serializes admission and changes;
the initiator's live permission and lock status are rechecked after reservation.
The operator snapshot and a permission update for every registered session of
the target UUID are sent without an intervening await. Queue saturation waits
for capacity rather than polling or publishing out of order. If the core stops
while waiting, the operator mutation is not applied.
Moderation commands re-acquire the same operator-update fence after selector
resolution and recheck the initiator's live permission and lock before any
kick, ban or pardon; the fence is held through those effects.

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

Player custody admission refuses missing, non-byte, out-of-range or repeated
inventory/ender-chest slots and repeated or unsupported equipment slots before
projecting entries into native arrays. Offline import also checks legacy
Inventory slots before PlayerEquipmentFix can collapse armor/offhand duplicates.
The error identifies the source player and slot; a refusal publishes no world
and leaves the source unchanged. This is a custody guard, not vanilla repair:
local 26.2 bytecode defaults a missing Slot to 0, skips out-of-range entries and
lets later repeated slots overwrite earlier stacks (ItemStackWithSlot.CODEC,
Inventory.load, PlayerEnderChestContainer.fromSlots). Valid inventory, equipment
and EnderItems survive import, native hydration, checkpoint and reopen; coverage
is `mc-world-conversion::player_slot_import_tests`.

Native WAL admission additionally requires exactly 41 player inventory slots
before applying a player post-image; a short or long vector refuses the whole
recovered world rather than reaching the join path's empty default. Nonempty
player item compounds without a string `id` refuse import and checkpoint
recovery, including equipment, cursor and crafting-grid custody. An empty
equipment/cursor/grid compound remains a legitimate empty slot.
Vanilla's 26.1.2, 26.2 and 26.3 inventory and ender-chest writers encode a Slot
for every saved entry, including slot 0; the 26.2/26.3 codec's optional read
default does not omit the field on write. Recovery coverage is
`mc-core::native_checkpoint::tests::player_custody`.

Repeated projected player custody names refuse before first-match decoding or
removal from the opaque image. The shared check also runs on WAL player opaque
bytes before replay and before player serialization; otherwise a WAL post-image
could bypass the native NBT codec's existing duplicate-name refusal and lose a
second cursor/crafting-grid payload on export or checkpoint. Native checkpoint
encoding and raw checkpoint recovery retain their existing recursive NBT
duplicate-name guard; no repair or weakened native admission is introduced.

Backups retain database custody throughout the copy. The native redb backend
and hot-backup reader share the exact lease-holding file handle (not a reopened
path or duplicated Windows handle). Positioned reads have independent cursors;
the native writer serializes the copy between committed jobs. The joined native
writer owns the backup store; callers send only a destination and reply channel.
A caller paused before admission must not retain a database owner past shutdown:
both writer joins finish database destruction before the world lease is released.
Offline backup/restore likewise read through their database lease handle. No
unlock/copy/relock interval is permitted. Windows directory durability uses the
existing platform directory-sync helper, not an ordinary `File::open` on a directory.
Conversion syncs only its output staging files, opened with write access but
without creation or truncation: Windows
[FlushFileBuffers requires GENERIC_WRITE](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers).

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
