# State ownership index

Status: current native runtime after ADR 0014 Phase 5. Authority is defined by
[ADR 0013](../decisions/0013-core-model-v2.md), not by the deleted regional-owner
and multi-journal designs. See [runtime.md](runtime.md) for execution boundaries
and [mutations.md](mutations.md) for routes.

| State | Authority / writer | Readers and projections | Persistence boundary |
| --- | --- | --- | --- |
| Player inventory, cursor, held item, survival and experience | `mc-core` native input/merge and gameplay modules | immutable player results; edge menu and wire caches | same tick WAL and native checkpoint as related effects |
| Player pose, combat and entity lifecycle | native core entity state and intent merge | edge tracking and per-viewer movement; never an alternate entity store | entity/player native post-images in tick WAL/checkpoint |
| Mob/player mount relation | vehicle row owns rider UUID; `mounts` rebuilds rider reverse link and passenger projection | rider/vehicle motion and edge passenger packet | `krasnokamen:Riders` in checkpoint and native WAL image; duplicate riders, self-riding and nested vehicles refuse recovery |
| Blocks, block entities and physical containers | native core block/input/merge phases | immutable chunk views, client block/menu projections, admitted plugin observations | native chunk/block-entity post-images and checkpoint fences |
| Scheduled block/fluid work | native core scheduled queues | gameplay reads and offline interchange | absolute native deadlines in WAL/checkpoint; Anvil export translates interchange delays |
| World clock, weather, rules and dimensions | native tick owner | edge time/weather packets, console/admin reads | native domain images in the same tick stream |
| Plugin KV, operation receipts and native plugin effects | native core plugin admission/merge | component callbacks and immutable observations | same native tick recovery boundary; no independent plugin state files |
| Native NPC work, custody and settlement operations | native core admitted operation paths, including [settlements](settlements.md) | guest policy receives opaque handles and outcomes | native records/effects journaled with their owning tick |
| Starter storage chest grants | global tick-owned `settlements/starter_chest.rs` granted counts; moved with global custody between dimension phases, never reset by town removal | founding admission compares lifetime grants with current owned towns; the per-dimension ownership view is derived and never persisted | `settlement_chest/<uuid>` native row and absolute root `StarterChestGranted` WAL image, in the same batch as the chest's dimension-wrapped block/container effects |
| Settlement migrant in transit (household or lone resident between two settlements) | `settlements/transit.rs` ledger, one record per unit ([ADR 0023](../decisions/0023-offscreen-settlement-simulation.md)); no world entity exists for a member while its record does | `settlement info` and policy read `inbound`/`summary`; the migration policy receives the `Arrival`s `transit::tick` returns in the same tick; offline census and crash-copy custody census | `transit/<uuid>` checkpoint row and `Transit` tick-WAL image; recovery refuses a UUID held both as an entity and in a record |
| Other plugin product policy | deployed component in its own repository | UI and guest callbacks | persisted only through admitted native plugin storage operations |
| Connection/session and Loader negotiation | `mc-edge` | routing, auth state, held wire baselines and queue credit | connection-scoped, not gameplay persistence |
| Immutable content tables and definitions | validated startup data / native registry | gameplay, worldgen, wire encoders, component declarations | external licensed content cache; world identity rejects unsupported state |
| Selected datapack container loot | datapack worker compiles; native tick installs a registry snapshot | first-open container owner, loot commands, chunk readers | selected pack files plus native unresolved table/seed or rolled inventory; never roll-time JSON/filesystem access |
| Validation run and owned processes | canonical `tools/harness` | agents/CI inspect receipts | one `.analysis/validation/<stamp>-<profile>-<id>/result.json` plus command logs |

Container loot is reloadable `CoreRegistry` content, not a mutable member of its
shared startup tables. The worker layers namespaced chest/generic tables,
recompiles references after filters/overrides and refuses unsupported functions,
conditions or missing references before publication. Both legacy
`functions`/`function` and carried `modifier`/`type` are accepted, never both
spellings at once. A saved table missing at materialization (for example after
pack removal) follows vanilla's `LootTable.EMPTY` lookup: block containers and
vehicles warn once with the table ID, consume the reference and generate no
items. Vehicle open, hopper transfer and death share that rule; destruction is
not refused. Seed metadata is omitted with the consumed reference. First-open
rolling still uses the existing native block-entity/WAL owner. This does not
claim whole-mod or modpack compatibility.

