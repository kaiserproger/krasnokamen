# Native settlement architecture

Settlements are native `mc-core` tick authority, not a first-party plugin
simulation. Player-facing commands and behaviour are in the
[settlements guide](../SETTLEMENTS_GUIDE.md). The plugin API exposes bounded
queries and requests; it does not own the native resident or physical inventory.

## Code map

All paths below are under `crates/mc-core/src/settlements/` unless noted:

- `registry.rs`, `mod.rs`: settlement records, identities and binding.
- `allocator.rs`: native resident home, job-site and meeting-point allocation;
  `crates/mc-core/src/villager/` owns resident brains and state.
- `buildings.rs`, `construction.rs`, `autonomy.rs`, `blueprints.rs`: certified
  buildings, construction and generated-village adoption.
- `construction_access.rs`: the construction module's narrow surface (owner
  D9.3). Code outside `construction*.rs` reads the record's projects and
  pending orders only through it; the `code-health` ratchet counts direct field
  accesses and its ceiling is zero.
- `households.rs`, `districts.rs`, `production.rs`, `economy.rs`, `commerce.rs`,
  `caravan.rs`, `army.rs`: native domain policy and execution.
- `inventory.rs`: physical container custody; `commands.rs`, `ui.rs` and
  `ui_menus.rs`: native command and presentation entry points.
- `persistence.rs`: record encoding and validation;
  `crates/mc-core/src/plugin_settlements.rs`: plugin request/response admission.

Each dimension owns its settlement records; a global monotonic sequence allocates
IDs, which are not reused when a record is deleted. A villager's native
`VillagerState::settlement` is membership authority, not a duplicate roster.
Binding uses the existing entity, not a replacement plugin NPC. Housing
allocation uses resident POIs and shares the native tick decision/path budget
with villager brains; denied work does not authorize loading arbitrary chunks.
Certified building work points own the corresponding job slots.

Construction checks the remaining active-project bill against physical storage
and exact Builder carry. If extraction is needed, the allocator staffs the
missing role; when the only suitable worker was borrowed by that project, the
worker returns to extraction without cancelling the structure, and is not
borrowed again until its remaining bill is funded. Lumber search for an active
wood shortage follows the project's selected wood family. Partial loads return
through the ordinary storage transaction when they can fund the remaining work.
After footprint interference the native structure executor derives a bounded
repair stage from completed build cells that a foreign edit emptied (air or a
clearable plant where the blueprint has a block); a solid foreign block in a
placed cell keeps its place. It persists that stage in the existing structure
ledger; repairs use the same material custody and one-cell transaction as
ordinary work, and completion reconciles the world before issuing a
certificate.

Building perimeter goals use real supported terrain and must put the next cell
within the existing 12-block placement reach, with room for navigation's arrival
tolerance. The bounded perimeter search can choose another side of a hillside
site; a fixed blueprint-height point over a lower slope is not a standing place.
Builders still stay outside the future footprint and pay for each placed cell.

Street and workstation construction are temporary Builder employment. Completing
that infrastructure releases its Builder, so production's live free-resident list
can change between dialogs. Opening or refusing a production plan does not own a
worker; admission owns the selected resident for the unfinished order.

Breeding-ration provisioning retains its own transient storage destination once
a resident finds a reachable spare bed. Arrival completes that food transfer
even when the bed is outside storage's local POI scan; ordinary worker hauling
does not become a ration trip. This intent is not persisted and grants neither
a bed claim nor a birth: those still require the allocator and birth checks.
Awake residents check nearby door approaches during admitted brain decisions.
Nonpersistent leaves at foot or head height can be cleared with their vanilla
loot credited to physical settlement storage; retained player edits and
persistent leaves are protected. Clearing waits if loot cannot be stored.
Logs are not cleared by this maintenance. Villagers embedded in collision
geometry recover to the nearest supported, clear position within two blocks;
sleeping residents are excluded, no position is chosen in unloaded space, and a
resident whose eye cell is a suffocating block is not moved (vanilla
suffocation applies there).

