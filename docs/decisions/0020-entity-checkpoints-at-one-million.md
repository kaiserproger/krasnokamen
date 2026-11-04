# ADR 0020 - Entity checkpoints and reopen at one million entities

**Date:** 2026-11-04  
**Status:** Proposed, amended after committee review. Not implemented; ADR 0013
"Durability" remains current until this is accepted and measured.  
**Parent contract:** [0013](0013-core-model-v2.md) (atomic native checkpoints, recovery).

## Problem

Every checkpoint (1,200 ticks) captures **all** entities on the tick owner.
`checkpoint_entities` builds an owned `VanillaEntity` (NBT fields) per slot
(~10 µs each, rayon) and hands the whole image to the writer. `write_entities`
encodes every record, decodes and compares every stored row, and scans the whole
`ENTITIES` table to tombstone omitted rows. Cost is O(population).

- **Capture transient.** At 262,144 entities: +1,176,381,720 B materialized
  (≈4.5 KB/entity) vs +22,765,216 B streamed. [INFERENCE] At 1M that is
  ≈+4.2 GiB on ≈4.3 GiB steady anon, so over 8 GiB.
- **Checkpoint-tick p99/max.** 50.8 ms at 10k and 279.2 ms at 100k.
  [INFERENCE] About 2–3 s at 1M.
- **Streaming.** Two 256-slot batches over `SyncSender` kept the transient at
  +49 MB at 1M, but the owner blocked on the writer: 1,151.8 ms (22.66×) at 10k,
  6,061.9 ms (21.72×) at 100k, single ticks up to 17.9 s. Not integrated.
- **Reopen.** With streaming, steady 1M anon is 4.09–4.43 GiB, and reopen is the
  peak: 6.45–6.69 GiB at startup and 7.04 GiB during the census.
- **Earlier fixes.** Image compaction (888 → 472 B) saved about 3% steady.
  Replay boxing cut 262k reopen anon by 28% (2,162 → 1,549 MiB), but median
  readiness rose 43.0 → 67.9 s.

## Constraints (0013, unchanged)

1. **No disk wait on the tick.** Handoff is bounded, with one periodic
   checkpoint in flight. Exception: shutdown queues a final checkpoint FIFO
   behind an in-flight one (`core/tick_loop.rs` 465–467).
2. **Exact recovery.** Recovery is checkpoint `c` plus the complete WAL after
   `c`. Mob motion and idle counters are checkpoint-durable only.
3. **WAL first.** The WAL is fsynced through `c` before writes. One Immediate
   redb transaction covers every domain and dimension. WAL up to `c` is pruned
   only after commit.
4. **One store.** No dual-write, second store or disk-format change; fail closed.

## Options

| Option | Tick work at `c` | Extra memory at 1M | Writer | Complexity / risk |
|---|---|---|---|---|
| A. Encode from the published immutable image | O(pages) freeze plus ~60 MB hot-column copy | Pages written during the multi-second write; up to the whole cold store | O(N) off-tick (~10 CPU-s) | High: paged COW on every mutable row path |
| B. Streaming with a byte-budgeted backlog | Waits whenever writer lag exceeds the budget | No-wait guarantee needs budget ≥ image, which is D | O(N) + scan | Medium; prototype measured 22× |
| C. Incremental (dirty-set) checkpoints | O(dirty) records + one `u64` scan per slot | ≈4.5 KB × dirty | O(dirty + tombstones) | Medium; needs the custody list below |
| D. Status quo | O(N), ≈2–3 s | ≈+4.2 GiB | O(N) + scan | Fails 1M in 8 GiB |

**A.** Chunks already work this way: capture freezes dirty chunks by `Arc`.
`EntityViews` carry only the wire view: identity, type, pose, velocity, ground,
equipment, item, health, name and wire data. The rest is missing:

- retained `ColdRow`/`Body` state: villager state, every `EntityStateImage`
  field, full active effects, projectile state, pickup delay;
- `TickState` raid, spawner and End-fight fields.

There are two ways to add it. Publishing that state per tick duplicates the
journal baselines: +0.5–0.9 GB steady [INFERENCE]. Paged copy-on-write rows add
a copy check to every mutable path. Deferring A is an engineering choice, since
it means a wider ownership change in the hottest store. It is not a measured
impossibility.

**B.** The image must be consistent at `c`, and only the owner can read live
state before tick `c+1` changes it. So the owner emits every record before it
advances, and the backlog it needs equals the writer's maximum accumulated lag.
Only a whole-image backlog guarantees no checkpoint backpressure under an
arbitrary writer delay, such as fsync contention or a slow previous commit, and
that backlog costs as much memory as D. The measured prototype cut capture
allocation by 98% but was 22.66×/21.72× slower. This rejects that prototype and
the lack of a guarantee, not every intermediate budget.

