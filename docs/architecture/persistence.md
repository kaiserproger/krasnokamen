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
domains at that tick. Only after durable commit may it publish the committed
tick, acknowledge captured revisions and prune wholly covered WAL batches.
A pre-commit failure publishes and prunes nothing; a post-commit lost ACK or
failed prune leaves a recoverable checkpoint and possibly redundant WAL. Dirty
captured chunk revisions remain pinned until the matching acknowledgement.
Explicit saves queue behind the current checkpoint. Shutdown requests a final
checkpoint. See [ADR 0020](../decisions/0020-entity-checkpoints-at-one-million.md)
for the entity checkpoint design.

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

Mob targets are runtime-only. Persisted zombie-family aggressive-arms metadata
is reconciled with the live target on the first admitted hostile evaluation
after restart; a changed appearance is journaled as usual.

On genuine first start, `Core::open` writes and fsyncs
`<world>/.solaris-first-start`, creates `native/state.redb.new` and renames the
committed store into place. The marker is removed after the initial checkpoint
is durable. A remaining marker permits retrying interrupted first start; a store
without a checkpoint or marker fails closed. Required checkpoint player records
must decode and match their keys; offline player custody is retained rather than
silently discarded.

## Backup and restore

Offline `backup` and `restore` require a stopped, exclusively leased world;
restore verifies the backup inventory and checksums. The running console
`backup` is a different operation: the native writer makes a consistent copy
of the committed checkpoint plus its WAL. Use the documented operator commands
and check their results; a filesystem copy of a live writer is not equivalent.
Anvil export is not a native backup: native-only plugin and settlement state
and block-log history are not represented. See [operations](../OPERATING.md)
and [migration](../MIGRATION.md) for procedures.