Wage accrual (`commerce`) and caravan expiry (`caravan`) use simulated game
ticks, so `/tick freeze` and server downtime add no elapsed periods. Payroll
keeps one persisted `next_due` for both simulated and unsimulated settlements;
caravans keep one `game_deadline`, while `epoch` remains immutable receipt
identity. Recovery converts legacy world-tick clocks once, after WAL folding
and restoration of the global game clock. Future deadlines preserve their
remaining duration; overdue wages preserve the old clock's complete-period
debt in a one-time `legacy_owed` amount accrued at the first payroll-cursor visit
(at most 16 contracts per settlement per tick), paid when budget and treasury
allow. The next due phase is translated to game time; recovery never pays money.
Already-expired caravans expire on the first simulation tick.
Historical freeze periods cannot be reconstructed, so valid legacy debt is not
forgiven. Only inconsistent future clocks or arithmetic overflow refuse startup.
Persisted clock discriminators and deadlines travel in the normal settlement
WAL/checkpoint images; missing discriminators mean legacy units, and unsupported
units or mixed caravan deadline schemas fail closed. Downgrade requires the
pre-upgrade backup. No residency transition resets either clock.
Remove `Clock::WorldTicks`, the recovery converters and `legacy_owed` only after
every supported live store has a game-clock checkpoint with no pending
`legacy_owed`, and its retained WAL/restore backups no longer require legacy
recovery. Archives that still need conversion must first be upgraded with this
version; removing the shim must reject them explicitly rather than reinterpret
their timestamps.

Production and daily consumption share their respective persisted game-time
watermarks across simulation transitions. New job periods have a deterministic
settlement-ID phase. A station outside `is_active` accrues deferred portions,
even when resident; it never changes carry, equipment, blocks or containers.
The first active owner visit reconciles those portions through the ordinary
recipe planner and fenced physical storage commit. All declared storage chunks
must be resident; otherwise reconciliation waits. Once resident, it applies only
affordable portions with output space and drops the rest in that visit. Navigation
and delivery retries are runtime-only and cannot authorize a portion.
Simulated fetch/walk/park visits keep a due opportunity until a portion is
actually produced. A persisted `waiting_simulated` guard caps that held span at
one opportunity at the first unsimulated visit; subsequent off-screen periods
then accrue normally. Leaving a blocked station cannot turn its on-screen wait
into a batch. This conservative unload boundary can forgo off-screen production
for up to one retry window (`RETRY_TICKS`, 20 game ticks), plus any `WorkCursor`
dispatch delay (16 jobs per tick); it never credits a period twice. For a period
of `d` ticks, the retry-only loss is at most `ceil(RETRY_TICKS / d)` portions,
including phase rounding. The runtime retry timestamp is not the unload time
and is never used to reconstruct it after restart. Deferred and unavailable-storage
retries wait 20 game ticks; new due periods are retained during that retry wait.
Legacy production deadlines translate remaining world duration once after WAL recovery;
an overdue legacy job retains just one opportunity, not historical catch-up.
Missing deferred fields default to zero. Remove the production legacy converter
only after supported stores and their retained restore backups have been upgraded.

Daily food consumption is new gameplay, owned by production's consumption module
for **all** settlements, including those with no jobs: one food point per living
member per 24,000 game ticks (`DAILY_FOOD_POINTS`). Existing villager food values
apply (bread four, vegetables one).
An absent consumption watermark (including a restored record without one) is
initialized lazily on its first economy tick and journaled once with the normal
record revision increment. Subsequent idle ticks emit no settlement image.
The first daily charge waits at least one complete day before its settlement
phase (strictly less than two days). Recovery rejects deadlines at least two days
ahead, credit above three points, debit beyond the exact native/script counter
limit, and legacy production deadlines more than one job duration ahead.
The same daily watermark debits simulated settlements physically and unsimulated
settlements abstractly. On the first
active visit with resident storage it consumes real unreserved food, drops any
unfunded demand, and carries only prepaid excess from the final whole item to the
next day. This credit is less than one item's food value, never an unpaid hunger
backlog. The anchor selects simulation for this town-wide demand. Freeze and
downtime accrue nothing. Debit, credit, watermark and inventory post-images share
the existing settlement/chunk tick-WAL and checkpoint boundary.