**C.** `ENTITIES` is already a materialized base, updated in place in one
transaction. The checkpoint writes every row whose persisted projection may
have changed, plus explicit tombstones, so the DB still holds the full image at
`c`. Recovery, WAL order and the disk format are unchanged. Idle rows cost
nothing; when every row is dirty, C costs the same as D.

## Decision (proposed): C, plus a replay-layout fix

**Dirty rule.** Capture a row at `c` when its persisted projection may differ
from its stored DB row:

1. **Store write.** The row had any store write since the last durable (ACKed)
   checkpoint.
2. **Tick-relative projection.** Today one field depends on the capture tick:
   a pending villager `golem_detected_recently` deadline. It is encoded as a ttl
   relative to the capture tick and decoded against the manifest tick
   (`villager/persistence.rs` 634–651, 1034–1050). Skipping the row would extend
   the deadline after reopen. Such rows are captured every checkpoint while the
   deadline is pending. The cutover audits every encoder that reads the capture
   tick.
3. **Outside owners.** Some fields are owned outside `EntityStore`: raid
   membership, the ominous item spawner's item and End-fight bodies
   (`entity_capture_extras`). Rows of those types are always captured, so a
   field that disappears is rewritten too.

**Marks are ticks, not bits.** Each slot carries `changed_tick`, which moves
with the slot on swap-remove and transfer. Store methods set it themselves.
Removals append `(uuid, tick)` to a log before the row is dropped. The core
keeps `durable_tick`, the last ACKed checkpoint.

- **Capture.** At `c`, select slots with `changed_tick > durable_tick`, plus
  rules 2–3, and build them with the same `entity_capture_record`. Tombstone a
  logged UUID newer than `durable_tick` only if it is live in no dimension at
  `c`, so transfers and rebirths become upserts.
- **Commit.** In the same transaction the writer upserts and writes explicit
  tombstones. A tombstone with no stored live row is skipped, so short-lived
  items never reach the DB. The full-table omission scan is deleted; per-row
  validation stays.
- **ACK.** A successful ACK of `c` only sets `durable_tick = c` and drops log
  entries ≤ `c`. Nothing is cleared at capture, so these keep full custody:
  capture or handoff failure, a failed commit, a lost ACK.
- **Newer marks are safe.** An ACK can never clear a newer mark. The queued
  shutdown checkpoint selects everything since `durable_tick`, a superset of the
  in-flight job, so it is complete whether that job commits or fails.

**Custody list (cutover contract, `f96a4d93`).** These paths write persisted
state without WAL marks. Each must set `changed_tick`:

- **`entities.rs` methods:**
  - `advance_age` and `advance_admitted_ages` (420, 435);
  - `advance_villager_clocks` (484) and `set_harvest_ticks` (1132);
  - `set_motion_state` and `apply_motion_result` (1188–1268);
  - `set_pos`, `set_vel`, `set_rot` and `set_ground` (1311–1370);
  - `row_mut_unmarked` (1179), which still skips WAL and wire marks.
- **`row_mut_unmarked` users:** lifecycle counters (`entity_life.rs`
  1065–1138) and `FallDistance` (`mob_fall.rs` 29–83).
- **`spawn` then `set_uuid`** (`plugin_npcs.rs` 460–470): `set_uuid` logs the
  previous UUID as a removal.
- **`remove` (665) and `transfer_to` (727–828):** log the UUID, move the mark
  with the swapped slot, and mark the destination.

Chunk eviction (`world.rs` 624–667) removes chunks, not entities, so it never
creates an entity tombstone. Checkpoint marks are independent of WAL and wire
marks.

**Reopen seeding.** The WAL fold drops a row, and its UUID, on `RemoveEntity`
and on `Entity`/`EntityVitals` with `alive = false` (`journal/replay.rs`
881–883, 935–947). So the fold must record `(uuid, tick)` before dropping. It
also lists rows that got a tail post-image or a non-zero clock projection.
`advance_replay_clocks` (987–1007) restamps every record's tick, so the tick
alone cannot identify them.

Restore seeds `changed_tick` and the removal log from those lists. It inserts
every other row as not changed (`durable_tick` = manifest tick), not as a
spawn. The first capture resolves dimension membership. The first checkpoint
after reopen then covers the tail it prunes.

## Reopen memory (the 1M peak)