## Tick and wire clocks

`WorldSnapshot::tick` identifies native frames and WAL order, including held
frames. `WorldSnapshot::game_time` projects the existing
`TickControl::game_ticks` counter: frozen frames hold it, a simulated step
advances it once. Edge `SET_TIME` uses that simulated age, not the WAL frame.
`day_time` and its client rate remain independent daylight-clock projections.
The climate post-image persists the simulated clock as the quantity its mode
keeps constant: the held-tick count while frames simulate, the game age while
they are held. Held also retains `held_since`, the first native output tick of
the uninterrupted hold; its last running boundary is `held_since - 1`.
Both values stay constant throughout that hold, so the WAL needs no entry per
frame. Restore preserves this image even before the first tick; freeze and
step requests themselves stay session-only, as in vanilla. A legacy Held
without the boundary refuses recovery rather than guessing it. Older images
without a clock recover with their frame as age.

Recovery builds one global piecewise running prefix `R(t) = t - H(t)` from
unrouted Overworld climate images before folding any dimension's entities.
Active chunk intervals use that prefix uniformly for all compact entity paths
and final projection. Villager age and conversion countdowns use running elapsed;
fresh Work/sleep anchors bridge to the last running native boundary, while
stale anchors and expired absolute panic deadlines remain stale/expired.
The prefix survives streamed folds and repeated projections until recovery
ends. Ambiguous same-mode counters or contradictory native boundaries fail
closed. Daylight time and native absolute deadlines are not retimed.

Catch-up debt stores a native anchor `L` and the held prefix `A = H(L)`.
Activation at output `T` owes `max(0, (T - 1 - L) - (H(T - 1) - A))`, excluding
both held frames and activation itself. Live, block-only and cancellation
writes advance both anchors together. Active recovery/checkpoint images use
one global recovered prefix for every dimension; inactive anchors and
outstanding `remaining` debt do not project. Only running ticks spend debt.
Accepted external edits and item births still cancel old debt while frozen.
Native extras retain the optional `KrasnokamenSimHeldTicks` Long, including
zero, and newer manifest/WAL authority supersedes older native extras.

Climate also retains a constant `sim_debt_anchored_since` tick. An unflagged
legacy running world with `H = 0` upgrades after its entire old tail is folded;
the first new output tick starts the epoch. Missing older anchors mean zero
exactly. Unflagged held/nonzero-prefix worlds refuse ordinary recovery, as do
anchorless WAL images at or after the epoch or a writer removing the epoch.
No ordinary open scans cold chunks or guesses a held prefix. Historical
pre-c9c13b37 restarts that reset `H` can already have overstated running time;
the exact upgrade does not repair that history or already-accrued legacy debt.
For unchanged running clock/debt state, capture/load/capture preserves the
stored bytes. Owned metadata updates preserve field positions; online-player
to offline-player session-clock normalization is a distinct state change.

### Mob death and corpses

`tick/damage.rs` commits a mob's death once: `alive = false`, entity event 3,
loot, equipment and experience (vanilla `die` drops everything then). A living
mob then stays an inert corpse row: `EntityStore::begin_dying` starts its clock
and `tick/dying.rs` alone moves it (gravity and lethal knockback, no AI). After
20 simulated ticks it broadcasts entity event 60 and removes the row. Gameplay
consumers treat it as dead through `alive` (no damage, interaction, targeting,
pushing, pickup, mob caps, special-mob AI or battle fighting); battle status
counts it as dead. Its published health is 0 (vanilla `setHealth` clamps).
Players keep their own death and respawn flow; vehicles, frames, paintings and
armor stands leave the store at once. Differences from vanilla:

- Slime splits and the oozing, weaving and wind-charged death effects happen
  at death, with the durable removal; vanilla runs them when the body is
  removed (`LivingEntity.remove`).
- Riders leave at death: a dying mount dismounts its rider
  (`mounts::death`, called from the death commit) and `mounts::tick` unlinks a
  dead rider. Vanilla keeps them seated until the body is removed
  (`Entity.baseTick` stops riding a removed vehicle; `setRemoved`).
- The corpse clock runs every simulated tick whatever the chunk's activity, so
  a corpse always leaves after 20 ticks. Vanilla `deathTime` advances only
  while the entity ticks.