## Durability and plugin boundary

Touched settlement post-images and deletions share the dimension-wrapped tick
WAL. Checkpoints store individual `settlement/<id>` rows and the shared
`settlement_sequence` high-water row, not a single unbounded registry blob; the
checkpoint that no longer carries a removed settlement deletes its row in the
same native transaction.
The villager membership column is persisted with the entity in the same
recovery boundary. Recovery validates settlement references instead of silently
clearing them; unsupported old plugin settlement state does not become a
second native authority. Building state lives in its settlement record.

Migration transit (`settlements/transit.rs`, [ADR 0023](../decisions/0023-offscreen-settlement-simulation.md))
holds a departing household or lone resident between two settlements of one
dimension. Departure removes the bodies and inserts one `transit/<uuid>` record
(keyed by the lowest member UUID) in the same tick; arrival restores the same
entities at its simulated edge, or at the nearest standing cell near its anchor
when the return bearing has no entry, joins them and deletes the record in the
same tick. The record keeps each member's complete WAL entity image and
the household record `leave` removed; member images name no settlement or
household of their own. Records ride the tick WAL as `Transit` entries and the
checkpoint as `transit/<uuid>` global rows; a checkpoint deletes the rows the WAL
deleted in the same transaction as the entity tombstones. Recovery refuses a
store in which a member UUID is both a world entity and in a record, in two
records, or whose record household also belongs to a settlement; the offline
census applies the same exactly-once check. A unit is refused before any
mutation while a member has construction work, wages owed or another custody,
so the stored image is exactly the body that left. Older binaries refuse a store
with transit rows (unknown global domain); downgrade is unsupported.

`crates/mc-script/wit/plugin.wit` defines `query-settlement` and
`request-settlement-action`; `plugin_settlements.rs` routes the requests to the
native owner. Guests receive handles and outcomes rather than direct access to
records or entity slots. Native admission revalidates identities and custody.
The shared [persistence contract](persistence.md) governs replay and backups.

## Migration policy and physical return

`settlements/migration.rs` evaluates each simulated settlement once per game
day, at a stable settlement-id phase of `game_time % 24000`. There is no
catch-up; changing daylight time does not trigger evaluation. Its runtime
observation contains physical beds in certified houses, body-backed members,
inbound transit members, storage food, vacant certified jobs, resident deaths
over seven game days and completed buildings. Attractiveness is
`100 × (0.30 housing + 0.30 food + 0.15 jobs + 0.15 safety + 0.10 development)`;
components clamp to `[0,1]`. Housing subtracts members and inbound migrants,
food divides by twelve food points per resident, jobs and deaths by population,
and development by twelve buildings (zero population uses one).

A destination must beat the source by more than ten points after a cost of
five points per hundred horizontal blocks. The outside world scores 35.
Unsimulated destinations need a cached observation from this runtime. Units
are whole households or household-less individuals. Founding families and
military, conversion, economic or construction reservations refuse before
mutation. Destination beds cover the whole unit, including inbound transit
and pending walks. Those are free beds: certified-house beds less one for
each member who sleeps in one, has no home or has no loaded body. Members
sleeping in uncertified generated houses (savanna houses fail House
certification) do not use them. One unit per source may leave daily; only
one outbound walk may be active. Proven loaded routes relocate membership immediately;
otherwise bodies walk to a route-proven simulated edge before transit takes
custody. The civil radius does not constrain that edge. Outside emigration
rechecks empty carry/equipment at departure and leaves no transit record.

