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
absolute post-images and tombstones. The writer fsyncs a temporary file and
renames it to publish the batch. Unpublished temporary files are ignored.
Entries cover changed state, not a complete copy every tick. Player state and
meaningful entity changes are journaled; ordinary non-projectile mob motion and
idle-despawn-counter changes alone are checkpoint-durable only. An entity
crossing chunks emits a full image. Do not claim exact per-tick mob motion
recovery.
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
4. Only then start native serving. No Anvil files or old JSON manifests are a
   serving recovery source.

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
dense compaction. Restart restores allegiance, not runtime battle targets or timers.
The native checkpoint/WAL/restart regression is
`battle_faction_survives_native_checkpoint_wal_and_restart`; see ADR 0020's
allegiance extension. Persistence changes require Main's independent review.

Shield presentation is entity appearance, not a runtime action clock: the
living-flags byte in `appearance.extra` is present in WAL entity post-images but
not in checkpoint entity records (`core/capture.rs`
`appearance_from_vanilla_entity` restores no living flags). Recovery keeps a WAL
pose until the next admitted AI decision; a checkpoint-only restart starts with
the shield lowered, like the zombie-arms pose above. Guard/swing/stagger/rout
decisions remain runtime-only. Staging raises shields before the initial spawn,
so starting a staged battle does not republish every shield bearer's appearance.

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
of the committed checkpoint plus its WAL. Use the documented operator commands
and check their results; a filesystem copy of a live writer is not equivalent.
Anvil export is not a native backup: native-only plugin and settlement state
and block-log history are not represented. See [operations](../OPERATING.md)
and [migration](../MIGRATION.md) for procedures.
