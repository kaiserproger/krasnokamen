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

`random_tick_speed` uses the vanilla nonnegative `i32` bounds. The native random
tick owner draws the requested count per eligible active section; checkpoints
and WAL retain the exact value without an independent recovery cap. Runtime cost
scales with that count and the eligible sections, not a fixed per-tick budget.

Client game age follows simulated ticks, not journal frame identifiers.
[State ownership](../architecture/state-ownership.md#tick-and-wire-clocks) defines the
freeze/step projection and how recovery restores that age; daylight time remains
independent.

Spectator cameras are transient native bindings, published with the immutable
world snapshot. `/spectate` and battle named-follow use that same authority;
the native tick follows the target position for residency and ignores viewer
movement while bound. Binding changes synchronize the viewer once, not on every
follow tick. Death/disconnect releases the current camera; only battle's named
intent retries a returning target or a respawned viewer on the same connection.
Explicit stop, sneak, viewer disconnect or leaving spectator mode clears that
intent. Entity-ID camera bindings are transient; battle history persists only
named follow intent and resolves it through this same camera authority.
Restored names remain pending while their viewers are offline; a real Leave
input cancels intent before removing the connection. Re-arm completion is
recorded only after equipment replacement, not when an identity is observed or
a placement is attempted. Refused re-arm retries on the same connection;
successful respawn equipment prevents an extra reconnect re-arm.
The edge binds only a paired target, with camera release, removal, pairing and
rebind ordered on the entity world FIFO; credit loss retains delivery debt.
Shared command resolution finds named/UUID targets across worlds and transfers
the viewer through existing dimension custody before publishing a binding.
When a bound target changes dimensions, its viewers move with it and then
release, rather than retaining a dimension-local entity reference. Explicit
player teleport also releases the player's camera; refused teleports do not.
The effects appearance owner projects spectator invisibility and suppresses
effect particles without deleting the player's potion state.
The snapshot shares the existing global roster allocation for current player
modes. Entity interest uses that roster and camera ownership for vanilla player
tracking eligibility, not a second mode store: ordinary viewers never receive
spectator bodies; spectator viewers receive only self-camera player bodies.
Retained interest revisits player rows when either immutable input changes,
including already paired and previously hidden players, without scanning mobs.

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
reset moves a viewer away from that tick's explosion projection, only that
recipient's stale old-world impulse is discarded. The connection stays open;
other recipients and new-world impulses are unchanged.
Before queuing the reliable reset, the pump advances that recipient's world-FIFO
generation. The writer discards queued older-generation frames, including
best-effort explosion visuals from an earlier tick; destination frames retain
the new generation. Already-taken bytes precede the reset in the single writer's
stream. This fence does not clear another recipient's queue.
An old-world explosion must never be projected into the new world. This is accepted delivery policy,
not a claim that vanilla resets motion: 26.3 `ServerPlayer.teleport` sends
respawn keep-data flags 3, and `ClientPacketListener.handleRespawn` copies old
delta movement when bit 2 is set. Skipped publications still close only their
affected recipients as above.
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

Staged battles are native dimension-local gameplay owners, not disposable AI
caches: phase and battle-local clock, regiment/member identity and rank history,
orders/targets, morale/rout/action/weapon timers, live holds and spectator
bindings share their entities' checkpoint transaction and tick-WAL frame.
Changed battle-body poses are WAL-durable even within a chunk, without changing
the existing age/chunk-clock anchor. The WAL journals a battle's whole image
only as its first record after each checkpoint capture or recovery and
otherwise patches its changed parts; replay refuses a patch without that full
image. The fighter fight-loop clock changed the version-1 image schema: a battle
image written by an older binary after its first battle tick refuses to open
(fail closed, no migration). Recovery rebuilds only derived execution
columns and connection state. Running battles resume automatically; downtime
does not advance combat. Removed roster/execution IDs remain historical
tombstones, while a missing live member or an inconsistent/unsupported image
refuses startup, including offline recovery. Tags/tickets alone never recreate
lost formations. Reset publishes an empty owner image in the same frame as unit
and ticket removal. See the [battle contract](../architecture/persistence.md#battle-decision-history).
Native death tombstones take authority immediately: the short-lived presentation
corpse never contributes compact persisted poses after its live body is gone.

The owner-approved native founding expedition is an optional, default-absent
`SettlementRecord.founding` field containing only its destination and household
identity. Paid construction stages remain derived from the canonical structure
ledger. The reservation is admitted by the settlement registry and fences both
construction rectangles and the civil radius that either daughter can publish.
Generic building certification remains strictly inside the current civil radius;
an expedition cannot lend its reservation to parent certificates. Handover consumes
the reservation and publishes the new daughter, its certificates, the same living
household and inventories, and committed ledger ownership in one native tick/WAL
boundary. Canceled payment/construction history is retained, not transferred as
live provenance. There is no second persistence owner, spawned replacement family,
or visual-only daughter.

Older native checkpoints and WAL rows without `founding` open with no expedition.
Once this field has been written, binaries predating it are **not downgrade-safe**:
restore a pre-upgrade native checkpoint and its matching WAL with the old binary,
or continue with the compatible binary. Do not strip the field or rewrite the
save to make an older reader accept it.

### Durability failures and publication order

A failed WAL `write`, `sync_all`, `rename` or journal-directory `sync_all` is
fatal: the journal owner acknowledges nothing, closes its queue and the tick
owner halts. There is no retry, because the batch sequence cannot be repaired
after a hole. Publication order stays WAL file fsync, rename, journal directory
fsync (within `journal_fsync_every_batches`, default 10), checkpoint gate,
`Durability::Immediate` commit, then prune, so a committed checkpoint never
outruns its WAL. The journal directory's own entry is fsynced when the writer
opens it. Close errors are not observable through Rust's `File` drop, so
durability rests on the explicit fsync before the rename, not on close; a
deferred writeback error surfaces on the next fsync of that file or directory.
Removing covered journal batches is best effort: a refused removal is warned and
counted (dashboard `/metrics` and `/stats`) rather than fatal, because batches at
or below the committed checkpoint are replay-inert and only the WAL grows, while
an unwritable journal directory still halts the world on the next append.
Unpublished `batch-*.bin.tmp` leftovers of a failed or interrupted append are
removed by the next journal open under the exclusive world lease. Policy and
drills: [persistence](../architecture/persistence.md).

### Immutable datapack worldgen inputs

The startup boundary folds checkpoint and contiguous WAL metadata before it
constructs any dimension generator, searches for spawn, or materializes a replay
chunk. The recovered Overworld datapack selection is the world-global authority.
Fresh worlds discover directory and ZIP packs before constructing generators.
Invalid selected resources refuse admission; a new pack that cannot load is
skipped with a named warning and stays unselected, so it is no generation input;
missing packs are de-selected.

Pack worldgen resources are binary, not UTF-8 by default. Each selected pack
applies its filters to lower layers, then adds its own resources; builtin vanilla
occupies its actual selected position. Biome tags merge in order, honor `replace`,
and resolve required/optional references across namespaces. Worldgen JSON/NBT
and cache reports are staged as a private bounded copy; neither shared vanilla
files nor hard links are modified. Generators eagerly compile owned immutable
values. The staging directory is removed after construction, including refusal;
the reserved world-owned `.krasnokamen-worldgen` directory is disposable after a
crash only with its fsynced sibling `.krasnokamen-worldgen.owner` marker (or when
empty). The sibling marker is never deleted with the disposable tree, so a crash
during cleanup cannot erase ownership proof. Unowned nonempty paths, unexpected
top-level entries and symlinks refuse startup.

`level.dat/Data/KrasnokamenWorldgenInputs` records schema version 1, the canonical
lowercase SHA-256 of effective worldgen inputs/reports, source hashes and
historical structure ownership. Unknown/missing/duplicate struct fields or map
keys, unsupported schema, invalid hashes/ownership and oversized metadata refuse
adoption before any identity or epoch mutation. File pack IDs share selection's
bounded direct-directory-entry predicate. Removed-pack ownership remains valid
diagnostic history. The normal checkpoint level-metadata path owns it (there is
no WAL image of it), not a separate startup transaction.

A valid startup change logs old/new identities and changed packs/cache, then
continues: stored chunk baselines remain unchanged; new chunks use selected
inputs. Changed inputs mint a terrain epoch. Fresh worlds record identity in
their first checkpoint (including offline pregeneration). Existing worlds
without it adopt in memory and persist it at their next normal positive-tick
checkpoint; a crash before that repeats adoption, so a WAL-only missing base
after an input change refuses until the previous inputs return. Vanilla-only
worlds require no packs or staged copy. Native seed/revision identity validation
is unchanged.

Every generated native admission image, including vanilla-only generation, must
become a durable native baseline before dependent WAL can publish. The tick
queues every required-baseline job before its append; there is no separate
deferred generated-chunk save. The journal owner waits for the sole native
writer's Immediate fill-generated commit (behind any checkpoint commit already
queued there), so a tick that generated chunks puts one native commit per batch
in front of its append; failure closes admission and withholds the append. The
tick performs no new synchronous DB write or durability await. This does not
strengthen gameplay publication: a queued/unflushed tail can still roll back
consistently, and no per-tick fsync is required.

Admission refuses and records a load error if the bounded required-baseline
buffer (1024 per dimension) is full; checkpoint cannot discard that chunk's
unmaterialized edits. The refusal is sticky until restart, and a startup replay
needing more baselines than that refuses on every start: a known limit.
Fill inserts only absent, uncovered slots. A first dependent native snapshot
installs its original base in the same transaction. Covered native state with
no physical baseline refuses reads, fallback, fill and checkpoint instead of
blessing current generation as historical ancestry. WAL-only missing bases may
be reproduced only from matching validated saved inputs, or the fixed builtin
legacy generator with no external inputs. Unknown/changed inputs refuse before
replay materialization or persistent mutation with chunk and source context.

The offline export consumer uses that same identity authority and ancestry fence
before reconstructing a WAL-only chunk, then builds the serving generators from
the admitted effective inputs for each dimension. In particular, selected Nether
sets must not disappear through a config-only fallback. Input staging belongs to
the unpublished destination; neither identity adoption nor temporary input files
modify the native source. This keeps genuine matching-input WAL reconstruction,
not a blanket prohibition on export.

Baseline jobs commit no gameplay changes and advance no checkpoint tick. A crash
before WAL can leave an unused baseline; retained WAL folds edits over the
original base after pack addition/change/removal. SIGKILL regressions cover all
three transitions, repeated recovery, then checkpoint/prune and another recovery.

Generators do not hot-swap. Enable or disable that changes effective worldgen
inputs, and a reload whose selected packs' worldgen changed, refuse atomically
and retain the old packs, selection and registry; a new pack that would change
worldgen is skipped by a reload and offered again at the next start. A new pack
a start cannot prepare, validate or admit past the replay baseline fence is
skipped and named. Gameplay-only changes and identical worldgen reloads remain
valid. Unsupported worldgen domains/types refuse the whole pack, naming every
such resource, through the one shared
`mc-data`/`mc-worldgen` supported-type authority. This is not general modpack
compatibility. Authored biome convention defaults are the lowest vanilla layer;
pack filters, addition and replacement apply normally.
Custom Moog `msl_pieces_spawn_counts` and `msl_pieces_spawn_counts_additions`
resources refuse admission until the generator implements their required-piece
and count-cap semantics; they are never ignored as unrelated datapack content.

When a stored chunk is loaded after removing a pack, unavailable structure starts
remain untouched in its native data, including through WAL edits and checkpoints.
They are inactive for generation/locate. Chunk loading warns once per missing ID,
using persisted pack ownership where known; re-adding the pack reactivates them.
There is no startup world scan or special native metadata transaction.

Retained player and entity NBT is validated and tag-set-normalized at the shared
recovery boundary before dimension copies, live hydration or offline
export. A valid root followed by trailing bytes and repeated `Tags` keys refuse
the entire image; rewriting a valid prefix is not corruption recovery.

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

WAL player opaque NBT may not contain any root field in `player_owned_field`,
even once or with an empty payload: these fields belong to the typed player
post-image and serialization removes them from opaque data. Admission refuses
the whole recovery before applying the batch, naming player UUID and field,
rather than accept a second item authority that export would silently discard.
Current capture strips projected fields on decode; its retained mutations write
only attributes, EnderItems, Tags and unowned extension data, which remain valid.
The same refusal applies to `recipeBook.recipes`, whose authority is the typed
unlocked-recipe set and whose export projection replaces that nested list.
Other recipeBook fields remain opaque and are preserved; neither empty nor
nonempty accepted recipe lists are repaired or silently replaced.
Regression: `mc-world-conversion::wal_owned_player_fields_tests`.

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