Outside immigration creates ordinary adult villagers only at a route-proven
simulated entry. The shared bind admission omits presence only for this
internal path; manual bind still requires the civil sphere. Newcomers are
members from their spawn tick and have no household. A birth between two
household-less adults adopts them into a new household with the child. A birth
between a household-less adult and a member of a household of three or more
founds a new `split` household instead: that member leaves the old household
(ordinary detach) and the parents form one with the child. Smaller households
still take the co-parent and child, and parents from different households
never breed. The split refuses before food, spawn or membership change while
the member's household leads a founding expedition or either parent walks a
migration exit. Daily arrivals have expectation
`(1 + min(0.5, (gain - 10) / 100)) × migration_rate`, capped by free beds.
Owner chat and structured INFO events report arrivals, departures and
cancellations. `settlement info` displays observations, components, reasons
and inbound/outbound transit.

`villager/return_home.rs` is a weak-goal fallback, not a work order. It uses
the allocator's completed empty Home search and shared bed eligibility,
choosing the nearest certified house's free native bed. Only outside the
civil sphere may it fall back to an anchor approach when no bed is known.
Homes, panic and useful native worker, army, economy, food and partner targets
win; explicit outbound migration travel also wins. Arrival within 32 blocks
wakes one actual allocator search and holds position until that result.
Useful-target preemption delays a fresh return search for at least 40 game
ticks without shortening an existing failure retry.

Return routes reuse the native router, shared Brain budget and committed
movement: supported simulated legs of at most 64 horizontal blocks, a resident
one-block halo, and a 1024-node ceiling. Truncation and stalled/no-waypoint
segments count as failures; admission denial does not. Three failures outside
enter runtime-only Hold, with one INFO reason; inside residents return to
normal work with backoff. Membership/anchor/radius changes reset an episode;
eight-block displacement never bypasses its deadline. **Main decision,
2026-11-17:** every failed attempt uses the same `1200 + entity-id % 40`
retry cadence. The reviewed design's 200-tick initial retries were dropped:
because Hold is not persisted, restored homeless bodies cannot be distinguished
from new returners. Successful legs still continue immediately. No return
intent or retry bookkeeping enters native entity rows or WAL images.
Outbound walk timeouts and return retry/watchdog deadlines use simulated game
time: `/tick freeze` neither expires a walk nor rejects a returner's bed.
Removing a settlement clears its runtime observations and return episodes,
and cancels outbound walks both from it and toward it, including their
per-resident targets. These derived maps have no checkpoint image.


## Spanish teaser culture and chronicle

The teaser's faction is the existing settlement owner UUID (Main decision,
2026-11-15), not a new diplomatic faction type. A completed native
`spanish_house`, `spanish_church`, `spanish_plaza`, `spanish_fortress` or
`spanish_wall` establishes the owner's style through committed structure-ledger
provenance. Every settlement with that owner subsequently selects the Spanish
variant for the corresponding building kinds. Unowned settlements do not share
style with one another. Pending orders, names, painted blocks and generated
village provenance never establish style. Other kinds retain their native
blueprints. Upgrades resolve the actually committed blueprint, not the first
catalog entry of the same kind.

The houses use white calcite masonry and terracotta/red-sandstone tiled roofs;
the chapel includes a real hanging bell in its contiguous tower and a cleric's
workstation; the plaza has a covered dry well and storage barrels; the fortress
and wall use masonry. These are authored vanilla-block interpretations, not a
claim of historical reconstruction. Calcite, roof material, bell and cleric
supplies must be physically delivered: the native miners do not manufacture
them. Builders pay and place each cell with the existing construction executor.
The plaza is a storage building, not a new civic simulation or free water source.

`python3 tools/settlement_chronicle.py --run RUN.json --telemetry TELEMETRY.jsonl
--server-log SERVER.log --out OUTPUT` extracts JSON and plain caption lines.
The run envelope records seed/server identity and an ordered `snapshots` list.
Each snapshot contains native `tick`, simulated `game_time` and `settlements`;
each settlement has native `id`, `name`, `owner`, observed `population`, and
`buildings` (`id`, `level`, and a proven native `blueprint` when available).
This is an offline observation format, not another persisted world field.
The first census is baseline. New records, native certificates and population
milestones receive observation-interval dates; building audit events replace
those intervals with exact simulation ages. Days are simulated age / 24,000,
not wall time or a `/time add` reconstruction. Missing tick telemetry is reported,
never interpolated into a TPS claim.

