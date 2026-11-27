# Native persistence and recovery

The serving authority is `<world>/native/state.redb` and the tick WAL in
`<world>/journal/`. One checkpoint covers all dimensions, players, entities,
world and plugin domains. Anvil is offline import/export, never serving
fallback or dual-write. See [ADR 0013](../decisions/0013-core-model-v2.md) and
[operations](../OPERATING.md).

### CoreUnload byte oracle

`tools/dev/bytecheck.sh BASE_TREE SLICE_TREE --out ABSOLUTE_RECEIPT_DIR`
compares the D65 deterministic fixture at matching ticks:

| Artifact | Comparison |
|---|---|
| Checkpoint | tick-1 native rows: version, capture tick, SHA-256, base64 payload |
| WAL | complete raw single-tick frames at ticks 2–3 |
| Recovery | live and offline `recover_offline → encode_checkpoint` at tick 3; fresh **base-world** copies, no tick thread |
| Export/censuses | every complete output file byte-for-byte (compression, headers, padding included), custody and crash censuses |

Requires local `data/vanilla`. Both captures use byte-verified copies of one
closed prepared store, preserving the WAL-v2 UUID without ignoring bytes.
`seed.json`/producer `input.json` record every seed file's size/SHA; absent or
changed immutable seed refuses cache reuse. Build receipts pin each producer
before discarding its private Cargo target, so only one scratch target is needed.

First run base/base in separate processes. Scratch controls reuse
`--base-run RECEIPT/base`; recorded base HEAD/source fingerprint must still match.
`--control X1|X2|X3` means respectively: swap `SettlementRecord.owner_name/anchor`,
swap `VillagerState.mining_progress/lumber_progress`, or swap only their NBT pushes.
X2 also changes `Transit.Record.members`' embedded villager in checkpoint/live
transit rows and Entity/Transit WAL. Classifier requires a complete 64-byte WAL-v3
header (length at offset 56), but compares the **whole frame**.
Red requires all producers to finish and the exact expected row/WAL/export
difference; compile/recovery/missing-artifact errors are not red controls.
Ordinary comparison refuses any byte difference; verdict records each artifact.

Real-image gate:
`python3 tools/dev/bytecheck_real.py BASE_TREE SLICE_TREE RETAINED_RUN --out ABSOLUTE_RECEIPT_DIR`.
Stopped input contains `world/`, `plugins/`, `server.toml`; `--world PATH` selects
a certified alternative world subdirectory, copying only that world/plugins/config.
Hash every source file, verify snapshot, repair redb using base pin's normal
`NativeStore::open/drop` **on the copy only**, record post-repair hashes and refuse
changes outside `native/state.redb`. Freeze repaired input read-only; operations
get fresh writable copies. Never open original as DB; recheck its hashes afterward.

Each tree's fingerprinted public-API probe compares all encoded global/player/
entity rows, complete exports, custody/crash censuses and ordinary census after
frozen startup/shutdown. Ordinary-census projection drops source paths/backing-file
CRCs only, retaining checkpoint tick/counts and the complete receipt.
Require settlements and a tail advancing checkpoint; `tail-records.json` states
actual domains, including replayed climate/game-clock state. A frozen image
does not imply active settlement/transit WAL; synthetic fixture provides that coverage.
Build manifests separately hash ignored vanilla path/content; an old pin's
current data hash is an observation, never retroactive build provenance.
`--base-run PREVIOUS_RECEIPT` verifies/reuses frozen input, base fingerprint,
pin/artifacts. `--census-refusal EXACT_ERROR` proves matching canonical refusal,
not successful ordinary census.

## WAL

