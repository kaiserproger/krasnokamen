# State ownership index

Status: current native runtime after ADR 0014 Phase 5. Authority is defined by
[ADR 0013](../decisions/0013-core-model-v2.md), not by the deleted regional-owner
and multi-journal designs. See [runtime.md](runtime.md) for execution boundaries
and [mutations.md](mutations.md) for routes.

| State | Authority / writer | Readers and projections | Persistence boundary |
| --- | --- | --- | --- |
| Player inventory, cursor, held item, survival and experience | `mc-core` native input/merge and gameplay modules | immutable player results; edge menu and wire caches | same tick WAL and native checkpoint as related effects |
| Player pose, combat and entity lifecycle | native core entity state and intent merge | edge tracking and per-viewer movement; never an alternate entity store | entity/player native post-images in tick WAL/checkpoint |
| Mob/player mount relation | vehicle row owns rider UUID; `mounts` rebuilds rider reverse link and passenger projection | rider/vehicle motion and edge passenger packet | `krasnokamen:Rider` in checkpoint and native WAL image; duplicate riders, self-riding and nested vehicles refuse recovery |
| Blocks, block entities and physical containers | native core block/input/merge phases | immutable chunk views, client block/menu projections, admitted plugin observations | native chunk/block-entity post-images and checkpoint fences |
| Scheduled block/fluid work | native core scheduled queues | gameplay reads and offline interchange | absolute native deadlines in WAL/checkpoint; Anvil export translates interchange delays |
| World clock, weather, rules and dimensions | native tick owner | edge time/weather packets, console/admin reads | native domain images in the same tick stream |
| Plugin KV, operation receipts and native plugin effects | native core plugin admission/merge | component callbacks and immutable observations | same native tick recovery boundary; no independent plugin state files |
| Native NPC work, custody and settlement operations | native core admitted operation paths, including [settlements](settlements.md) | guest policy receives opaque handles and outcomes | native records/effects journaled with their owning tick |
| Other plugin product policy | deployed component in its own repository | UI and guest callbacks | persisted only through admitted native plugin storage operations |
| Connection/session and Loader negotiation | `mc-edge` | routing, auth state, held wire baselines and queue credit | connection-scoped, not gameplay persistence |
| Immutable content tables and definitions | validated startup data / native registry | gameplay, worldgen, wire encoders, component declarations | external licensed content cache; world identity rejects unsupported state |
| Validation run and owned processes | canonical `tools/harness` | agents/CI inspect receipts | one `.analysis/validation/<stamp>-<profile>-<id>/result.json` plus command logs |

## Tick and wire clocks

`WorldSnapshot::tick` identifies native frames and WAL order, including held
frames. `WorldSnapshot::game_time` projects the existing
`TickControl::game_ticks` counter: frozen frames hold it, a simulated step
advances it once. Edge `SET_TIME` uses that simulated age, not the WAL frame.
`day_time` and its client rate remain independent daylight-clock projections.
The held offset is currently runtime-only: recovery restores the WAL frame but
not that offset (`TICK-FROZEN-GAME-AGE-RESTART` in the bug ledger).

### Staged battle actions

`battle/combat.rs` owns action timers and finite ammunition, `archery.rs` owns
regiment volley clocks, and `cavalry.rs` owns charge/refusal timers. Fighters
and charge state share the formation-aligned dense execution index; a cold
identity remap preserves actions and ammunition when roster layout changes.
These are runtime battle state: pause holds their clock; restart restores
bodies and mount custody, not an in-progress battle. The native projectile
owner alone moves arrows and resolves hits.