- A bee that stung dies without a corpse: `bees.rs` marks it dead after 1200
  ticks and removes the row with no death event or loot. Vanilla `Bee` kills
  itself through `hurtServer(generic, getHealth())`, an ordinary death.

Persistence: [corpses](persistence.md#corpses).

### Staged battle actions

`battle/combat.rs` owns action timers and finite ammunition, `archery.rs` owns
regiment volley clocks, and `cavalry.rs` owns charge/refusal timers. Fighters
and charge state share the formation-aligned dense execution index; a cold
identity remap preserves actions and ammunition when roster layout changes.
These are runtime battle state: pause holds their clock; restart restores
bodies and mount custody, not an in-progress battle. The native projectile
owner alone moves arrows and resolves hits.
The shield's raised/lowered pose is different: it is native `appearance.extra`
metadata, persisted in WAL post-images and checkpoints. The combat runtime
owns decisions, while the shared changed-only metadata setter owns presentation
mutation; unchanged guard never copies or republishes appearance. Staging
pre-raises equipped shields so the first running tick makes no mass pose write.
The sole kernel tree is `mc-core/src/battle`, restored from the F battle
line; `hostile` retains its runtime state, not another implementation.

Sword/spear/mace cycles last 21/23/25 ticks, with contact at 10/11/12.
An unparried blow removes about eight times in a thousand on the authored
deterministic roll; seven removals in ten die on the spot and the rest
withdraw wounded through the ranks. Any landed blow finishes a wounded man, and
pursuit removes nearly every landed blow, nine in ten fatally. Front ranks
surge in section pulses whose fatigue follows the share of the front pulsing;
outside a pulse a front-rank man still trades blows at the cautious rate with
an enemy already in weapon reach. Riders and pursuers fight from any rank, and
fugitives pinned against the battle bounds turn at bay; a fugitive stays where
it met an edge instead of running on along it into a corner. A regiment worn near
its pulse floor while in contact erodes until it breaks; one whose enemy broke
steadies. Film pacing changes lulls, never swing speed. These are simulation
choices based on
[Sabin](https://gwern.net/doc/history/2000-sabin.pdf), not measured historical
per-swing probabilities. Volleys use 220 ticks sustained and 60 arrows per
archer; about one arrow in seventy finds an armour gap and kills, the rest are
one-HP hurts.
Each regiment has an absolute staggered phase, including after wavering/rejoin;
the soldiers within that regiment still draw and release together. The staged
battle uses sustained cadence; there is no unused opening-burst control.
Horses refuse steady frontal foot ranks and run out to a flank; accepted charge
bonus decays over 260 ticks. Once their own foot closes, unengaged riders swing
round the nearest enemy foot regiment into its rear, and ride down the
fugitives of the nearest broken foot regiment until those reach the edge of
the field. A rider who fled to the edge rallies there and rides back until he
falls, even when his regiment has broken; foot fugitives stay at bay on the
edge. A regiment rides these sweeps as
a block, never toward one shared point: each rider follows the sweep from its
own file/rank slot. Targets and swings are laid round the regiments' live
centroids, not their virtual leaders, which march ahead of files lagging on
relief. A rider attacking closes on its own
place round its target (golden-angle rings 1.2-3.6 blocks out), so riders that
chose the same foe do not pile onto one block. The work counters bound visits,
target checks, reaction checks, strikes, arc iterations and charge decisions;
they are not a 20-TPS performance measurement.
Mounted terrain turns are retained for up to 60 ticks, including while pursuing a
target or formation slot. A side slide must retain the headroom required by
steps in its projected direction; otherwise steering weighs another direction
instead of pressing into a canopy-blocked step. The bounded detour prevents
the slot from pulling the horse back into the same obstacle on its next tick.
The ten-second stall census runs after target validation. A soldier holding a
live enemy within six blocks is in contact, not advancing, even when its virtual
leader is farther ahead. An invalid retained target cannot hide a blocked soldier;
the census is derived telemetry and never changes motion.

A new `battle live` hold replaces the holder's entire inventory with a fresh
copy of his kit. It retires plugin reservations before changing stock, clears
the old inventory, and routes the close through the ordinary
native/plugin/settlement menu owners. The cursor and crafting grid fit in that
emptied inventory, then are discarded with all non-kit contents; no overflow
item entities are minted. Menu ownership and the client's open window close
together. A held player's respawn, reconnect (once per connection) or repeated
`battle live` re-arms him under the same custody, but only with the kit pieces
he still carries (matched by every identity field except count and wear, wear
kept): a piece he dropped or stored is never issued a second time.

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
Hydration refuses an anchored projectile without a native lifetime-age long;
retained lifetimes accept only 1,201, or 100 for Arrow/SpectralArrow.
Block impact establishes the native-age anchor. Loss of support exits the idle
path and clears both the anchor and ground age before falling; a later landing
starts a fresh ground lifetime, including across checkpoint/WAL recovery.
Active-clock WAL replay, frozen chunks and wrapping entity age retain elapsed
time without one arrow image per tick. Loyalty tridents keep their dynamic
return path and are not treated as idle arrows.

Native mob NBT writes `krasnokamen:Faction` exactly once for a nonzero faction;
ordinary faction-zero mobs carry no faction key.

Mob riders use the same horse-owned UUID relation as player riders. Staging
uses `battle::cavalry::mount`, requires a nonzero rider faction and copies it to
the horse so native battle motion is not suppressed as ordinary tamed AI.
Recovery allows a live mob passenger rather than requiring a player UUID,
rejects item/boat/dead/nested passengers, and rebuilds reverse links/appearance.
Native regressions include checkpoint/WAL mount custody, real mounted movement,
100-tick grounded expiry, idle no-touch pickup, native lazy-clock replay, and
matching uninterrupted/restarted expiry through actual checkpoint and WAL producers.
Protocol and persistence changes require Main's independent review.

### Battle factions on the wire

A mob's side is its persisted `Body::faction`; a live player's side is his
`battle live` hold (runtime only, his own faction stays 0). The snapshot
publishes both: `EntityView::faction` and the derived
`WorldSnapshot::player_factions` (same `Arc` while the holds are unchanged).
The edge (`mc-edge/src/faction_teams.rs`) projects them per connection into
vanilla scoreboard teams `sf <faction>` (mobs by UUID string, players by name;
nametag never, collision always, red/blue colours) that exist only on the
client: no core team, no persistence, no gameplay effect. A team leaves when
the snapshot holds no row and no player of its faction; respawns keep the
teams and a dimension change syncs them again. Each connection keeps the
entries it sent (mob UUIDs, player names) as its baseline. Entries whose
faction ends remain in that team until the whole team is removed; a change
to another nonzero faction moves the entry by an add. Projection packets
use only create (0), remove-team (1), and add (3), never remove-entry (4).
A routed core `/team` add implicitly moves an entry and the edge forgets
that projected membership without sending a removal. Holds end only by
`battle reset` or a restart (`spectate` and going offline keep them).
`battle reset` removes staged rows, but summoned faction rows keep their team.
An ex-holder keeps the projected team's hidden nameplate and team colour while
that team survives a reset, until the team leaves.
Nameplates are always hidden for projected teams, including with hidden HUD;
options enable friendly fire but not see-friendly-invisibles.
No `/team` command can name a team
`sf <n>` (team names are one token); a core team of exactly that name
(`WorldSnapshot::team_names`) blocks the faction. Because an add moves a
client's entry and a removal from the wrong team disconnects a vanilla client,
an entry gets a faction only while it is in no `/team` team
(`scoreboard::require_no_team`: summon with `UUID`, battle mount, `battle
live`), and `/team join` refuses battle-faction UUIDs and holders.

Allegiance is defined once by `battle::faction::relation`: two nonzero equal
factions are Same, two nonzero different factions are Enemy, and either zero
is Neutral. Melee selection uses Enemy, friendly support uses Same, and
arrows collide with everything except Same. Separately, `battle_row(faction)`
marks native roster/AI ownership (currently nonzero). Ordinary AI suppression,
battle columns, sparse roster indexing and battle arrow lifetime use this
marker, not an enemy predicate. Neither changes the persisted faction format.


## Spectator cameras

Spectator cameras are transient native bindings, published with the immutable
world snapshot. `/spectate` and battle named-follow use that same authority;
the native tick follows the target position for residency and ignores viewer
movement while bound. Binding changes synchronize the viewer once, not on every
follow tick. Death or disconnect releases the current camera; only battle's
named intent retries a returning target or a respawned viewer on the same
connection. Explicit stop, sneak, viewer disconnect or leaving spectator mode
clears that intent. Entity-ID bindings are transient; battle history persists
only the named follow intent and resolves it through this same authority.
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
The snapshot shares the global roster allocation for current player modes.
Entity interest uses that roster and camera ownership for vanilla player
tracking eligibility, not a second mode store: ordinary viewers never receive
spectator bodies; spectator viewers receive only self-camera player bodies.
Retained interest revisits player rows when either immutable input changes,
including already paired and previously hidden players, without scanning mobs.

## Mount custody

Mount custody belongs to the vehicle's ordered, bounded rider UUID list. The
first rider with a live body controls motion; absent recovery claims are skipped
and reserve no attachment position for live passengers. A passenger's reverse
link and published passenger entity IDs are derived, never separate
authorities. Removing one passenger preserves the other seats. Native images
retain the list; offline animal NBT uses `krasnokamen:Riders`. The shared image
reader validates duplicate UUIDs and the vehicle's seat capacity; replay also
rejects one UUID claimed by multiple vehicles. Before adding a rider, live Ride
removes that UUID from every other vehicle in the dimension and removes absent
claims (and an existing claim for that actor) from its target. Only the camel
(3, `Camel.canAddPassenger`) and happy ghast (4) seat more than one rider. The
earlier scalar `krasnokamen:Rider` NBT key and WAL `rider` field are refused
outright, so an old image fails the load instead of silently unseating its rider.

A recovered absent UUID is a pending claim, not an occupied seat: it survives
idle ticks until its passenger reconnects or another live Ride takes that
vehicle. No rider kind is persisted, so an absent mob UUID follows the same rule.
`mounts::live_riders` resolves those UUIDs against live bodies and is the single
gameplay occupancy source for control, AI, leash follow, breeding, despawn,
portals and battle projections; there is no cached mounted flag to go stale, and
the durable rider index intentionally keeps absent claims. Explicit
leave/dismount, death, or a known dead or invalid passenger clears its relation.
Plain Join and successful plugin-login unlock both call `mounts::join`; the
active vehicle tick also rebuilds the reverse link and passenger projection.
Re-seating has no distance or chunk-activity check within the dimension:
bringing the returning player to the vehicle is accepted policy, not vanilla
parity. Dimension travel ejects seats and ends pending absent claims but keeps
the vehicle's chest and cargo; a later claimant joins at their saved position.
Death ejects seats and drops that custody. WAL recovery regression:
`happy_ghast_admission_leave_and_reconnect_survive_wal_recovery`
(`crates/mc-core/src/mounts_persistence_tests.rs`); native tests also cover
reclaimed seats, first-live steering, plugin unlock, competing claims and a
clean `Core::shutdown` with a still-seated player.

## Container openers

Each dimension's `redstone::custody::OpenContainers` owns menu and temporary
native openers. Its per-cell counts are derived from those records, including
both halves of a double chest; copper weathering and trapped-chest power read
the same counts. Menu projections enumerate only menu viewers, never native
openers. Removing an entity or leaving a dimension removes its membership.

An external custody change (block log, `/loot`, plugin inventory writes) republishes
a menu to its viewers only when the retained menu shape matches the reconstructed
storage and every cell is resident. A cold half — whichever one — closes that
viewer's menu instead of imaging its real slots beside empty ones, and the close is
followed by a window-0 resync so predicted stack and cursor edits roll back. The
click, recipe, refusal and refresh paths share this check; a viewer anchored on a
half that has already gone cold is not refreshed (no partial image is sent) and its
menu closes on the first click.

Copper golems retain the existing native item-transfer path and dwell with the
container open for 60 simulated ticks. Freeze holds that deadline. Expiry runs
before random block ticks; a single commit-tail emitter publishes final ordinary,
trapped and copper chest/shulker lid transitions after menu closures and removals.
It reuses dirty/recipient scratch and the existing block-event encoder. Opener
membership is transient: checkpoint/WAL item ownership remains in native container
slots and ordinary entity equipment, not a second persisted opener registry.

## Container checkpoint capture

The native checkpoint preflight materializes and hydrates container custody
before freezing live slots. Each dimension's chunk capture then builds one
borrowed chunk index from **all** frozen live owners, including clean neighbours
in a dirty chunk. Euclidean division on both horizontal coordinates preserves
negative chunk boundaries. Buckets retain the ordered input's record order;
they neither clone inventories nor become another authoritative owner.

Healthy resident selection/projection visits the live keys once, looks up each
dirty resident chunk once, and projects only matching owners: `O(B + D + M)`,
not one full live-map walk per dirty chunk. Empty buckets still remove stale
typed chest/furnace images; unopened loot without a live owner is retained.
Opaque native owners keep their bytes and are never retyped by slot count.
The conservative quarantined-chunk refusal path is unchanged.

Capture still uses native copy-on-write chunks and the existing revision ACK.
Only the writer's WAL-gated commit may advance the manifest and prune covered
frames; stale ACKs cannot erase newer inventory custody. Player and container
post-images retain that same durable fence in Overworld, Nether and End.

Measured effect (one BASE/AFTER pair, release-test pins `53e3a2cae5f7-d3518158`
and `53e3a2cae5f7-3ecc759c`, medians of three samples, chunk capture only, not
the writer, fsync or end-to-end checkpoint latency): 2,000 dirty chunks with
10,000 disjoint live owners took 21.759 ms before and 0.130 ms grouped; with no
live owners the grouped path was 0.008 ms slower, within noise.

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
   Replacing a tracked player's profile retains removal and spawn debt while
   visible, even when the core snapshot revision is unchanged. Camera release,
   body removal, pairing and camera rebind share the world FIFO in that order;
   reliable roster delivery must not overtake camera release with a removal.
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
8. Every cross-thread queue is bounded in bytes or items, or it has a written
   reason and a counter. The tick→edge reliable handoff is the one the audit
   caught unbounded: it now charges `outbox_queue_bytes` (256 MiB; two documented
   exceptions — an oversized batch alone in an empty queue, and the halt's
   terminal `Disconnect` batch), warns once per 8 MiB crossing (re-armed below
   4 MiB), and a non-empty queue that consumes no batch for 10 s halts the tick
   fail-closed. A refused push never blocks and never drops a reliable message
   (`crates/mc-core/src/core/outbox.rs`,
   [persistence](persistence.md#hazards-and-defense-layers)).
   The same rule's other channels are bounded elsewhere or by construction: the
   journal work queue by `JOURNAL_QUEUE_BYTES` and 128 slots, the per-connection
   reliable queue by 512 frames, and the handle's request channels
   (`checkpoint_tx`, `backup_tx`) carry one request per caller that is blocked on
   its reply, so their depth is the number of concurrent waiting callers.

## Entity wire publication optimization

Effect of the dense pages of rule 3: historical isolated 100k-row
microbenchmarks reported 8.65–8.92 ms versus 57.48–58.53 ms for moving-row
publication; this excludes gameplay, edge and other server work and is not a
TPS claim. The fence is snapshot and ordering parity (rule 3).

`target_grid.rs` addresses repeated linear scans by building a non-authoritative
per-class nearest-target index only above its consumer and table thresholds.
Its first-minimum equivalence fence, test and linear fallback are documented in
the source. Historical release horde measurements with 10k zombies, 5k
villagers and 100 players reported entity-phase p50 74.1 to 13.5 ms; host load
varied, and this is not a whole-server throughput guarantee.

Ordinary melee hostile target memory belongs to `hostile.targets`, not the
battle roster. The zombie/shared melee blueprint does not schedule fire panic:
vanilla `Zombie.registerGoals`/`addBehaviourGoals` has no `PanicGoal`. Daylight
burning must leave normal survival-player acquisition and pursuit eligible;
animal panic and explicitly authored fire-fleeing goals remain independent.

`MC_CORE_PHASE_LOG` diagnostics use the tick's `Laps` collector for both
per-callee laps and already measured phase/journal/contact spans. Recording a
known span does not advance a lap boundary. Disabled collectors allocate
nothing; the `phase ms:`, `tick outer ms:` and `journal build ms:` summary
prefixes and field units remain compatible with the coreperf log reader.
These opt-in log records do not replace authoritative `TickTimes` telemetry.

Battle staging resolves its vanilla item carriers and husk/horse type IDs once
in the stable registry tables. Loadout component variants still use the canonical
item codec; resolved equipment enters the summon image before normal attribute
and wire-appearance restoration. No per-fighter item-name NBT round trip or
entity-name registry lookup is needed, and no combinatorial loadout cache is kept.

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

