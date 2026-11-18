# Native persistence and recovery

The serving authority is `<world>/native/state.redb` and the tick WAL in
`<world>/journal/`. One checkpoint covers all dimensions, players, entities,
world and plugin domains. Anvil is offline import/export, never serving
fallback or dual-write. See [ADR 0013](../decisions/0013-core-model-v2.md) and
[operations](../OPERATING.md).

## WAL

`crates/mc-core/src/journal.rs` writes `batch-<lo>-<hi>.bin` containing whole
ticks. Its little-endian frame is `[magic u32][version u32][tick_lo u64]
[tick_hi u64][length u32][crc32 u32][payload]`; the payload is JSON lines of
absolute post-images and tombstones. The writer writes
`batch-<lo>-<hi>.bin.tmp`, fsyncs that file, renames it to publish the batch, and
fsyncs the journal directory once per `CoreConfig.journal_fsync_every_batches`
published batches (default 10, `[data] journal_fsync_every_batches`); a
checkpoint gate and shutdown flush it sooner. Until that directory fsync a host
power loss can drop the published names; the process-crash window is separate
— see [the acknowledgement window](#write-errors-and-publication-order). Readers ignore
unpublished `.tmp` files, and the next `JournalWriter::open` removes them
(`open_removes_unpublished_batch_leftovers`): the exclusive world lease
guarantees no other writer owns one, pruning never sweeps them because a live
append writes its temporary file on another thread, and a leftover the sweep
cannot remove is warned about with its path and reason rather than silently
kept.
Entries cover changed state, not a complete copy every tick. A battle image
journals only its changed parts after its first full record in the tail
([battle decision history](#battle-decision-history)). Player state and
meaningful entity changes are journaled; ordinary non-projectile mob motion and
idle-despawn-counter changes alone are checkpoint-durable only. An entity
crossing chunks emits a full image. Do not claim exact per-tick mob motion
recovery.
A body whose appearance changed (a battle shield is one metadata byte) journals
`JournalEntry::EntityAppearance` instead of the whole image: the fields of the
appearance image that differ, plus the health and effects scalars the image
would have set. A projectile flight tick journals `JournalEntry::ProjectileFlight`:
the authoritative pose and age, the landing transition, and the whole retained
`ProjectileState` only when a payload, ledger or custody field changed
(`journal_touched.rs` `compact_entry`). Both are deltas of the last full `Entity`
image of the same WAL tail: the first record per body after a checkpoint capture
or a recovery is always a full image, because a checkpoint stores neither the
WAL's transient appearance (living flags, variant metadata) nor a replayed
projectile state. Replay proves the base per id and dimension
(`Replay::tail_images`, dropped by a removal) rather than trusting a record the
checkpoint populated; a compact record without that proof refuses recovery
(`journal::validate_replay`, `entity_fault`). A compact record lands the clock
projection a full image would have baked in — the state image, the villager
clocks and the lifetime age advance to its tick — applies its delta first (the
projection reads the record's `baby`), and then moves the record's clock anchor.
Full images remain for spawns, deaths, chunk crossings, dropped items, a changed
entity identity, a changed pickup delay or villager custody, and a gained or lost
projectile. Fields that only a full image refreshes stay checkpoint-durable
between records by contract: `EntityStateImage::no_action_time`, the idle/despawn
counter, and the position of a body whose motion is not journaled. "Identical"
in this document means identical in every field replay projects, not that a
compact record rewrites those checkpoint-only fields.
Dropped-item pose changes use compact native `ItemMotion` images, including items
whose lifetime or pickup clock is infinite. Resting items emit no pose image.

Block-log records in `JournalEntry::BlockLog` and the native `block_log` table
carry the committing server tick. Records from before tick capture have an
unknown tick, not a reconstructed value; tick-based checks must reject them.
After replay, block-log WAL deltas remain queued for native-table upsert before
the next checkpoint can prune their batches, including rollback marks.

Journal admission uses a 128-item FIFO and a 256 MiB estimated in-flight
append budget; an oversized batch can enter alone. A full queue blocks the
tick, not discard or reorder accepted work. A failed writer halts the core.
Append acknowledgement is not a promise that the directory has been fsynced;
a crash can consistently roll back an unflushed tail, even after a reply.

The tick→edge reliable handoff (presentation `PlayerChange`s) is bounded too,
because the tick must never wait for the network layer: `[data]
outbox_queue_bytes` (256 MiB by default) caps the queued bytes, the tick charges
each batch before pushing and the edge releases it as it drains, and one
oversized batch is admitted alone into an empty queue. Two documented exceptions
exceed the cap: that oversized batch, and the halt's own terminal `Disconnect`
batch. A push that does not fit, or a non-empty queue whose drain has consumed
nothing for 10 s, halts the core exactly like a failed journal append: one error
naming the cause, an explicit `Disconnect` batch for every live connection, no
publish and no checkpoint. The process exits after the journal owner's final
flush even when the edge pump itself is wedged: `serve` watches the tick owner's
own closing signal, waits up to 5 s for connections to close (a healthy pump's
chance to route the batch), then kicks every remaining connection with the reason
the tick owner recorded (`Core::halt_reason`) and exits (`shutdown_background`, so
a worker parked in a synchronous section cannot keep it alive). The reason is
best effort: a full connection queue or a kick timed out behind a registry lock
the wedged pump holds leaves none, and the socket closes with the process. Those
bounds are runtime timers; the edge's `edge-halt-watchdog` OS thread
(`crates/mc-edge/src/server/halt_watchdog.rs`) ends the process with status 1
when `serve` has not returned 60 s after a halt (every runtime worker blocked),
after joining the core's owners. The
tick's `Append` for that tick was already accepted
by the journal queue — the handoff is asynchronous and the tick does not wait for
the write — so recovery uses the published contiguous tail: everything before the
last complete batch, with nothing accepted reordered. The warning mark is 8 MiB:
one warning per crossing (re-armed below 4 MiB) and the queue peak, never a halt.
This halt is the fail-stop above, not a new commit boundary.

All generated native chunks, including vanilla-only chunks, may outlive their
selected inputs. Before enqueueing a tick's WAL append, the tick owner drains
immutable pre-edit admission images from every dimension into required-baseline
jobs. In FIFO order, the journal owner sends each to the sole native writer and
waits for its Immediate fill-generated commit before appending dependent WAL.
The tick does not await that commit or perform DB I/O, but on a tick that
generated chunks every later append waits behind one native commit per baseline
batch (and behind a checkpoint commit already on the native writer). Failure or a
lost acknowledgement withholds that append and closes journal admission. There is
no separate deferred generated-chunk save. Exhausting the bounded admission buffer
(1024 baselines per dimension) refuses the chunk and records a load error;
checkpointing cannot silently lose its baseline or prune its unmaterialized edits.
Known limit: the refusal is sticky until restart, and a startup replay that must
regenerate more than 1024 WAL-edited chunks without a stored baseline refuses on
every start (the queue lives in memory).

Baseline fill inserts only absent, uncovered slots, never overwriting imported
or checkpointed chunks. The first dependent native snapshot also installs its
original baseline atomically. Native metadata/overlays/watermarks without a
physical baseline refuse reads, procedural fallback, fill and checkpoint; the
current generator cannot prove historical ancestry. A WAL-only missing base may
be reproduced only when the validated saved input digest matches current inputs
(or the legacy world uses the fixed builtin generator without external inputs).
Otherwise recovery refuses before materialization or writes, naming the
dimension, position and input sources; restore the original inputs or baseline.

Offline Anvil export applies the same selected-input identity and replay-baseline
fence before procedural fallback. Its generators use the validated effective
pack image and the serving dimension constructors, including selected Nether
structure sets. A genuine WAL-only chunk remains exportable with matching saved
ancestry; unknown or changed inputs refuse instead of supplying today's terrain.
Disposable input staging lives under the unpublished export destination, never
in the read-only native source. Export does not persist an adopted identity.

Baseline jobs commit no gameplay post-images and advance no checkpoint tick.
This is not a stronger publication guarantee: replies still permit rollback of
a queued/unflushed tail, with no per-tick fsync or tick-thread durability await.
A crash between baseline commit and WAL can leave an unused baseline. Retained
WAL folds over the original base even after adding, changing or removing a pack.
SIGKILL regressions use real flushed WAL, repeated recovery, checkpoint/prune and
another recovery.

### Write errors and publication order

- A failed WAL `write`, `sync_all`, `rename` or journal-directory `sync_all` is
  fatal: the journal owner returns without acknowledging, closes its work queue
  and the tick owner halts (failed closed). No retry, because a hole in the
  batch sequence cannot be filled afterwards.
- The checkpoint gate and the pre-backup flush follow the same rule for I/O: a
  failed directory fsync or a failed WAL-tail verification stops the world
  (`GateError::Io`). A **coverage hole** is the one non-fatal gate refusal
  (`GateError::Hole`): that checkpoint is skipped, its ACK is negative, the
  appended batches stay durable and the world keeps serving. A backup *copy*
  error stays local to that backup job; only its pre-copy flush is fatal.
- Order is always batch file fsync, rename, journal directory fsync (within the
  window), WAL gate, native `Durability::Immediate` commit, then prune. A
  checkpoint therefore never claims state whose WAL is not durable, and the
  directory entry of a published batch becomes durable no later than the next
  gate.
- The journal directory's own entry is made durable when `JournalWriter::open`
  creates it, through the shared `atomic_file::create_dir_all_durable` helper
  (which persists every entry it created, including one left unflushed by an
  interrupted first start), so no batch rename can claim durability inside a
  directory a later recovery cannot find.
- Close errors are not observable: Rust drops a `File` without reporting
  `close(2)`. Durability rests on the explicit fsync before the rename, never on
  drop, and nothing here re-checks a closed file: the directory fsync that
  follows a rename publishes the *name*, it is not a second look at that file's
  writeback errors.
- **Acknowledgement window.** A tick reaches clients once its batch is queued,
  before the batch is published (written, fsynced, renamed). Two windows follow.
  Any crash, SIGKILL included, can lose every batch not yet published: up to
  128 queued batches or 256 MiB of post-images (about 6.4 s at 20 TPS, fewer
  ticks when batches are large) and the batch being written if its rename has
  not happened, so an empty queue does not close it. A host power loss can also
  drop up to `journal_fsync_every_batches` published batches whose names the
  directory fsync has not yet made durable, and a lost name before later ones
  leaves a hole in the tail. Setting the cadence to 1 closes only this second
  window.
- A failed prune is warned and counted, never fatal: batches at or below the
  committed checkpoint are ignored on replay, so only the WAL grows. One line
  per failed attempt names the directory, failure count, retained bytes and the
  first error (`prune_reports_and_counts_a_refused_removal`); the counters
  `krasnokamen_journal_prune_failures_total` and
  `krasnokamen_journal_prune_retained_bytes` are on `/metrics` and under
  `journal` in `/stats`. A directory that cannot be written at all still halts
  the world on the next append.

### Recovery evidence and clocks

Network acknowledgement, native tick, game time and durable WAL coverage are
different observations. A held/frozen game clock need not advance with journal
frames; unloaded or unadmitted entities need not age for every elapsed game tick.
Establish the checkpoint/envelope and replay-clock mapping before comparing a
multi-tick census. An asynchronously published dashboard/pump tick is not an
upper bound on the census interval. See the clock images in
[`journal.rs`](../../crates/mc-core/src/journal.rs) and their
[`replay`](../../crates/mc-core/src/journal/replay.rs).

A process SIGKILL exercises process recovery, not arbitrary power-loss or storage
hardware durability. The [drills below](#hazards-and-defense-layers) state their
fault model; [crash-census evidence](../QA_AGENT_PROTOCOL.md#crash-census-evidence)
keeps lifecycle uncertainty separate from proved loss or conservation.

## Checkpoints

The default capture interval is 1,200 ticks (`CoreConfig.checkpoint_every_ticks`).
The tick freezes owned images; only one checkpoint is in flight. Before opening
checkpoint writes the writer gates and fsyncs WAL coverage through the captured
tick. A single immediate redb transaction commits all dimensions and participating
domains at that tick. Block-log rows are the exception: their own immediate
transaction upserts the queued rows and drops rows below the retention floor
before that commit, so a crash between the two leaves the table ahead of the
manifest while the WAL that replays the same rows idempotently is still durable.
Only after durable commit may it publish the committed
tick, acknowledge captured revisions and prune wholly covered WAL batches.
A pre-commit failure publishes and prunes nothing; a post-commit lost ACK or
failed prune leaves a recoverable checkpoint and possibly redundant WAL. Dirty
captured chunk revisions remain pinned until the matching acknowledgement.
Offline `world prune` therefore refuses a store whose last shutdown left a
published tail above the manifest tick
(`the last shutdown did not checkpoint the published journal tail`) instead of
judging chunks only the WAL changed; the journal is kept and replays on the next
start, and a start and stop whose final checkpoint commits clears the refusal.
Explicit saves queue behind the current checkpoint. Shutdown requests a final
checkpoint. See [ADR 0020](../decisions/0020-entity-checkpoints-at-one-million.md)
for the entity checkpoint design.

Disk-full checkpoint drills wrap the real lease-holding native file backend.
The injected writer and `NativeStore` retain the same `Arc<File>`, as the
production database and its backup reader do; failure injection never reopens
the leased database through a second handle.

Container WAL post-images can restore live custody without loading their chunk.
Before freezing a checkpoint's container snapshot, materialize their dirty chunks
and hydrate their complete persisted inventories. Clean evicted chunks stay unloaded.
Hydration fills only vacant live entries, so newer WAL slots (including an emptied
inventory) retain priority.
Do this in every dimension before projecting chest/furnace slots and the
hopper/brewing owners' opaque records. Otherwise a partial live snapshot can
erase untouched neighbours when typed maps are rebuilt. A refused load or
hydration holds the entire checkpoint and its WAL; it never commits a partial
container image. Behavioural coverage:
`container_transfer_replay_holds_unloadable_chunk_and_wal` and
`container_transfer_replay_materializes_full_healthy_chunk`.
The matrix includes decorated pots, chiseled bookshelves and lecterns; their
opaque WAL images require a resident chunk and refuse recovery if it cannot load.
Campfire items, vault rewards and trial-spawner pending items instead use
opaque-only custody and are covered by the unreadable-chunk recovery refusal.
Chiseled bookshelves hydrate through their six-slot owner only, not the
three-slot wooden-shelf decoder; a populated sixth slot guards against loss.

The custody hole combined lazy container replay, a dirty-chunk-only capture,
and pruning by the whole-world fence: an omitted chunk's WAL was no longer a
recovery source after the fence advanced. Dirtying the replay alone was not
enough: materialization after the slot snapshot could clear untouched typed
inventories, while hopper and brewing projection depended on their hydrated
owners. Transfer-based tests cover both defenses together, including another
restart after the WAL is pruned. The separate startup guard for block/opaque
replay into an unreadable chunk remains fail-closed; a dropped write into a
refused chunk still creates no unsaved custody.

## Startup and recovery

1. Acquire the exclusive world-directory lease. Normal startup waits up to 30
   seconds for a holder to exit, then refuses. Offline maintenance and backup
   also require exclusive custody; never copy a live database as an offline backup.
2. Validate generation identity, dimension descriptors, checkpoint and required
   records before admitting the world. Missing, malformed, inconsistent or
   unsupported required state fails closed. A missing database alongside old
   world history is not an empty world: use explicit Anvil import as directed.
3. Start from the committed checkpoint; ignore WAL wholly at or below it.
   Decode and fold the contiguous published tail starting at checkpoint tick
   plus one, in tick order across all domains. Replay uses post-images and
   tombstones. A gap, invalid identity, truncated header or payload, checksum
   mismatch, trailing bytes or invalid JSON refuses startup, including in the
   final published batch. Recovery does not repair or silently shorten the log.
4. Restore world-global Overworld datapack selection from the folded checkpoint
   and WAL metadata before constructing any generator. Discover directory/ZIP
   packs, preserve binary NBT, apply ordered vanilla/pack layers and filters,
   merge biome tags and prepare the bounded private image. Invalid selected
   resources refuse startup; a new pack that cannot load, or whose worldgen
   inputs cannot be prepared, validated or admitted by the replay baseline fence
   (steps 5-6), is skipped with a named warning and stays unselected; missing
   packs are de-selected with a warning. Shared caches are never modified.
5. Validate `level.dat/Data/KrasnokamenWorldgenInputs` from the folded native
   checkpoint's level metadata (only checkpoints write it) before adopting current
   inputs. Schema version 1 is required; unknown/missing/duplicate struct fields
   or source/owner map keys, unsupported schema, malformed canonical lowercase
   SHA-256 digests, invalid source/structure ownership or an oversized record
   refuse startup without changing identity/epoch state. File pack IDs use the
   same bounded direct-entry predicate as selection (including rejection of
   backslashes and NUL). Historical owners may name removed packs.
   A well-formed changed identity is diagnostic, not a permanent pack lock:
   log old/new SHA and changed sources, mint a terrain epoch, retain stored
   baselines and use the selected generator only for new chunks.
6. Validate selected registries/templates and compile immutable owned generator
   values before spawn search or replay chunk materialization. Dispose of the
   image after compilation or refusal. Apply absolute replay edits to native
   baselines, restore remaining domains, and only then start native serving.
   Fresh worlds persist identity in their
   first checkpoint, including offline pregeneration. Existing worlds without
   identity adopt in memory and persist it at the next normal positive-tick
   checkpoint; a crash before that repeats adoption. Because only checkpoints
   write the identity, a crash after an input change and before that checkpoint,
   with a WAL edit on a chunk that has no stored baseline, refuses the next start
   even with the new inputs unchanged; restoring the previous inputs (or the
   baseline) opens it. No extra startup transaction or invented simulation tick
   publishes this metadata. The required-baseline-before-WAL fence is independent
   of identity adoption. Hot enable/disable changing worldgen, and a reload whose
   selected packs' worldgen changed, refuse atomically; a new worldgen pack found
   by a reload is skipped until restart; gameplay-only or effectively identical
   reloads remain valid. No Anvil files or old JSON manifests are a serving
   recovery source.

Gamerule images retain canonical names and exact accepted values across checkpoint
and WAL replay. In particular, `random_tick_speed` keeps the vanilla nonnegative
`i32` range; recovery does not impose a lower operational cap or rewrite the value.

Scoreboard `Tags` retain the NBT string-list format but normalize duplicate
members on checkpoint/WAL recovery, preserving first-occurrence order. The
shared recovery boundary validates both entity and player retained fields
before live hydration, offline export or checkpoint capture. It consumes the
whole opaque NBT image before rewriting it: trailing bytes refuse recovery,
rather than being discarded. More than one `Tags` key in either retained
compound is corruption, not a choice between two lists.
The native checkpoint codec already refuses repeated compound names.
`duplicate_tags_checkpoint_and_wal_recovery_is_idempotent` reopens the actual
checkpoint and WAL twice; player WAL tests cover normalization and differing-key
refusal through offline projection and live recovery. The conversion regression
`export_refuses_ambiguous_player_tags_from_real_wal_without_publication` also
requires unchanged source bytes and no published destination.

Mob targets are runtime-only. Zombie-family aggressive-arms appearance is
present in WAL entity post-images but is not stored in checkpoint entity
records. Recovery preserves a WAL pose until the first admitted hostile
evaluation reconciles it with the live target; a changed appearance is
journaled as usual. A checkpoint-only restart instead reconstructs the default
pose and the hostile tick derives it again from live targeting.

An item entity with `Age: -32768` (vanilla `INFINITE_LIFETIME`) never ages or
despawns, and one with `PickupDelay: 32767` never becomes pickable or mergeable.
The native lifetime clock is unsigned, so it holds the never value as `u32::MAX`:
checkpoint and Anvil records carry the vanilla short -32768 (an import of a vanilla
world maps it back). Native checkpoint NBT also carries
`krasnokamen:LifetimeAge=4294967295`; WAL entity images and `ItemMotion` records
carry `u32::MAX` in their typed age field. Replay never advances that age.

Cow, pig, chicken and zombie-nautilus variants (from `/summon ... {variant:...}`)
are persisted. The checkpoint stores the vanilla string `variant` field of the
entity record; the WAL stores a typed appearance value at the species' metadata
index. A corrupt checkpoint `variant` (not a string, or not an identifier)
refuses hydration, and a WAL image whose variant value has another species' kind,
the wrong index or an id outside the registry refuses recovery
(`validate_entity`): only Krasnokamen writes WAL images. A well-formed identifier
that names no entry of the species' registry (a datapack variant) is ignored with
a `unknown variant ignored` warning naming the entity, species and id, and the
default is kept, as vanilla does; records imported by an older Krasnokamen, which did
not drop such values, still hold them and must open. An absent field is the
vanilla default. Naturally spawned animals carry no variant and use that default
(egg-hatched chickens keep the egg's variant); Krasnokamen does not yet choose
biome-dependent variants. Bred cow, pig and chicken calves take this parent's or
the partner's variant by a fair coin, as `getBreedOffspring` does (javap,
receipts/Q04CoreFix/javap). Anvil import also drops an unknown or malformed
`variant` before the native hydrate (the game ignores it and keeps the default).

On genuine first start, `Core::open` writes and fsyncs
`<world>/.krasnokamen-first-start`, creates `native/state.redb.new` and renames the
committed store into place. The marker is removed after the initial checkpoint
is durable. A remaining marker permits retrying interrupted first start; a store
without a checkpoint or marker fails closed. Required checkpoint player records
must decode and match their keys; offline player custody is retained rather than
silently discarded.

### Corpses

A killed living mob keeps its body for 20 ticks (vanilla `deathTime`) so
clients play the death tilt, but its death is the durable removal. A corpse is
a row on the dying clock (`EntityStore::is_dying`), not every `alive = false`
row. WAL images of the corpse carry `alive = false`, which replay folds as a
removal; a checkpoint writes no record for it and tombstones its UUID. A retired
deployed stand is removed at the commit tail and its UUID is likewise
tombstoned in the next checkpoint. Vanilla saves `DeathTime`; after a restart Krasnokamen shows no
remaining corpse, and the loot journaled at death exists once. Regressions:
`corpse_is_a_durable_removal_across_checkpoint_crash_and_reopen`,
`retracted_stand_checkpoint_keeps_the_returned_item_single_across_restart`;
[ADR 0020](../decisions/0020-entity-checkpoints-at-one-million.md).

### Battle allegiance

`Body.faction: u16` is native entity state; zero means unaffiliated.
`EntityStore::set_faction` marks WAL and incremental-checkpoint custody,
including a reset to zero. `EntityStateImage.faction` carries full/compact WAL
post-images; `krasnokamen:Faction` (`TAG_Int`, checked as 0–65535) is the
checkpoint projection. Both omit zero, leaving unaffiliated entity bytes unchanged.
Missing allegiance is zero; a wrong NBT type, out-of-range value or malformed WAL
value refuses decoding. Recovery and `/summon` use the same setter.
The key is visible to `/data` and NBT selectors while allegiance is nonzero;
no wire metadata or protocol change is needed. Allegiance travels with the body on transfer and
dense compaction.
The native checkpoint/WAL/restart regression is
`battle_faction_survives_native_checkpoint_wal_and_restart`; see ADR 0020's
allegiance extension. Persistence changes require Main's independent review.

Shield presentation is entity appearance, not a runtime action clock: the
living-flags byte in `appearance.extra` is present in WAL entity post-images but
not in checkpoint entity records (`core/capture.rs`
`appearance_from_vanilla_entity` restores no living flags). Recovery keeps a WAL
pose until the next admitted AI decision; a checkpoint-only restart starts with
the shield lowered, like the zombie-arms pose above. Persisted staged-battle
action history rebuilds the visible guard pose on the first resumed battle tick.
Staging raises shields before the initial spawn, so starting a staged battle
does not republish every shield bearer's appearance.

### Battle decision history

Dimension-local `battle/<dimension-id>` native global rows (version 1) and
dimension-enveloped `Battle` tick-WAL post-images own the staged/running/paused
phase and battle-local clock, regiment specifications/leaders/orders/sections,
stable entity-ID roster and rank/file assignments, morale/rout/charge clocks,
retained targets and escape plans, action/ammunition/weapon-swap history,
retaliation windows, live holds/kits and named spectator bindings.
They share the entity capture's WAL coverage gate and atomic native transaction.
A reset publishes an empty post-image, not an absent update.
Recovery installs a journal baseline, so even a reset on the first live tick
publishes the empty image before any new checkpoint.

The checkpoint row and a `Battle` record carry the whole image. Within one WAL
tail a changed tick journals `BattlePatch` instead: the image without its six
index-aligned lists (roster members, regiments, fighters, fighter IDs,
execution columns, removed IDs) when it changed, plus each changed list
element and list length as absolute values. Unchanged ticks journal nothing.
The first battle record of a dimension after every checkpoint capture and after
recovery is a whole `Battle` image, so a patch never builds on a checkpoint
row. Replay applies patches in one place (`battle::persistence::fold_patch`)
and refuses a patch without a `Battle` earlier in the tail, a missing list, a
length beyond capacity or an index past the length; an element left unset by a
longer length stays `null`, which decoding refuses. A WAL mixing records from
binaries before and after this format is unsupported. A fighter's fight-loop
clock (the `fighting` telemetry window) changes only when he enters or leaves
his fight loop, so a steady fight does not rewrite every fighter. That clock
changed the image schema while the row stays version 1: a world whose battle
row or WAL tail was written by an older binary after its first battle tick
refuses to open (fail closed). Before upgrading, run `battle reset` and then
`save-all flush` with the old binary (or reset the world). The harness gates
read the same chain (`battle_recovery.py` `fold_battle`, the same refusals).
A fugitive's guard hold is stamped once per flight, like the fight clock, and
resolved on the first pass that reads it, so a rout does not rewrite every
fleeing fighter each tick; recovery stays exact because the stamp and
`elapsed_ticks` share the image. An older binary opened mid-rout keeps such a
fighter in Guard until he is hit; run `battle reset` before a downgrade.
Regressions:
`battle_wal_journals_changed_parts_not_the_whole_image_each_tick`,
`battle_wal_patches_changed_parts_and_replays_the_live_image`,
`battle_wal_patch_refuses_without_a_full_image_or_with_bad_elements` and
`battle_wal_does_not_rejournal_a_routing_fighter_every_tick`.

Native entity custody remains sole authority for life, pose, equipment in hand
and mounts. Live battle bodies additionally journal changed intra-chunk poses through
`EntityMotion`, so replay cannot pair a new order/action with an older pose.
The compact pose does not move the native age/chunk-clock anchor; the existing
full crossing image still owns that anchor. Transient slots, position/faction
caches, grids, projections, work counters and connection IDs rebuild.

A native death image immediately tombstones the body. Its remaining 20-tick
presentation corpse can move, but no longer emits compact persisted poses;
replay still rejects every pose referencing an absent authoritative body.

Both WAL and checkpoint capture reconcile staging decisions with proven native
tombstones: dead pending/refused bodies no longer await ground. An absent
staged body is not proof of death and remains a recovery error. Pending
retaliation blows feed only the next running battle pass (a pause keeps them),
which answers a living attacker alone, so capture keeps a blow only by a living
native attacker or a held player, online or gone (the image carries his last
identity and a reconnect rebinds it). It drops an ordinary player, a dead
native attacker (a horse extra that left the columns, a mob killed in that
merge: a dying row has no record) and any other attacker without a row (a
creeper's blast removes it first). Replay's identity checks stay strict.

An image-encoding failure propagates through journal capture as a typed error.
The tick owner halts before queuing the WAL or publishing that tick, reliably
disconnects live players, and does not checkpoint its unjournaled state.

Removed roster IDs and execution-tail IDs are retained historical identities,
not permission to spawn replacements. A missing live roster member, mismatched
side/liveness, unknown target/action identity, invalid roster index, invalid
motion identity, malformed payload or unsupported version refuses online and
offline recovery. A dead roster member may lack its native body. Named live
roles retain only their last wire ID; recovery reserves those IDs and roster/
execution tombstones against allocator reuse, and a reconnect transfers that
role's retained targeting references to its new wire ID.
Named roles remain pending while a connection is auth-locked. Re-arm and
spectator binding run only after private player state has been hydrated; a
restored holder's kit comes only from the pieces in his durable player record,
never from a fresh copy of the saved hold's kit.

No downtime catch-up or tag-based formation reconstruction occurs. Running
state resumes automatically; paused state remains paused. `battle setup` reports
the existing battle. Native backups retain this authority; Anvil does not.
Production-tick regressions:
`battle_resume_wal_published_tick_excludes_inflight_decisions_and_motion`,
`battle_resume_abrupt_checkpoint_commit_recovers_whole_history`,
`battle_resume_encoding_failure_halts_before_wal_or_publication` and
`battle_resume_sigkill_during_dying_body_keeps_tombstone_authority`.
Capture-boundary SIGKILL regressions cover an ordinary unheld player's actual
blow and staged-body death before settlement, after real terrain refusal, and
after waiting settlement. Each compares recovered history/native poses and a
subsequent checkpoint/restart; running battles also land further native damage.

### Migration transit custody

`transit/<uuid>` native global rows (version 1, canonical lowercase hyphenated
UUID, the record's lowest member) and dimension-enveloped `Transit` tick-WAL
post-images (a `None` image deletes) own households and lone residents between
settlements ([ADR 0023](../decisions/0023-offscreen-settlement-simulation.md)).
A member is never both a world entity and a transit member: the departure tick
removes the bodies (WAL `RemoveEntity`, checkpoint entity tombstones) and
inserts the record; the arrival tick restores the same UUIDs and deletes the
record. The checkpoint drops `transit/` rows the WAL deleted in the same native
transaction. Recovery fails closed when a UUID is both an entity and a member,
a member of two records, or a record household is also a settlement's; the
offline census reports `transit_records` / `transit_members` and refuses the
same duplicates. Binaries without this domain refuse such a store.

Crash evidence reads the durable boundary itself: `krasnokamen world
custody-census --copy <disposable copy> --receipt <new file>` repairs the
copy, folds the checkpoint and the contiguous published WAL through offline
recovery's replay records (before any entity is restored), lists every villager
UUID and every transit record with its members, and fails when a member is
also an entity, is held by two records or its household is still a
settlement's. The `transit-crash` harness profile SIGKILLs a pinned release
server at the departure, in-transit and arrival cuts and checks that census,
the frozen recovery at the durable tick and the clean-stop `world census`.

## Quarantined chunks

A chunk whose stored record cannot be loaded (decode error, invalid block
entity record, or an undecodable runtime record found by a block-entity owner)
is quarantined: it is never loaded, never generated over and never rewritten,
so its stored record stays byte-identical.

A quarantined chunk that holds no unsaved state (refused when it was loaded,
never edited) does not block checkpointing: every other chunk checkpoints
normally and each checkpoint logs one error naming every quarantined chunk
with its dimension, coordinates, reason and counts. The chunk-local
projection of changed scheduled block and fluid ticks of such a chunk cannot be
written and is skipped (`scheduled projections skipped` in the log). The queues
themselves are not lost: the manifest carries the global scheduler images, and a
restart restores the same ticks (test
`scheduled_ticks_of_a_refused_chunk_survive_checkpoint_and_restart`). Debt and inhabited-time metadata of it are not written.

A quarantined chunk that holds unsaved state declines the whole checkpoint
instead, and the log error (`checkpoint declined`, repeated on every declined
checkpoint so a stuck server is visible) names the dimension, chunk and counts.
An early container-hydration refusal reports `container_records` (live container
records in that chunk); capture-time refusal reports the aggregate unsaved entries
and skipped scheduled projections. The
WAL is retained. Unsaved state means pending block writes, container records of
the chunk held by the live owners, edited sections, and the container edit or
removal mark (`block_entity_dirty`; a removed record leaves nothing but the
mark). Refusal never clears these. Skipping such a chunk would commit the rest
of the world (a player's inventory, a dropped item) without the chunk's edits,
so an item taken from or dropped by a container of that chunk could exist twice
after a restart; recovery ignores journal records at or before the checkpoint
tick, so keeping the WAL behind a committed checkpoint would not help.
A chunk refused at runtime while it holds containers therefore declines every
checkpoint until the server restarts, even with no edits, because its live
records count. After a restart it is refused at load, so it is skipped unless
the retained WAL replays edits into it, and then the declines continue; the
exception is a WAL block write or opaque block-entity record (campfire, hive,
brushable block) into a chunk refused at load: recovery needs that chunk
resident to replay it, so it stops with the chunk's load error and the world
does not open, nothing is checkpointed, and the WAL keeps the write until the
stored chunk is valid again (test
`opaque_replay_into_a_chunk_refused_at_load_fails_recovery_and_keeps_the_wal`). Krasnokamen
has no command that repairs or removes a quarantined chunk: the operator's way
out is an offline `restore` of the last backup (play since that backup is
lost); the stored record stays byte-identical for inspection. Importing an
Anvil chunk, and loading a native one, validates crafter, shelf, shulker box
and conduit records by the block that owns them, never by the record's own id.

The `container_transfer_replay_*` regressions exercise actual transfers for chest,
trapped chest, barrel, copper chest, furnace, smoker, blast furnace, hopper,
brewing stand, dispenser, dropper, crafter, shulker box, shelf, jukebox,
decorated pot, chiseled bookshelf and lectern, with an untouched chest in the same chunk.
Typed-only WAL tails hold a refused checkpoint
and materialize healthy unloaded chunks before capture. Owners that also emit an
opaque WAL image already require a resident chunk at recovery, so an unreadable
chunk refuses startup instead. Both routes retain full container/player contents
and total custody through repair, checkpoints and repeated restarts.

In RC5, a chiseled bookshelf with books confined to slots 1–3 could load
through both the ordinary three-slot shelf decoder and its six-slot owner.
Hydration applied the ordinary shelf first; interaction replaced that
wrong-length record with an empty six-slot record, losing its books. Books in
slots 4–6 instead made the three-slot decode reject the chunk. The storage
format did not change. The owner classification fix removes the overlapping
decoder; `chiseled_bookshelf_old_save_interaction_preserves_all_books` tests
an old stored record without a live or WAL substitute through interaction and
restart. Prior load and custody tests did not cover this combination; the
slot-6 refusal was a different failure, not proof against this data loss.

## Hazards and defense layers

Focused sources: `native_persistence_drill_tests.rs` and its
`native_checkpoint_pipeline_drill_tests.rs` sibling (`mc-core`), plus
`native/checkpoint_drill_tests.rs` (`mc-world`). Harness `test` filters are
`persistence_drills` and `native::checkpoint::drill_tests`, respectively.
The table describes assertions in the drills, **not their run status**; use
candidate-matched receipts for acceptance.

| Hazard | Defense layers | Drill assertions and scope |
|---|---|---|
| Take, insert or break splits item custody | Whole-tick post-images; one checkpoint transaction | The randomized model validates the selected operation, freezes complete named-token before/after owner-and-slot maps, then requires the appropriate map after recovery. Counts independently reject loss, changed identity and duplicate custody. |
| Refused request mutates items | Authoritative window validation | A wrong-window click must leave every named token's owner and slot unchanged, not merely its global count. Attempted operations, real mutations, actual append faults, faulted mutations and ordinary durable restarts have separate counters. |
| ENOSPC during a transfer-frame append | Failed append cannot supply WAL coverage; unpublished temporary files are ignored | Every byte boundary of a **real transfer frame** fails before publication. The production recovery fold over a pinned native checkpoint must retain the exact old placement. A complete published frame must yield the exact new placement. This is not a fresh Core process per byte. |
| Failed WAL writer keeps admitting work | Journal owner closes its queue without acknowledging the failed tick | A queued real transfer fails during append; neither its append ACK nor the queued checkpoint ACK/save reply may succeed. Native custody remains old and recovery must select the exact old placement. |
| Process stops at a WAL boundary | File fsync before rename; directory fsync for durable publication | Transfer subprocesses stop after create, write, file fsync, rename and directory fsync. Recovery requires the whole old or new owner/slot map. The unflushed rename is modeled both retained and lost. |
| ENOSPC in a native checkpoint backend write | Immediate redb transaction | The `mc-world` backend fails successive **write calls** until the first successful commit; complete old/new globals, player, entity, chunk and index records are checked. Its synthetic Score/Health fixture is not an item transfer or a core ACK/prune test. |
| Checkpoint gate or commit refusal loses a durable transfer | Journal queue owns WAL gate; native worker sends a negative ACK/save reply and does not prune on failure | A real take followed by an unpublished inverse transfer exercises a genuine coverage hole. A separate pre-commit failure follows a durably appended take. Tests inspect checkpoint-only player/chest slots, retained item-bearing WAL and recovered owners; both failure paths must refuse positive ACKs. The pre-commit hook is a generic injected error, not a physical ENOSPC in the core process. |
| Successful checkpoint prunes a newer transfer | Frozen capture; ACK and pruning after commit; pruning only through the committed tick | A take is captured, then a newer inverse transfer is append-ACKed before the native commit proceeds. An event-driven pre-commit probe requires no checkpoint ACK/save reply and no premature pruning. The committed checkpoint must contain the captured placement, send its matching positive ACK, remove covered WAL and retain/replay the newer inverse-transfer frame. |
| Process stops in the core checkpoint pipeline | WAL gate before native commit; ACK and pruning after commit | Item-transfer subprocesses stop at WAL-gated, pre-commit, native-committed, pre-ACK, ACK-sent and post-prune boundaries. Persisted stage receipts count only reached boundaries. Checkpoint-only custody must be entirely old or new; checkpoint plus retained WAL must recover the complete durable transfer. |
| Process stops inside a native transaction | One transaction across all participating records | Separate `mc-world` children exit after globals, chunks, entities, players and durable commit. These check storage atomicity, not the core queue or item-transfer semantics. |

The core drills run real `PlayerInput`s through `run_tick`; checkpoint jobs
then traverse the actual `journal_loop` queue, WAL gate and native writer
thread. They do not claim a live `Core::open` tick-thread/client gate.
Core recovery cases explicitly write a **new checkpoint**, prune its covered
WAL, drop the process state and recover again. Repeating `NativeStore::open`
in the transaction-only tests checks repeated database open, not that stronger
checkpoint/prune/recovery sequence.
The successful core case also applies the older capture's ACK to a live newer
inverse transfer and checkpoints that live state before restarting. Recovery
cannot hide a stale-ACK dirty-mark error by seeding fresh marks first.

These are bounded software-fault drills, not proof that storage hardware
honours fsync or survives arbitrary torn sectors. The WAL hook writes a prefix
and returns ENOSPC before publication. The native backend injects failures in
`write` only: `set_len` and `sync_data` delegate normally and their ENOSPC paths
are **not covered**. Native storage positives do not substitute for the core
pipeline receipts. Subprocess stops exit without database destructors; ordinary
model restarts drop the live state without a final checkpoint.

The randomized drill prepares a survival chest break with actual Start and idle
ticks appended through the same WAL writer before faulting the Finish tick.
The mining test helper's direct clock jump is unsuitable here: it leaves a
journal coverage gap, so append refuses before reaching the injected ENOSPC.

## Backup and restore

Offline `backup` and `restore` require a stopped, exclusively leased world;
restore verifies the backup inventory and checksums. The running console
`backup` is a different operation: the native writer makes a consistent copy
of the committed checkpoint plus its WAL, and it decodes and CRC-checks every
batch it copies, so a byte-identical copy of a damaged source is refused
instead of being published as a backup (Books R5 G3). A `journal/` directory
belongs to the store it was written with: batches carry no store identity, so a
foreign or mismatched tail replays into the wrong world without a refusal.
Never mix the two — `backup` and `restore` always move both, and restoring only
`native/state.redb` over an existing `journal/` is operator error, not a
supported path (Books R4-P3). Use the documented operator commands
and check their results; a filesystem copy of a live writer is not equivalent.
Anvil export is not a native backup: native-only plugin and settlement state
and block-log history are not represented. See [operations](../OPERATING.md)
and [migration](../MIGRATION.md) for procedures.
