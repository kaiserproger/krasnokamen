# Actual runtime map

Status: current server after ADR 0014 Phase 5. The model is
[ADR 0013](../decisions/0013-core-model-v2.md); the summary is
[ARCHITECTURE.md](../ARCHITECTURE.md). Historical extraction paths are not
current runtime entrypoints.

## Entry points

```text
crates/mc-edge/src/bin/krasnokamen.rs
  operator_cli: parse --config (default server.toml), dispatch offline command
  server: resolve/validate content cache and immutable startup tables
  mc_core::Core::open: native world, recovery, gameplay and tick ownership
  EdgeServer: connections, authentication, fanout, Loader and dashboard state
  serve_with_console: listener + optional stdin command admission
```

Offline commands (`content import|verify`, `backup`, `restore`,
`world import-anvil|export-anvil`) finish without starting normal serving.
`--no-console` disables stdin commands; it is not a TUI/plain-input switch.
Normal serving, console `stop` and SIGINT/SIGTERM share native shutdown and
final checkpoint behavior. Tracing goes to stderr; console feedback to stdout.
The console rejects invalid UTF-8 input lines without terminating the stdin reader.
Play chat and command packets containing §, ASCII control characters, or DEL
are disconnected with the vanilla illegal-characters reason.

## Execution boundaries

| Boundary | Owner | Constraint |
| --- | --- | --- |
| Authoritative state and tick | `mc-core` | decoded input → native validation → entity intents → deterministic merge → block work → one tick change set |
| Parallel entity work | core worker pool | reads immutable state; writes disjoint entity next-state data, not shared regional state |
| Disk/journal/checkpoint I/O | native journal worker | freezes immutable/owned checkpoint images; no synchronous fsync on a gameplay reply path |
| Datapack reload and function-trace writes | short-lived `datapack-worker` thread in `mc-core` (`datapacks.rs`) | one thread per job, result over a bounded channel, outside the tick |
| Socket decode/write and sessions | `mc-edge` connection tasks | no independent inventory, world or entity authority |
| Per-viewer packet planning | edge fanout workers/pump | own workers, bounded connection credit, retained projection revisions |
| Component execution | `mc-plugin-host` worker | serial bounded guest callbacks outside the tick; `mc-script` owns DTO/admission |
| Loader wire | `mc-loader` | manifest/artifact/ACK/sound codecs, no gameplay state |
| Vanilla capture/tooling | `mc-vanilla` | content import and shared harness client/process helpers |
| Run/process/receipt lifecycle | `tools/harness` | v2-only server launch; one owned artifact directory and fail-closed receipt |

