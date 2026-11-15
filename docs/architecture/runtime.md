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
| Datapack reload and function-trace writes | short-lived `datapack-worker` thread in `mc-core` (`datapacks.rs:1322-1326`) | one thread per job, result over a bounded channel, outside the tick |
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
tick: after broadcast `Lagged`, the next retained output identifies exactly
which recipients lost impulses, even if that retained tick also knocks them.
Queue registration seals the latest native count, not the first output the
pump happens to process. Older outputs cannot lower that baseline or replay
their impulses into the new client; later skipped impulses still close it.
Those connections close with `skipped-explosion-knockback`; unaffected viewers
reconcile snapshots. These counters are transient edge observations, not
gameplay authority or persisted state. Socket completion and client rendering
are separate from native commit.

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