Sword/spear/mace cycles last 21/23/25 ticks, with contact at 10/11/12.
Unblocked contact attempts remove 1.3% on the authored deterministic roll,
nine wounded per death; shield parries reduce actual removals. Wounded soldiers
withdraw via formation membership rather than being killed. Pursuit increases
fatality; film pacing changes lulls, never swing speed. These are simulation
choices based on [Sabin](https://gwern.net/doc/history/2000-sabin.pdf), not
measured historical per-swing probabilities. Volleys use 220 ticks sustained
and 60 arrows per archer.
Each regiment has an absolute staggered phase, including after wavering/rejoin;
the soldiers within that regiment still draw and release together. The staged
battle uses sustained cadence; there is no unused opening-burst control.
Horses refuse steady frontal ranks and run out to a flank; accepted charge
bonus decays over 260 ticks. The work counters bound visits, target checks,
reaction checks, strikes, arc iterations and charge decisions; they are not a
20-TPS performance measurement.

Large projectile batches (at least 16 shots) derive a reusable, counting-sorted
two-block XZ grid from the frozen native body snapshot. Conservative half-extent
expansion covers wide bodies, Y bounds reject other heights, and canonical
candidate sorting preserves hit order. Dragon/deployed geometry remains an
explicit exceptional superset. Sparse world spans fall back to the existing
native chunk query before allocating empty-world cells (maximum 1,048,576).
Differential flight tests exercise both paths against exhaustive collision.

Battle arrow/spectral-arrow ground lifetime is captured at launch as 100 admitted
ticks, independent of later owner death. Ordinary arrows retain their previous
1,201-tick expiry. Settled arrows anchor to native entity lifetime age; idle
projectile ticks neither take `row_mut` nor publish appearance merely to age.
Pickup, expiry and physical-profile custody changes still use native owners.
`grounded_at_age` and `ground_lifetime` persist in projectile state, paired with
native `krasnokamen:LifetimeAge`; vanilla `life` projects the effective ground age.
Active-clock WAL replay, frozen chunks and wrapping entity age retain elapsed
time without one arrow image per tick. Loyalty tridents keep their dynamic
return path and are not treated as idle arrows.

Mob riders use the same horse-owned UUID relation as player riders. Staging
uses `battle::cavalry::mount`, requires a nonzero rider faction and copies it to
the horse so native battle motion is not suppressed as ordinary tamed AI.
Recovery allows a live mob passenger rather than requiring a player UUID,
rejects item/boat/dead/nested passengers, and rebuilds reverse links/appearance.
Native regressions include checkpoint/WAL mount custody, real mounted movement,
100-tick grounded expiry, idle no-touch pickup and native lazy-clock replay.
Protocol and persistence changes require Main's independent review.


## Rules

1. Only native gameplay writes authoritative world/player/entity state. The edge
   submits authenticated inputs and projects outcomes; it cannot repair a refusal
   by committing its own speculative candidate.
2. Intents validate their complete footprint against accumulated merge state.
   Credits, debits, lifecycle changes and dependent effects succeed together or
   are rejected together.
3. Immutable snapshots are observations, not locks or writable state. Reader
   retention never blocks tick progress.
   Entity wire views consume the store's mutation-owned candidate IDs and compare
   complete after-images before changing persistent rows. Images occupy dense
   64-row copy-on-write pages; identity-to-slot lookup is Fx-hashed and shared
   unchanged through ordinary motion. Spawn/removal copy that index at most once
   per retained publication; removed slots are reused. The separate ordered
   chunk/ID index preserves publication order and changes only on spawn, removal
   or chunk crossing. Equal candidates copy no pages and do not advance the wire
   revision. This replaces per-row persistent-tree updates, not gameplay or WAL
   ownership; revert the collection together if snapshot/order parity fails.
4. Publication is not the commit point. A lost packet does not undo an effect;
   reconnect/reconciliation uses current authority, not client speculation.
   `ViewerState::player_profiles` owns the connection's delivered profile baseline.
   Replacing a tracked player's profile removes its old wire body and retains
   spawn debt while visible, even when the core snapshot revision is unchanged.
   Queue credit may defer that replacement, but must not discard it.
5. Append completion is not fsync durability. A crash may lose an unflushed tail,
   but does not recover different tick boundaries for player and plugin/world data.
   Entity UUID tombstones are terminal. Hive entry transfers bee state into the
   block entity and retires the active body; release assigns a fresh UUID, as
   vanilla does when stripping occupant UUIDs. Retained native hive images may
   preserve gameplay state, but cannot restore a retired body's identity.
6. Plugin code owns policy, not native pointers, entity slots, region keys or
   custody authority. Revalidate live session/generation/revision at native apply.
7. Names alone are not evidence: inspect the owning symbol before changing a
   specific mutation or claiming a test covers it. Historical warehouse-pilot
   tests and old multi-journal recovery receipts do not prove native v2 behavior.

## Entity wire publication optimization

The wire projection uses dense copy-on-write pages and mutation-owned candidate
IDs instead of persistent-tree updates. Equal candidates do not advance the
revision; the ordered chunk/ID index preserves publication order. Snapshot and
ordering parity are the correctness fence. Revert the collection together if
that fence fails.

Historical isolated 100k-row microbenchmarks reported 8.65–8.92 ms versus
57.48–58.53 ms for moving-row publication; this excludes gameplay, edge and
other server work and is not a TPS claim.

`target_grid.rs` addresses repeated linear scans by building a non-authoritative
per-class nearest-target index only above its consumer and table thresholds.
Its first-minimum equivalence fence, test and linear fallback are documented in
the source. Historical release horde measurements with 10k zombies, 5k
villagers and 100 players reported entity-phase p50 74.1 to 13.5 ms; host load
varied, and this is not a whole-server throughput guarantee.

## Recurring allocator and farming projections

Villager threat and pickup projection visits entity bodies only in neighboring
chunks of resident villagers, preserving the original sensor-order tie break.
The 15-block threat and 1.425-block pickup bounds fit this neighborhood.
Golem detection gathers its 16-block neighborhood at the existing 100/200-tick
cadence after brain motion, so moving residents still see current nearby golems.
The static golem identifier is parsed once, not on idle ticks.

POI eligibility bootstraps once, then consumes the world's pushed revision and
unload events. The existing bounded FIFO scan and live claim validation remain
authoritative; direct owner fixtures must deliver revision events too.
Farming caches the active resident chunk order behind `active_revision`, prunes
evicted eligibility on that fence, and visits only set section-eligibility bits.
Block edits still invalidate section masks through each chunk's wire revision;
uncommitted overlays do not enable previously inert sections.