`Replay.entities` (`BTreeMap<i32, ReplayEntity>`) holds the whole population
before `restore` drains it. Every `ReplayEntity` stores these inline:

- `Option<VillagerState>` (2,072 B);
- `EntityStateImage`'s rare special-mob/bee/allay options;
- `Option<ProjectileState>`.

Live `ColdRow` already boxes all of them. Box them in replay as well (in-memory
only; serde unchanged), and reserve the cold columns to the checkpoint's entity
count.

At 262k, boxing saved 613 MiB (≈2.4 KB/entity; [INFERENCE] ≈2.3 GiB at 1M).
Readiness median, however, rose by 58%, and 4/5 pairs were slower on a shared
host. Remeasure in an exclusive `-x` A/B. Accept only if readiness median is
within +10%. If it fails, any recovery redesign needs its own amendment; no
fallback is pre-accepted.

## Staged plan

1. **Replay boxing and reservation**, gated as above.
2. **C as one cutover, without a flag.** This covers the dirty rule, tick marks,
   removal log, writer tombstones and fold-time seeding. Delete the live
   full-capture path. Offline `encode_checkpoint` and Anvil export keep
   materialized capture. Update "full entity membership/tombstones" in 0013.
3. **Proof**, as below, then mark this ADR Accepted. Revisit A only if dirty
   sets from real play exceed the checkpoint budget.

## Proof

- **Oracle (small world, failing-first).**
  - At each durable fence (committed `c`), every decoded DB row must equal a
    full capture of the same state.
  - After crash/reopen, state must equal the last durable WAL fence. Only
    motion and idle counters may sit at their checkpoint values, per 0013.
  - Randomized ticks, plus deterministic visits to each window:
    - mutation after capture, before ACK;
    - capture or handoff failure;
    - commit failure with the shutdown checkpoint queued;
    - crash after commit, before ACK/prune;
    - stale ACK;
    - swap-remove and transfer of a marked slot;
    - NPC `set_uuid`;
    - checkpoint → tail removal/transfer → crash/reopen → checkpoint/prune →
      reopen;
    - inactive villager with a pending golem ttl across two checkpoints →
      reopen → exact deadline;
    - a raider leaving a raid.
  - Existing checkpoint, crash and three-dimension suites stay green.
- **Latency.** `measure_checkpoints.py`, release build, `-x` window.
  - **Held equal across arms:** allocator, CPU quota and affinity, worker count
    and fixture.
  - **Runs:** 5 AB/BA pairs × 10 checkpoints × ≥200 normal ticks, at 10k and
    100k.
  - **Profiles:** CPUs 0–3, plus an ordinary 1–2-core server.
  - **Fixtures:** frozen-distant, and all-active in ticketed chunks (ageing
    alone dirties every row).
  - **Metric:** median of per-run sampled maxima (with 10 samples, p99 = max),
    paired deltas, and the worst observed stall.
  - **Pass:** none of the three is worse than baseline, for checkpoint ticks
    and normal ticks. No population p99 is claimed from 50 samples.
- **Memory (1M, mostly inactive).**
  - **Recorded:** resident/active/dirty counts per checkpoint, allocator,
    cgroup composition, swap policy and the maximum sampling gap; keep kernel
    `memory.peak`, `memory.events` and phase samples.
  - **Run, 8 GiB scope:** admission, ≥3 checkpoints, then SIGKILL during and
    after a checkpoint. Record membership and the durable fence before each
    kill. Check the first reopen with an offline native fold before any live
    tick or clean checkpoint, then live.
  - **Pass:** membership equals the fence, sampled anon ≤ 6 GiB in every phase,
    and `max` and `oom` are 0.
  - **Limit:** C does not promise all-active 1M in 8 GiB. With dirty ≈ N its
    transient returns to D's ≈4.2 GiB.

## Rollback

The disk format and recovery are unchanged. A DB written by C is a valid full
checkpoint for the old binary, whose next full write tombstones and rewrites as
before, and the reverse also holds. Rollback is a revert and redeploy with no
migration. Replay boxing is in-memory only.

## Evidence

- `solaris-work/scenarios/rnd-memory-stream-20261103/receipts/{REPORT,DESIGN}.md`
- `solaris-work/receipts/rnd-mem-integrate/RESULT.yaml`
- `solaris-work/receipts/rnd-mem-split/{RESULT.yaml,reopen-summary.json}`
- committee review `ReviewAdr0020` (verdict amend; all findings addressed above)

The 1M and streaming runs used jemalloc. The 10k/100k A/B runs used the default
allocator on a shared noisy host.