`crates/mc-core/src/journal.rs` writes `batch-<lo>-<hi>.bin` containing whole
ticks. Its little-endian frame is `[magic u32][version u32][store id 16][branch
8][block palette 8][tick_lo u64][tick_hi u64][length u32][crc32 u32][payload]`;
the payload is JSON lines of absolute post-images and tombstones. Frame version
3 binds every batch to the store that wrote it, to the branch of that store and
to the block-state palette its numeric block ids mean (Books R4-P3, R5-G5,
`DIVERGED-BACKUP-COPIES-SHARE-STORE-IDENTITY`): the store id is the native
header's required random `store_uuid`, the branch is the one the
[checkpoint marker](#checkpoint-marker) records (the root branch for a world as
created or imported, a fresh random id for every restored copy), and the palette
is the first eight bytes of SHA-256 over the registry's `(state id, block,
properties)` mapping. Recovery opens the batch only when all three match the
opened store, its marker and the registry this build replays with; a foreign
journal directory, a copy of another world's tail, the tail of another copy of
the same backup or of the world a copy was restored from, or a tail written
under a changed palette is refused by name, never replayed with a warning. All
three fields are compared unconditionally: a zero id, branch or digest is a
value, not a wildcard, because the CRC covers only the payload and the header
can be rewritten. A frame of format 1 or 2 is refused too, with the way out in
the message: start the world once with the build that wrote it, stop it and
confirm its final checkpoint covers the last published batch (ADR 0013 keeps no
compatibility surface of our own; `docs/OPERATING.md` has the upgrade
procedures). A batch at or below a published checkpoint is never replayed, so no
journal file has to be removed or moved. The writer writes
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
Each dimension also checkpoints one coverage row (row kind 2): the lowest
sequence from which the log is complete. Eviction raises it past the evicted
record, and a restart with capture off reserves a sequence and moves it there.
The first tick after such a restart journals that floor in its block-log delta
(`coverage`), so the gap is durable before any unlogged edit; recovery takes the
highest floor of the row and the WAL tail. Restore never assigns a sequence below it.
Each settlement structure record
carries a required `admission` (capture off, or the log sequence at its survey),
and construction provenance outside village territory is judged against it.

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
selected inputs, so every generated admission image must become a durable
native baseline before dependent WAL can publish. Before enqueueing a tick's
WAL append, the tick owner drains immutable pre-edit admission images from
every dimension into required-baseline jobs; there is no separate deferred
generated-chunk save. The journal owner hands the sole native writer its
Immediate fill-generated commit (behind any checkpoint commit already queued
there) and then keeps draining the tick's queue into a hold of at most
`JOURNAL_HOLD_LIMIT` jobs instead of parking it: held work keeps its arrival
order, nothing held is published before the baseline is durable, and appends
keep the budget reservations they were admitted with. The wait is the writer's
own answer through the same work queue (`JournalWork::BaselineDurable`, tagged
with the handoff's serial), so no timer or polling interval exists; past the
hold limit the owner falls back to the job's own answer channel. A tick that
generated chunks therefore still puts one native commit per batch in front of
its append. Failure or a lost acknowledgement withholds that append and the held
work and closes journal admission. The tick itself performs no DB write or
durability await. Exhausting the bounded admission buffer (1024 baselines per
dimension) refuses the chunk and records a load error; checkpointing cannot
silently lose its baseline or prune its unmaterialized edits. Known limit: the
refusal is sticky until restart, and a startup replay that must regenerate more
than 1024 WAL-edited chunks without a stored baseline refuses on every start
(the queue lives in memory).

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
  128 queued batches, plus up to 1024 batches the owner holds while a generated
  baseline waits behind an in-flight checkpoint commit (1152 items in all, about 57.6 s at 20 TPS
  for payload-free batches; payload bytes stay under 256 MiB of post-images), and the
  batch being written if its rename has not happened, so an empty queue does not
  close it. A host power loss can also
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
- Write cost is counted, not timed from outside: `/stats` `journal` (and
  `/metrics` `krasnokamen_journal_*`) carries `append_bytes_total` and
  `batch_bytes_last` — bytes written to batch files, which are **not yet
  published** until the rename and directory fsync of that batch — plus
  `file_fsyncs_total`, `dir_fsyncs_total`, the cumulative `fsync_nanos_total`
  and the slowest single fsync since process start `fsync_nanos_max`. Only a
  successful `sync_all`/`fsync_dir` is counted (an error returns before the
  counter), and the counters are read-only relaxed atomics beside the existing
  write order: the format, the order and the error path are unchanged. A
  receipt reports the delta between two samples, so the max is the process-wide
  worst, not a window figure.

### Structure ledger keys

Native plugin domain `@native-structures` stores `structure/<id>`, `survey/<token>`,
`worksite/<id>` and meta `ledger={schema:2,revision}`. Writes dirty/journal only
changed keys plus meta, sealed in one tick delta (cost independent of ledger size);
checkpoint stores all keys. Recovery is a tick boundary.

Before checkpoint/WAL/prune, refuse old single-image `structures` metadata from
checkpoint **or** tail, schema ≠2, records without meta, unknown prefixes/fields,
record revision above meta or watermark above its revision. An advance gives a
new global revision and increments watermark; neither decreases.
Older binaries also refuse new metadata (missing their required fields).
[Operator reset boundary](../OPERATING.md#changing-the-binary).

Work receipts are returned, not stored: next sequence = stored watermark+1.
Only `watermark <= snapshot.revision` is checked; per-receipt sequence/identity/
revision/stage/work/material validations and length equality no longer exist.
Wrong watermarks inside the bound are undetected; repair/replanning means watermark
is not current-stage work sum.

Required `portions`: native settlements have per-stage certificates with
parallel expected-before raw block IDs and sorted registered roots (sapling
recertification); generic plugin structures explicitly use `[]` and whole-box
fencing. Missing/invalid certificates, stage/cell/ID counts, raw IDs, roots or
non-disposition stage order refuse; no old fallback/global plugin-version bump.
This is the LedgerSplit/D8 schema-2 reset: pre-split survey/worksite-only ledgers
also refuse; worlds without old records/settlement rows are unaffected.

Each advance checks its next unprocessed cell, not already-worked cells. Mismatch
re-derives live state; outside owner's per-cell 3D territory it retains forestry/
history vetoes. Veto returns Blocked without stale pause. Loot/payment form one
custody transaction before block writes, then cursor/root retirement/watermark/
structure/meta publish together; failures have no partial effects.
Keep original expected-before evidence behind cursor, never rehash it.
Repair certificates replace/truncate with live cell/stage arrays; checkpoint
retains them and WAL never reconstructs from a later world.

### Recovery evidence and clocks

Network acknowledgement, native tick, game time and durable WAL coverage are
different observations. A held/frozen game clock need not advance with journal
frames; unloaded or unadmitted entities need not age for every elapsed game tick.
Establish the checkpoint/envelope and replay-clock mapping before comparing a
multi-tick census. An asynchronously published dashboard/pump tick is not an
upper bound on the census interval. See the clock images in
[`journal.rs`](../../crates/mc-core/src/journal.rs) and their
[`replay`](../../crates/mc-core/src/journal/replay.rs).

Live active-chunk refresh uses the root's decided pair `H(T), H(T-1)`, also
after dimensional travel; parked Nether/End `TickControl` copies are not clock
authorities. A dormant activation subtracts held time with checked arithmetic:
an inconsistent prefix fails before commit rather than wrapping into catch-up
debt. The already-published WAL remains the recovery boundary.

Recovery installs the recovered native tick and held prefix before admitting
chunks. An extras-only debt anchor ahead of that boundary is invalid. A
checkpoint and its WAL tail also share the climate image's debt epoch; a test
that captures a checkpoint and then keeps writing with the same live state
must persist that image rather than synthesizing a legacy base.

An Anvil export carries that prefix as `level.dat/Data/KrasnokamenSimHeldTicks`
(the chunk extras' `KrasnokamenSimHeldTicks` is anchored at each chunk's own
`KrasnokamenSimLastTick`, so it cannot be served by a store whose prefix
restarts at zero). The import installs it as the running clock's prefix: a
freeze is session state, the simulated age it produced is durable. The field is
unowned by vanilla, like `Data.Time`, and derived the same way: every checkpoint
rewrites it from the live clock (or drops it when the clock is not frozen), so a
re-export of an imported store carries the installed prefix again. An export
without it (a vanilla world, one re-saved by vanilla without the unowned field, or
one written before the witness existed) imports with a zero prefix and its
anchored chunks stay invalid — fail closed, `repair required`. `import-anvil`
validates every chunk's extras against the clock it is about to serve and refuses
the conversion — naming the missing `Data.KrasnokamenSimHeldTicks` — rather than
publishing a store that quarantines them at play; it also refuses a witness that
is negative, above `Data.Time`, or present without a climate or weather image it
could describe.

A process SIGKILL exercises process recovery, not arbitrary power-loss or storage
hardware durability. The [drills below](#hazards-and-defense-layers) state their
fault model; [crash-census evidence](../QA_AGENT_PROTOCOL.md#crash-census-evidence)
keeps lifecycle uncertainty separate from proved loss or conservation.

### Explicit conversion of legacy frozen clocks

Ordinary recovery refuses a frozen legacy clock without debt anchors. On a
**disposable, stopped copy only**, run
`krasnokamen --config <matching-config> world upgrade-game-clock --copy <copy>`.
Keep the original immutable. Canonical paths must differ from the configured
original (including symlink aliases). The command holds the exclusive copy
lease and folds checkpoint plus published WAL using the existing running-time
clock, then writes one checkpoint at recovered native tick `c`. A legacy Held
checkpoint at `K` without `held_since` uses `K` as its initial boundary; tail
Held boundaries come from the preceding running counter. Held tail ticks do
not age dropped items or entity countdowns. Ambiguous transitions refuse
before checkpoint publication. After the commit it durably writes the checkpoint
marker at `c` with the copy's own store id and branch, and only then moves every
file of `journal/` unchanged into `journal-before-game-clock-<K>/`, keeping the
`journal/` directory itself, now empty: a restored copy is refused without it
(see restore below). Once the WAL is archived the marker is the only witness of
`c`, so a store that later rolls back behind it is refused instead of opening at
`K` without its tail; if the marker write fails, the command fails and the WAL
stays in `journal/`. A crash before or during the archive leaves covered batches
in `journal/` that recovery ignores.

Every winning debt, including cold native-only records, gains an explicit held
prefix. Active records re-anchor to `(c, H(c))`; inactive records at `L` use
`max(0, H(c) - (c-L))`. Remaining debt loses `min(remaining, H(c))`, deliberately
discarding uncertain running debt rather than falsely ageing anything.
The durable epoch is `sim_debt_anchored_since=c`; anchorless WAL images at or
after it refuse, including a held world subsequently written by an old binary.

A Held image gets `held_since=c`. When `G>0`, only live-law-eligible Work and
sleep memories in `[G.saturating_sub(1), c-1]` normalize to the law's targets
at `r=c-1` (Work saturating subtraction, sleep checked subtraction). An
eligible value above `c-1` refuses. `G=0` skips normalization, including a one-tick
world and sleep `None`. Panic deadlines and conversion countdowns stay raw.
No anchor becomes stale because of this normalization; it is a conservative
choice, not reconstruction of exact history. An H reset before c9c13b37 can
overstate G and hide an otherwise fresh value below the chosen range.

The auxiliary conversion record
`data/krasnokamen-game-clock-upgrade.json` stores checkpoint tick, recovered
tick, held tail tick count, H, chosen boundary, active/inactive and
normalized/raw counts and per-chunk dropped debt. The operator command logs
those totals. Chunk/container post-images, entity tombstones,
plugin state, migration transit rows and block-log WAL deltas are checkpointed
before archiving. Existing immutable baselines remain authoritative.

Settlement clocks are restored through their existing owners, not retimed
generically. Commerce employment `next_due` and caravan `game_deadline`
distinguish WorldTicks from GameTicks and their restore functions convert
legacy deadlines once. Production job `next_due` does the same; consumption
`next_due` is already GameTicks. Migration walking uses game ticks. Production
`accepted_tick` is provenance, not an elapsed-time clock. Army `next_attack`
is a native absolute cooldown and remains native. Construction cursors,
allocator queues and site-turn budgets carry no persisted elapsed-time anchor.
Villager courtship/despawn/restock and scheduled block/fluid deadlines likewise
remain native; item ages/countdowns exclude held time during conversion too.

Pre-rename Anvil chunks carrying `SolarisSimLastTick` or `SolarisSimDebt`
refuse import: the export has no historical held-time H witness. Use a fresh
world, or convert on a release that still retained held time. Pre-rename
chunks without debt and vanilla Anvil retain their ordinary import path.


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

The checkpoint manifest carries a required `manifest` format version beside its
per-domain entries, written into the root and both child images and compared for
exact equality on recovery. Every added, removed or reinterpreted persisted
`CheckpointManifest` field bumps it, including a field `serde(default)` would
otherwise supply. There is no converter and no previous-version read path: a
world written under another version refuses startup. The constant lives in
`crates/mc-core/src/native_checkpoint.rs` (`MANIFEST_VERSION`) and is mirrored
by `crates/mc-world/src/native/checkpoint.rs` for readers that open a store
without the core.

### Checkpoint marker

`<world>/checkpoint.marker` is a fixed-size file independent of redb. It records
the store's immutable random `store_uuid`, the branch of this copy of the store
and the tick of the last checkpoint whose WAL was acknowledged. The store id is
written once into the native header when the store is created, by world creation
and by the Anvil import (`NativeStore::create`); it is a required key, so a store
without it refuses to open and is never given a fresh identity. The marker cannot
be derived from redb because redb's unattended repair switches to an older commit
when the newest one fails its checksum, and the damaged commit leaves no trace
inside the store (Books R4-P1, `WAL-REDB-REPAIR-ROLLS-BACK-BEHIND-PRUNED-CHECKPOINT`).
Every redb repair is therefore logged at `error!` as a possible rollback to an
earlier commit: the repair callback cannot tell an unclean stop from a damaged
commit or header slot.

The branch tells copies of one store apart
(`DIVERGED-BACKUP-COPIES-SHARE-STORE-IDENTITY`). A world as created or imported
is on the root branch, and its marker keeps the 64-byte format-1 layout
`[KCMARK 1][store id][tick][SHA-256]`; every `world restore` gives its copy a
random 8-byte branch (all 64 bits random, never the root or the backup's own;
two restores collide with probability 2^-64, which is improbable, not
impossible), and that marker takes the 72-byte format-2 layout
with the branch after the store id. Each branch has exactly one encoding (a
format-2 marker naming the root branch is corrupt), so no marker is converted: a
world that was never restored writes the same bytes it wrote before branches
existed. Every checkpoint writes the marker on the writer's own branch, and the
crash-window rewrite at open keeps the branch it read. The branch lives in the
marker rather than in redb for the same reason as the tick: a repair that rolled
the store back behind the restore would drop it without a trace.

Every durable checkpoint, starting with a new world's initial one, writes the
marker in this order: redb `Durability::Immediate` commit, then a temporary
file, `sync_all`, rename over the marker and the world-directory fsync
(`atomic_file::replace_file_durable`), then the checkpoint ACK, and only then the
prune of the WAL batches that checkpoint covers. A marker write error is a
durability failure: no ACK, no prune, and the writer stops the world (fail
closed); the marker keeps the previous checkpoint, so the retained WAL still
replays on the next start. On Windows there is no directory fsync
(`sync_parent_dir` is a no-op there): the marker's rename survives a process
crash, and a power loss can still lose the directory entry, exactly as for the
native store file and the journal directory entry.

Before any replay, startup refuses a store that has a checkpoint while the marker
is missing or fails its checksum, whose `store_uuid` differs from the native
header, or whose stored checkpoint tick is behind the marker's tick — the store
rolled back behind an acknowledged checkpoint whose WAL is already pruned. A
store ahead of the marker is the crash window between the commit and the marker
write: it opens and the marker is rewritten to the store's tick. There is no
converter; a world written before the store id and marker refuses startup
instead of being rewritten.

Hot backup copies the marker with the store and refuses a marker ahead of the
committed checkpoint. The offline backup copies it as an ordinary world file and
covers it by the manifest digest. `world restore` verifies every hash and builds
the copy in a staging directory beside the target, `<target>.restore-staging`,
never in the target itself. It checks the restored pair — store id and
checkpoint tick read back from the copied store against the backup's marker,
which is verified from memory and not copied — and on a mismatch removes the
staging directory and reports no world. It then starts the copy's own branch:
the store id is the world's identity and travels with every backup, so without
it two copies of one backup, or a copy and the world it came from, would replay
each other's tails. The restore mints a random branch that is neither the root
nor the backup's own, moves every copied batch above the store's checkpoint onto
it (`journal::rebind_tail`: each frame is first checked against the copied store
and branch, its tick range, length and CRC, then only its eight branch bytes are
rewritten in place and synced; batches at or below the checkpoint never replay
and stay as copied), and writes the copy's first marker, naming the new branch,
last. The staging directory's name is reserved: `Core::open`, offline recovery,
the offline tools' store entry (`NativeStore::open_checked`,
`check_marker_file`), every marker check (`store_marker::verify`/`check`) and
backup refuse a directory named `*.restore-staging` (or
`.krasnokamen-conversion-*`) by name, directly or through an alias, even once
the copy's own valid marker is written (the opens that can write refuse it
before touching its store, read-only ones after opening it read-only), and
restore refuses such a name as its target. Before that marker the staged store
also has a checkpoint and no marker, which every open refuses on its own. The
restore then fsyncs every directory of the staged tree (each file was synced
when written), publishes it with one rename over the empty target it keeps
leased from before the staging directory is created until after cleanup, and
fsyncs the parent. Restore runs on Unix (Linux, macOS) only and refuses
elsewhere: Windows renames no directory over another and keeps the lease in a
file inside the target, so publishing there would mean releasing and removing
the target first, and another process could take it in that gap. An
interrupted restore therefore leaves the target as it was or the complete copy,
never a partial one, plus the staging directory, which the next restore refuses
(it is never reused or removed automatically) until the operator removes it.
"As it was" is an absent target or an empty directory, which `Core::open`
initialises as a new, empty world; only the target a finished restore published
is the restored copy. A refusal removes only what the restore owns: its staging
directory, and the target only if the restore created it and it is still empty;
anything else at the target stays. Every copy a restore publishes has a
`journal/` directory, an empty one included, and an offline backup keeps an
empty one: recovery reads a missing journal as an empty tail only on the root
branch and refuses a restored copy without it, whose tail is lost. Backup
refuses a restored copy without `journal/`, and restore refuses a backup of one,
on their own and whether or not a content cache lets the command replay the
copy, so a lost tail never becomes the empty journal of a fresh restore.
Restoring one backup twice therefore yields two copies that refuse each other's
journals, and a backup of a restored copy carries that copy's branch until its
own restore branches again. The branch is held by the marker alone, so the
bindings do not catch every swap inside a world directory: a sibling's marker
moved together with its journal over another copy's store is that sibling as far
as they can tell, and a sibling's `state.redb` swapped in at the same or a later
checkpoint tick carries the same store id and passes the marker check, so the
local journal may be replayed onto it (a checkpoint that falls inside a batch can
still be refused by tick range, which is not a check to rely on). A world
directory is therefore always moved and copied whole. A hot backup is not reported
successful until it has been tried: the finished copy is opened read-only and
the ordinary recovery reader and WAL replay run over it, requiring the recovered
tick the copy claims (`HOT-BACKUP-STATE-DATABASE-COPY-UNVERIFIED`). Every batch
of the copied tail is bound to the same store id, branch and palette as the world
that wrote it, so a CRC-clean but unreplayable batch fails the trial restore
instead of surfacing at disaster time.

Every offline tool that opens a store without `Core::open` goes through the same
entry (`NativeStore::open_checked`, or `check_marker_file` where the worldgen
revision is legitimately rewritten) before its first write or reported read:
`world prune` and `world compact`, offline pregeneration, the worldgen-revision
upgrade, `world census`/`world probe` and the crash and custody censuses. A tool
must not report or rewrite a store that lost a committed checkpoint.

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
   Retained entity and player NBT is additionally canonicalised: a duplicated
   `Tags` key, non-string or non-list `Tags`, or trailing bytes inside an opaque
   image refuses startup with a `CoreError::Storage` error naming the image
   (`native_checkpoint.rs` player/entity hydration, `core/recovery.rs`,
   `journal/replay.rs`); duplicate *elements* are de-duplicated instead. The
   on-disk format is unchanged — fix the image (remove the duplicate key/bytes)
   or restore from a checkpoint/WAL that does not carry it.
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
committed store into place. The first-start marker is removed after the initial
checkpoint is durable. A remaining first-start marker permits retrying interrupted
first start; a store without a checkpoint or marker fails closed. (This is not the
[checkpoint marker](#checkpoint-marker): that file is `checkpoint.marker` and
stays for the world's lifetime.) Required checkpoint player records
must decode and match their keys; offline player custody is retained rather than
silently discarded.

### Datapack worldgen staging and identity fields

Pack worldgen resources are binary, not UTF-8 by default. Each selected pack
applies its filters to lower layers, then adds its own resources; builtin
vanilla occupies its actual selected position, and authored biome convention
defaults are its lowest layer. Biome tags merge in order, honor `replace` and
resolve required/optional references across namespaces. Worldgen JSON/NBT and
cache reports are staged as a private bounded copy; neither shared vanilla files
nor hard links are modified, and generators compile owned immutable values from
it. The staging directory is removed after construction, including refusal. The
reserved world-owned `.krasnokamen-worldgen` directory is disposable after a
crash only with its fsynced sibling `.krasnokamen-worldgen.owner` marker (or when
empty); the marker is never deleted with the tree, so a crash during cleanup
cannot erase ownership proof. Unowned nonempty paths, unexpected top-level
entries and symlinks refuse startup (`crates/mc-core/src/datapacks_worldgen.rs`).

`level.dat/Data` also carries Krasnokamen identity and clock fields that vanilla
does not own: `KrasnokamenTerrainEpoch` and `KrasnokamenEntitySalt` (world
identity) and `KrasnokamenSimHeldTicks` ([recovery clocks](#recovery-evidence-and-clocks)).
The normal checkpoint level-metadata path owns them and
`KrasnokamenWorldgenInputs`; there is no WAL image of them and no separate
startup transaction. Unsupported worldgen domains or types, including Moog
piece-count rules, refuse the whole pack through the one shared
`mc-data`/`mc-worldgen` supported-type authority; a removed pack's saved
structure starts stay untouched and inactive
([operator view](../OPERATING.md#datapacks-and-command-blocks)).

### Player custody admission

Player custody admission refuses missing, non-byte, out-of-range or repeated
inventory/ender-chest slots and repeated or unsupported equipment slots before
projecting entries into native arrays. Offline import also checks legacy
`Inventory` slots before `PlayerEquipmentFix` can collapse armor/offhand
duplicates. The error names the source player and slot; a refusal publishes no
world and leaves the source unchanged. This is a custody guard, not vanilla
repair: vanilla defaults a missing `Slot` to 0, skips out-of-range entries and
lets later repeated slots overwrite earlier stacks (`ItemStackWithSlot.CODEC`,
`Inventory.load`, `PlayerEnderChestContainer.fromSlots`), while every vanilla
26.1.2–26.3 writer encodes a `Slot` for each saved entry, slot 0 included.
Coverage: `mc-world-conversion::player_slot_import_tests`.

Native WAL admission requires exactly 41 player inventory slots before applying
a player post-image; a short or long vector refuses the whole recovered world
rather than reaching the join path's empty default. A nonempty player item
compound without a string `id` refuses import and checkpoint recovery, including
equipment, cursor and crafting-grid custody; an empty compound remains an empty
slot. Coverage: `mc-core::native_checkpoint::tests::player_custody`.

Repeated projected player custody names refuse before first-match decoding or
removal from the opaque image, on WAL player opaque bytes before replay and
before player serialization too, so a WAL post-image cannot bypass the codec's
duplicate-name refusal and lose a second cursor/crafting-grid payload. WAL player
opaque NBT may not contain any root field in `player_owned_field`
(`crates/mc-core/src/journal/player_nbt.rs`), even once or empty: those fields
belong to the typed post-image, and serialization removes them from opaque data.
The same applies to `recipeBook.recipes`, whose authority is the typed unlocked
recipe set; other `recipeBook` fields stay opaque. Admission refuses the whole
recovery before applying the batch, naming player UUID and field.
Coverage: `mc-world-conversion::wal_owned_player_fields_tests`.

### Shutdown and the world lease

Explicit shutdown and a plain drop share one stop order: `Core::shutdown` and
`Drop for CoreInner` both call `CoreInner::stop_owners`
(`crates/mc-core/src/core.rs`): signal the tick owner, join the tick (its final
checkpoint and the journal `Shutdown`), join the journal (final WAL flush), and
only then release the exclusive world lease. Both take the same `Option`
handles, so the paths are idempotent; with unpoisoned owner mutexes a plain
`drop(core)` never detaches a running owner or frees a lease an owner may still
write. Regression: `core_drop_join_tests::plain_drop_holds_the_world_lease_until_the_publication_commits`.

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
`battle_faction_survives_native_checkpoint_wal_and_restart`. Persistence
changes require Main's independent review.

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

Undecodable stored chunks or owner-refused block-entity/runtime records are
quarantined: never loaded, generated over or rewritten; stored bytes stay intact.

| Custody at refusal | Checkpoint/recovery |
|---|---|
| No unsaved state (load refused, never edited) | Skip chunk, checkpoint others; log dimension/position/reason/counts every capture. Skip chunk-local scheduler projections (`scheduled projections skipped`) and debt/inhabited metadata, but retain global scheduled block/fluid queues in manifest; restart restores them. |
| Unsaved state | Decline **whole** checkpoint every time (`checkpoint declined`), retain WAL and dirty marks. Early hydration reports `container_records`; capture reports unsaved entries/skipped scheduler projections. |
| WAL block/opaque image (e.g. campfire, hive, brushable block) needs an unreadable chunk | Refuse startup with load error; no checkpoint or dropped write. Stored chunk must become valid before replay can proceed. |

Unsaved = pending block writes, live container records, edited sections or
`block_entity_dirty` edit/removal mark (even with no remaining record).
Skipping those while advancing the whole-world fence can duplicate custody:
covered WAL is ignored even if retained. Runtime-refused live containers therefore
hold every checkpoint until restart even without edits; load-refused chunks
after restart skip only if replay adds no unsaved state.
There is no repair/remove-quarantined-chunk command: offline restore of a backup
loses subsequent play but preserves refused bytes for inspection.
Anvil/native loading validates crafter/shelf/shulker/conduit by **owning block**,
not record ID.

Behavioral regressions:
- `scheduled_ticks_of_a_refused_chunk_survive_checkpoint_and_restart`;
- `opaque_replay_into_a_chunk_refused_at_load_fails_recovery_and_keeps_the_wal`;
- `container_transfer_replay_*`: actual transfer, untouched neighboring chest,
  healthy materialization or refusal, repair, checkpoints and repeated
  restarts with full player/container custody. Matrix: chest/trapped/barrel/
  copper chest, furnace/smoker/blast furnace, hopper/brewing, dispenser/dropper,
  crafter/shulker/shelf/jukebox, decorated pot/chiseled bookshelf/lectern.
  Typed-only tails hold checkpoints; opaque-owner tails require residency and
  refuse unreadable startup ([capture ordering](#checkpoints)).

**RC5 bookshelf failure:** slots 1–3 passed both the three-slot shelf decoder
and six-slot owner. Wrong owner hydrated first; interaction replaced its wrong-size
inventory with empty six slots. Slots 4–6 instead refused load. Format did not
change: fix removes overlapping decoder.
`chiseled_bookshelf_old_save_interaction_preserves_all_books` uses the old stored
record, no live/WAL substitute, through interaction/restart. Earlier slot-6
refusal/custody tests did **not** prove absence of this slot-1–3 loss.

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

Offline backup/restore requires stopped, exclusively leased world; restore
verifies inventory/hashes, gives every restored copy its own branch in an
unpublished staging directory and publishes the finished copy with one rename.
Console hot backup is native-writer checkpoint+WAL copy, decoding/CRC-checking
every batch (Books R5 G3) and checking store/branch/palette identity.
Neither reports success until ordinary recovery replays the copy to its claimed
tick; unreplayable copies are withdrawn ([marker contract](#checkpoint-marker)).

Trial recovery is read-only or repairs a **scratch copy**, never backup bytes.
Scratch DB sits beside destination (same volume, outside archive inventory):
peak is archive+one DB. Close before removal and report cleanup errors; crash
may leave `.verify-<pid>-<n>` for manual cleanup.
A filesystem copy of a live writer is not backup. Anvil omits native-only plugin/
settlement authority and block history. Use [operating](../OPERATING.md) and
[migration](../MIGRATION.md) commands and inspect their results.

Backups retain database custody throughout the copy. The native redb backend
and the hot-backup reader share the exact lease-holding file handle (not a
reopened path or duplicated Windows handle); positioned reads have independent
cursors, and the native writer serializes the copy between committed jobs. The
joined native writer owns the backup store; callers send only a destination and
a reply channel, and both writer joins finish database destruction before the
world lease is released. Offline backup and restore likewise read through their
lease handle; no unlock/copy/relock interval exists. Windows directory
durability uses the platform directory-sync helper; conversion syncs only its
output staging files, opened with write access but without creation or
truncation ([FlushFileBuffers requires GENERIC_WRITE](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers)).