World fanout may skip to the newest immutable snapshot and reconcile the
viewer's held world state. Each viewer's world credit is its world queue's
free frame slots (hard) and free bytes (soft: the last admitted group may
overshoot); entity updates that do not fit wait, ranked by wait time and
distance, and are planned again from a newer snapshot. Recorder connections
(`[recorder]`, `crates/mc-edge/src/recorder.rs`) track every entity type to
their view distance and get a per-tick `krasnokamen:tick` marker. Visual-only
explosions spend world credit and may drop. Non-zero recipient impulses are
summed once per publish, independently of visual range, and ride one reliable
packet rather than filling the FIFO on a large TNT burst. Reliable overflow
still disconnects explicitly rather than losing an impulse.
The carrier is also sent to a hit player beyond the 64-block visual range:
this deliberate exception preserves knockback using `ClientboundExplode`,
including that packet's particle and sound fields, without another motion path.
Every published
snapshot carries cumulative per-player knockback-tick counts and the last such
tick: after broadcast `Lagged` or a queue-budget skip (below), the next
retained output identifies exactly
which recipients lost impulses, even if that retained tick also knocks them.
Queue registration seals the latest native count, not the first output the
pump happens to process. Older outputs cannot lower that baseline or replay
their impulses into the new client; later skipped impulses still close it.
Those connections close with `skipped-explosion-knockback`; unaffected viewers
reconcile snapshots. These counters are transient edge observations, not
gameplay authority or persisted state. Socket completion and client rendering
are separate from native commit.
An ordered dimension reset discards only the traveler's stale old-world
explosion impulse, without disconnecting or projecting it into the destination.
The reset first fences that recipient's queued best-effort world frames with a
new FIFO generation. The real writer discards stale generations before delivery;
already-taken bytes remain before the reset, and subsequent destination frames
remain eligible, including a destination impulse in the same tick.
Other viewers and destination-world impulses are unaffected. Delivering the old
impulse before reset would require splitting world fanout around the independently
drained reliable outbox; the existing pump routes that outbox first. Dropping it
is deliberate delivery policy, not vanilla motion-reset parity (26.3 retains
motion with respawn keep-data bit 2); skipped-publication detection remains
fail-closed.
The pump skips every output that waited in its queue past `PUBLISH_QUEUE_BUDGET`
(1.6 s, measured from the output's monotonic `TickTimes::published_at`) while a
newer one is queued, exactly as after `Lagged`. The reliable outbox drains only
through the published tick, so this bounds how long a slow pump holds reliable
changes (a `Joined` too) behind stale outputs. It bounds the queue wait, not
delivery. Reliable changes stay FIFO per connection, arrive once, and never
drain past the published output. A full reliable queue still closes the
connection; a refused world frame closes nothing and marks that viewer's held
geometry and planned entities for repair. After a skip, a held chunk is sent
again when its own revision moved and, under anti-xray, when a side
neighbour loaded, unloaded or changed revision since the last publish before
the skip (an unloaded side counts as open, so each changes the ore mask across
the chunk border). Entities that
appear and vanish within a skip are not shown. One-shot effects and recorder
tick markers of skipped outputs are lost. A skipped knockback impulse closes its
recipient as above. A pump that stops for good still halts the core through
the outbox stall watch. Policy and limits:
[ADR 0013](../decisions/0013-core-model-v2.md#threads-and-backpressure).

### Reliable delivery and backpressure

Journal admission is a bounded FIFO that waits under pressure; accepted batches
are neither dropped nor reordered, and a dead journal stops the core
([persistence](persistence.md#wal)). Reliable per-player results are ordered and
bounded; exhaustion disconnects (`reliable-queue-full`) rather than silently
discarding results. Bulk producers such as the scoreboard join snapshot and one
scoreboard publication send one ordered batch per connection, and the edge
queues its frames as one item, so a large scoreboard does not hit the bound.
Chat and system-message lines are bounded at their source (the sender's spam
budget, the command sequence budget), so a connection's trailing run of them
compacts into one `TextBurst` entry at the cap instead of disconnecting a
recipient. The knockback counts above are aliased, not copied: the publisher
holds the output's canonical map `Arc` under a write guard through the
synchronous broadcast and the scalar getter takes its read guard, so queue
registration is ordered wholly before or after a publication without another
count writer. Entity ids stay stable through dimension travel; the observer
resolves the recipient across current worlds after travel commits.

Durable plugin events have their own bounded queue, and one slow guest callback
stalls every plugin's delivery. The core therefore refuses new plugin commands
and drops plugin-menu clicks while a quarter of the queue capacity is waiting;
the world halts fail closed only when events that admission cannot refuse
(graves, guest command results) pass the full capacity, and the halt log counts
the backlog by event kind.

Input-ordered chat proves native input handling, not geometry delivery: reliable
traffic can overtake the world queue. The operator-only
`krasnokamen:setup_world_fence` request keeps one transient nonce per
connection; the edge appends the nonce response to the same world FIFO only once
a publication has the viewer's current dimension and complete geometry window
with no block or chunk-repair debt. The setup client dispatches all preceding
frames before using the marker. Disconnect drops the fence; gameplay authority
and durability are unchanged.

Operator changes reserve the whole authoritative input batch before changing
edge-owned operator state. An async mutex serializes admission and changes; the
initiator's live permission and lock status are rechecked after reservation.
The operator snapshot and a permission update for every registered session of
the target UUID are sent without an intervening await; queue saturation waits
for capacity rather than polling or publishing out of order, and a core stop
while waiting applies nothing. Moderation commands re-acquire the same fence
after selector resolution and recheck the initiator before any kick, ban or
pardon, holding it through those effects.

Pickup publishes changed slots in the active menu. Player inventory slots hidden
by that menu use the dedicated player-inventory packet rather than a window-0
contents image, so another menu's cursor and crafting slots are not overwritten.

### Connection liveness

Play keepalive follows vanilla and lives in the connection's writer task
(`crates/mc-edge/src/server/connection.rs`, `write_play`). Every 15 s it sends a
challenge unless one is still outstanding, so there is at most one, and a new
period never replaces it. The connection closes with `keepalive-timeout` 30 s
after the challenge unless its exact echo arrives; a wrong echo closes it with
`keepalive-invalid-reply`. Other inbound traffic does not extend the deadline.
The writer serves keepalive before reliable and world frames, and the reader
forwards echoes to the writer directly, also while an administrative command
holds the remaining input (`await_play_operation`), so a large outbound backlog
or a slow owner reply does not starve the echo. Before Play, Configuration and
Login use their own read deadlines; the Loader exchange sends its own
Configuration keepalive every 10 s (`crates/mc-edge/src/plugin_loader.rs`).

### Tick cost accounting

`/load-metrics` preserves the original completed `TickTimes` for each tick.
`total_us` is the pre-publication core cost: gameplay plus WAL image building,
bounded writer enqueue and generated-chunk capture. Its disjoint `phase_us`
fields cover inputs, chunk admission/hydration, entity/merge/block work,
commit, snapshot/views/publication construction, plugin admission/observations,
dimension coordination, journal building/queueing and generated saves.
`coordination_us` is the measured dimension envelope less its local phases;
it includes routing, travel, global projection, return-time scratch destruction
and sub-microsecond phase boundaries, not an inferred disk/network cost.

`owner_pre_us` and `owner_post_us` are **outside** `total_us`: acknowledgement
and input drains before gameplay, then reliable/world handoff and checkpoint
capture after it. Include them only when comparing the phase sum to `wall_us`
(the complete active iteration, excluding pacing). Microsecond rounding leaves
small gaps. The journal worker owns fsync, generation/lighting workers own
off-tick work, and edge fanout has its separate dashboard `publish` distribution;
writer backpressure paid by the tick is visible in `journal_queue_us`.

Admission seam lighting remains synchronous on the tick: moving it off-tick
requires revision/reconciliation design. A cross-crate `LightLayer::get`
inline hint addresses sampled accessor cost; full recompute and seam oracle
tests fence correctness. Remove the hint if a future profile no longer
justifies it. Historical interleaved diagnostic windows reported tick p50/p99
of 7.576/22.008 ms before and 6.145/19.864 ms after, with variable load; these
are not an isolated speedup or a production guarantee.

## Persistent and external boundaries

The serving world is native `native/state.redb` plus tick WAL. Player, world,
entity and plugin effects recover at the same tick boundary; an unflushed tail
can roll back consistently. Anvil import/export is an explicit stopped-world
conversion with a new destination and receipt, not a second serving backend.

Production core loads independent compiled component packages. Neither
`../solaris-loader` nor `../solaris-default-plugins` is required to build it;
gates requiring actual sibling behavior declare that prerequisite explicitly.
There is no v1 server variant or old regional/storage actor fallback.

Use [state-ownership.md](state-ownership.md) for authority and
[mutations.md](mutations.md) for end-to-end commit/publication routes. Consult
current symbols before changing a boundary; generated indexes only navigate.