Lineage requires a native `settlement_daughter_founded` audit event with parent,
daughter, tick and simulation age, plus same-owner native records. A second
same-owner town alone is never a daughter event. Audit logs must be retained
alongside the world and telemetry; the chronicle does not invent lost history.

### Native daughter founding

The owner-approved founding schema and its five committee amendments are
recorded in [owner decisions](../settlements/OWNER_DECISIONS.md#daughter-founding--owner-approved-2026-11-15).
Main requires independent persistence review before integration.
A styled owned town with at least 24 residents considers one complete, fed
household at a time. The initial outpost has three house beds, so admission
requires two or three members, including at least two adults, with no military,
conversion or economic reservation. No resident is spawned for expansion.

The parent persists only an optional `founding` object containing a ground-level
`destination` and the existing `household` key. It reserves a loaded, buildable
site outside the parent's radius against other towns, expeditions and structure
claims. Four ordinary paid projects build three Spanish houses and a Spanish
plaza. Construction progress (stages, completed work and the advance watermark)
remains in the existing structure ledger. An outpost is not yet a civil
settlement, and its beds are not available to the parent allocator.

After all four physical structures are committed and validate, one native
tick prepares and publishes the parent, daughter, reissued daughter building
IDs, transferred structure-ledger ownership, unchanged resident bodies and
inventories, household membership, and cleared expedition. The daughter shares
the original owner and therefore inherits committed culture. Its anchor is the
actual outpost plaza bell. No teleport, inventory copy or free block paste is
part of handover. The native daughter audit supplies chronicle lineage and
names the household; `settlement_founding_admitted` records the admitted
household's id, origin, members, adults and destination. `settlement_birth`
names the origin of the household the child joined, and the offline census
lists each settlement's households with origin, stored members and adults.

Recovery rejects invalid or overlapping persisted claims. Losing the selected
household or moving it elsewhere cancels its active expedition; an interrupted
handover must recover either the entire old image or the entire new image.
Old checkpoint and WAL rows decode missing `founding` as `None`. Once a world
contains this new field, downgrade to a binary whose row schema denies it is
unsupported; restore the matching pre-upgrade world backup instead.

Short land strolls reject steps whose actual standing feet are in water
(`LandRandomPos`'s dry-land policy); the general work/POI router retains its
wading and floating permission. A dry stroll must not send a resident down
into a roofed village well merely because the water bed is standable.
This is entry prevention, not a teleport or a claim that a trapped resident
can jump through the well's low roof.

Purposeful work and POI routes also charge the vanilla `WATER` malus (8.0,
scaled to 80 against a cardinal step of 10). A standable water bed is not a
dry-land preference: without that cost, workers cut through a roofed well
instead of following its dry rim and can no longer reach food or construction.
Water is still traversable when needed; swimming, fords, collision and physical
escape rules are unchanged. This preference is not an unconditional water-entry
fence and does not recover an already trapped saved resident.

MEET approaches the bell's gathering area, not its exact block. Like vanilla
`SetWalkTargetFromBlockMemory` in the meeting package, it stops within Manhattan
distance six on a supported, dry cell; an immersed resident still follows
its ordinary shore route. Both route admission and already committed movement
honour that arrival distance, including ticks without a new brain decision. Other work,
food, household and military destinations keep their own arrival contracts.

Resident motion bounds each horizontal walking step by the remaining distance to
its committed waypoint. A lower waypoint stays committed until vertical arrival,
but the resident stops at its horizontal centre instead of stepping across it
and reversing while falling or stepping back onto the ledge. Route selection,
arrival tolerances, gravity, jumping and external impulses are unchanged.
