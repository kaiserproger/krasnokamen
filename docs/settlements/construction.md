# Construction and buildings

Settlements build from authored blueprints and can certify player-built structures and natural work zones.

See [jobs](jobs.md), [menus](menus.md), [breeding](breeding.md), and the [settlement architecture](../architecture/settlements.md).

## Blueprint files

Krasnokamen owns the JSON templates in `crates/mc-core/assets/blueprints/`. The embedded catalog has 21 blueprints, one per building kind (`id` is the `/settlement build` argument):

| Category | Blueprints | Levels |
|---|---|---|
| Housing | `house` | 1–3 |
| Storage | `storage` | 1–2 |
| Production | `farm`, `mason` (the Builder hut), `lumber_camp`, `mine` | 1–2 |
| Production, owner-ordered workshops | `armorer`, `butcher`, `cartographer`, `cleric`, `fisherman`, `fletcher`, `leatherworker`, `librarian`, `toolsmith`, `weaponsmith` | 1–2 |
| Production, shepherd | `shepherd` | 1 |
| Military | `barracks` | 1–3 |
| Fortification | `wall`, `watch_tower`, `gate` | 1–3 |

Each workshop is built around its profession's vanilla job-site block (blast furnace, smoker, cartography table, brewing stand, barrel, fletching table, cauldron, lectern, smithing table, grindstone), one more per level, so a resident of that profession claims it like any certified workshop. A workshop's bill includes that block, which workers do not produce (iron, flint, books, a blaze rod): stock the delivered item or its ingredients first. The gate opens one more door per level between four solid pillars; wall, tower and barracks level 3 meet the level-3 certificate dimensions below (13 × 6 wall, 10-high tower, 6 beds). A geometric certificate can reach level 3 even when its blueprint has no corresponding upgrade.

Each file has an `id`, a building `kind`, a character-to-block-state `palette`, and `levels`. Each level contains `cert`, the certification box dimensions `[x, y, z]`; `entrance`, an `[x, y, z]` offset; and bottom-up `layers`, each made of Z rows of X characters. A dot is air. Palette states can include properties, for example `loom[facing=north]`. Each level is a complete picture, not an upgrade patch: the loader derives placements, removals, materials and rotations from it. A level must keep every container of the previous level (chest, barrel, furnace and other inventories) as the same block state; the loader refuses a level that replaces or removes one, because Builders never write over a stocked container and such an upgrade could not finish. The maximum placed dimensions are 13 × 11 × 11 blocks: 13 is the level-3 wall's length, which runs along its road, and a lot is at most 11 deep across the front a building turns to the road.

The origin is the minimum corner of the rotated **maximum footprint across all levels**. Its Y is the ground-surface block, replaced by the floor layer. Certification bounds rotate in the same frame and may be smaller than the placed footprint. Upgrades retain the origin, prepared foundation and rotation. The farm's certificate includes its field as well as its shed.

For authoring, `tools/blueprints.py check` checks structure, `tools/blueprints.py materials [id ...]` estimates material totals, and `tools/blueprints.py preview [id ...]` writes PNG previews. Material estimates are not runtime funding proofs: recipes may have fractional estimates even though real crafting consumes whole batches. The Rust blueprint tests check certification of the levels; the Python check is not that gate.

## Planning and streets

Keep the anchor, storage and worksite chunks loaded. Construction uses real items in certified storage and the generated village's bootstrap container, not player inventories. A player-founded settlement has no free stock; certify storage and stock it.

The planner selects one active settlement every 20 ticks. It considers missing kinds in this order: storage; houses when certified house capacity is below population; farm; lumber camp; mine; Builder hut (`mason`); barracks from 6 residents; tower from 8; wall from 10. Natural mine and lumber zones do not count as built blueprint kinds. The gate, the shepherd's hut and the ten profession workshops are never started autonomously; the owner orders them. Funded upgrades are considered for eligible blueprint-built certificates; workshops requiring iron or flint are not on this automatic list. A need without available stock or a capable local source does not conjure materials or have to block another funded kind.

An unfunded priority bill does not stop the search for funded buildings or upgrades. Its staffed extraction worker is protected from Builder recruitment for autonomous work, leaving another resident free to build; explicit owner projects retain their active-bill gathering rules. Optional street projects still wait for that priority bill.

The native `civilization-seed-qa` census uses `minecraft_list_entities` cursor pages of 16 across the original 128-block radius, without reducing or dropping entity kinds. Native type filtering is used only for the targeted newborn refresh; birth proof still requires that same UUID's received baby metadata, targeted screenshot and final checkpoint audit. This chooses native cursor pagination rather than replacing the client census with server counts: the original client-visible coverage remains intact while each MCP reply stays below its transport byte limit. Use a pinned client exporting these optional arguments through `mcp-schema`; the Loader schema remains its own actual Java catalog.

Each native paginated census captures its complete matching received rows on the first page. Later pages retain that membership and metadata despite incoming packets or player motion; continuation tokens cannot be replayed, reused after a new census, or used with another context/query. The harness checks the same total on every page and never substitutes a partial baseline.

Non-house building milestone captures keep the original inside-building look target but navigate to one supported exterior standpoint selected deterministically from received perimeter geometry, using the same dry-cell walkability check as the village approach. A bounding-volume midpoint is neither a player-feet position nor a route through a closed door. Missing support fails the capture; there are no candidate retries, door mutations, teleports or relaxed ray-target checks. House captures instead approach the received entrance, use it ordinarily if closed, await both open door halves and retain the exact-bed raycast and screenshot before returning to the original observer.

The deterministic adopted-village closure runs the native root tick, including renewable growth and block publication, rather than advancing only autonomy, settlements and brains. It still requires all four harvested source types, built storage and housing, certified natural worksites and a Builder; construction evidence is published plank placement rather than an undrained pending-write buffer. Closure is deterministic at tick 72,901; the test is bounded at 80,000 ticks and never substitutes a shorter successful scenario. Instead of a wall-clock guard it checks a deterministic work budget on every tick: at most 30,000 full funding proofs and 1,500,000 recipe candidates over the run (closure uses about 20,000 and 870,000). Disabling the recipe index, dependency admission or the funding memo separately exceeds that budget. Timings belong to profiling runs, not to this test. This is fixture evidence, not server-capacity or graphical acceptance. Movement follows admitted navigation targets in this fixture; graphical movement remains the separate real-client gate.

The street plan is fixed relative to the bell: roads are three blocks wide every 28 blocks on both axes, with a one-block verge, an open 9 × 9 plaza, and two back-to-back rows of lots up to 11 blocks deep. `/settlement layout` reports the plan, reachable and obstructed roads, paving, lamps, building access and next street work.

- A frontage must be reachable from the plaza on loaded ground within eight blocks of the bell's elevation, with steps no higher than one block. A funded building may include natural-forest clearing on its lot and walking corridor; admission itself changes no blocks. Water, lava, steep terrain and unrelated obstructions still cut the network off.
- A building's entrance faces its road. The planner seeks a free, fundable lot in walking order from the plaza. Autonomous passes examine up to 64 candidates and resume from their cursor; a new owner request searches the loaded plan. An owner order waits in the single pending-order slot if forest evidence is still being generated.
- The future footprint may span at most three blocks of ground-height difference. Its floor is at the door step, at most one block from the verge height. Natural ground above the floor can be excavated; gaps below it require real dirt from storage. Each cleared or filled cell is work and each fill uses one dirt. Unloaded or unsuitable sites and protected paths, farmland, fluids, containers and other obstructions are not cleared to prepare a site. The foundation is included in the bill.
- When no building or upgrade starts, Builders can pave reachable road stretches serving standing buildings: natural soil becomes dirt path and replaceable plants above are cleared without a material charge. Lamps on served stretches use two fences and a torch from storage, at the planned positions every nine blocks. Stone and sand remain as they are. There is no automatic water placement for a well.
- The first Farm's field costs 12 wheat seeds, and only a farm produces them. While a settlement has no Farmer workshop and fewer seeds in storage than the Farm needs, the same free Builder time goes first to a **meadow clearing**, even while a priority material worker is out (that worker is not recruited for it): a 16 × 16 window within 48 blocks of the bell holding at least 8 seed-bearing plants (short grass, tall grass, fern or large fern), nearest window first, outside every building and container. The clearing costs nothing, and each plant drops its vanilla loot into real storage (a wheat seed one time in eight), so a fresh town gathers the seeds itself; a sparse savanna needs several clearings. Street work follows when no window qualifies or the stock suffices.

Natural forest must match deterministic regeneration by this world's generator, with no retained player edit at that block; imported Anvil baselines and baselines without generator provenance are protected. Natural leaves may differ from regeneration in their evolving `distance` property. Regeneration runs off-tick and caches only tree evidence within a 64 MiB payload budget. Its worker stops with its world: closing the world waits for at most the chunk being generated, then releases the world's store. Lumberjack-planted sapling positions are persisted, protecting their trunks and nearby crowns after restart; vanished trees are retired when their trunk column is loaded. With block history disabled, forest within 32 horizontal blocks of a live player is additionally protected. An identical player replacement at its original natural coordinate after `/co` history expires is indistinguishable from nature and may be felled into settlement storage, with its items conserved.

Builders approach a tree's footing within four blocks and clear its trunk and leaves inside the lot footprint and walking corridor through the canonical one-cell executor. A preparation cell is cleared from a dry, open stand within four blocks of that cell's footing. A stand is a cell the builder's body can stand on, so the top of a fence or wall post never counts. On uneven ground the builder walks to such a stand rather than the nearest one, and a cell with no such stand is cleared from the perimeter like any blueprint cell, so it never holds the project slot. Forced leaf removal uses vanilla loot without a tool (saplings, sticks and apples by chance); those drops and log drops are credited atomically to real settlement storage before removal, and full storage leaves the block untouched. Surface plants a new building or street clears also yield their vanilla loot without a tool, so short grass drops wheat seeds one time in eight. That loot goes to storage; what storage cannot hold drops at the cleared cell. It is how a settlement gets the seeds for its first farm. Leaves outside the footprint and access path are left to vanilla decay. Once preparation is complete, normal footprint/perimeter construction rules apply.

## Builders and supplies

An eligible adult resident can become a Builder; the planner prefers unemployed residents and may borrow eligible miners or lumberjacks, restoring their profession afterwards. A Builder who cannot reach storage can be released for another candidate. A Builder whose project has not advanced a cell in 3000 shift ticks (a hostile mob that never closes in can keep a resident fleeing and returning on the only road to the lot) is released the same way; the replaced Builder is chosen last for the next 24,000 ticks. The watch is runtime only and is not saved. Work requires the daytime work shift, ticks 2000–8999, and `mob_griefing` enabled.

Construction admissions select a settlement every four ticks. One admission performs either a real material transfer or one cell of construction, not both. The Builder batches materials into its real carry slots and walks to the site. For blueprint work, its entire body must be outside the full future footprint and the next cell within **12 blocks** of its feet; perimeter targets are two cells outside the footprint. Missing materials wait for replenishment. Completion checks the resulting certificate.

If an active project's remaining bill loses its funding, the planner staffs the missing extraction job. When its only suitable worker is the project's borrowed Builder, it releases that worker back to extraction until real deliveries fund the remaining bill; the project stays in place. A lumberjack gathering for that shortfall seeks the project's fixed wood family. A partial load that funds the remaining bill is returned to storage rather than held until a stack fills.

A partial lumber load triggers an early delivery only when it closes a funding gap. If the candidate or active remaining bill is already funded, normal harvesting continues instead of sending each newly cut log home while the planner waits for a legal site.

The planner tries wood species by available stock and chooses one that can fund the complete bill, including oak as a candidate. The chosen species and bed color are fixed in the admitted project's cells. A bed needs three wool of the same color; local eligible workers can shear an adult sheep using real shears, including shears crafted from two iron ingots. Construction iron demand can be supplied by workers carrying real ore and fuel to an owned ordinary furnace and returning ingots to storage. Unemployed residents, shepherds and miners load the furnace; other employed residents only carry its output to storage. The demand is the larger of the active project's remaining bill and the largest skipped autonomous candidate, so a funded later building does not stop the shears for an unfunded house bed. When needed and fundable, a Builder can place a furnace near storage using a stored furnace or one crafted from eight cobblestone. These supplies are not free.

Builders can use bounded recursive vanilla shaped and shapeless crafting chains, such as logs to planks to doors or planks and wool to beds. Fetch and crafting commit against real storage and carry; insufficient inputs or carry space do not partially debit the batch. This crafting path does not itself smelt or stonecut. Farmland costs dirt, wheat costs seeds, door upper halves and bed heads are free counterpart cells, and double slabs cost two slabs. Water is not created for fields.

The whole-bill proof and the Builder's hauling account for wood needed by the remaining species-specific cells. Generic recipes use stock left after that reservation, so a mixed-species stock can fund generic items without exhausting the project's chosen wood. Crafting yields whole batches, and surplus from a batch remains available for later cells.

The content registry rebuilds an immutable, ordered recipe/ingredient index with every recipe or tag installation. Boolean funding can reject a bill whose ingredient dependency has no available base stock; cycles, the depth boundary and custom-model inputs or outputs remain conservative and fall through to the ordinary proof. Quantities, physical slot choice, wood reservations, whole-batch surplus, capacity and work limits still use that proof, with per-proof derived counts and an exact slot undo journal.

A non-persisted settlement-registry memo retains at most 16 Boolean or wood-reservation proofs together. Its keys compare the probe kind, full ordered bill, every slot and stack component by value, and a strongly owned recipe-index identity; it is never saved, so a loaded world starts without it. Native callers run on the tick owner, not entity workers. Concurrent misses may duplicate computation and vary eviction order, never results. The typed structure ledger is likewise derived from the canonical plugin domain: every write, mutable borrow, deletion and WAL replay invalidates its exact domain, while checkpoint and journal bytes remain unchanged.

A foreign edit in a fenced construction footprint pauses the project as `footprint_changed`; the next construction admission re-snapshots and resumes it. **Remaining cells can be placed over the changed world**, while already placed cells are not re-planned. Removing a settlement cancels its construction handle but leaves placed blocks and carried materials.

## Owner build requests

Chat build commands target the claimed settlement containing the player and require its owner or an operator:

- `/settlement build <blueprint>` requests a new level-one blueprint.
- `/settlement build upgrade <buildingId>` requests an available blueprint upgrade by building ID; it is not the same as rechecking a certificate.
- `/settlement build cancel` cancels the pending owner order, not active work.

The **Projects** menu offers `Order: <blueprint>`, available upgrade buttons, **Status**, and **Cancel pending order**. Menu targeting can select a nearby settlement or an owned-settlement fallback; permission is checked on the selected claimed settlement, without requiring the player to stand inside it. See [menus](menus.md).

An order can start when the project slot is available and the full real-material bill, including crafting and site preparation, can be funded. Admission itself neither places blocks nor debits stock. A funded owner request can preempt an autonomous project with no completed work, but not an owner project or work already started. If busy or short of materials, one pending order is retained; another queued request replaces it. While pending, it blocks new autonomous projects but not completion of current work. Refused requests are not queued. **Projects** and **Status** report progress and unmet needs; cancelling the pending order releases autonomous planning.

A queued upgrade targets the building and chooses its next available level when it starts. If the building disappears or has no available next upgrade, the planner drops the order and notifies the online owner. A wall order builds a wall section with Builders; it is neither certification of player blocks nor an automatically enclosed perimeter or gate system.

## Certification commands

Certification inspects loaded blocks without placing them or charging materials. Certify and list select the settlement containing the player, choosing the nearest anchor and then lowest ID if radii overlap. Certification requires the owner or an operator of a claimed settlement; certificate upgrades address a building globally and require management permission for its settlement.

- `/settlement building certify <kind> <x1> <y1> <z1> <x2> <y2> <z2>` certifies level 1 using inclusive, ordered minimum and maximum coordinates. Include floor and roof where required.
- `/settlement building upgrade <id>` checks the existing bounds for the next certificate level, up to 3; build the extra blocks first. This does not order Builder work.
- `/settlement building list` shows IDs, kinds, levels and bounds.

Re-certifying the same kind with bounds containing an existing certificate refreshes or expands it without changing its ID or level. Other overlapping certificates are rejected. Bounds must fit inside the settlement and loaded chunks, with at most 64 blocks per axis and 32,768 blocks in total. A settlement can have at most 1,024 buildings.

Kinds are `house`, `storage`, `barracks`, `wall`, `tower`, `gate`, `mine`, `lumber_camp` and profession workshops: `armorer`, `butcher`, `cartographer`, `cleric`, `farmer`, `fisherman`, `fletcher`, `leatherworker`, `librarian`, `mason`, `builder`, `shepherd`, `toolsmith`, `weaponsmith`.

## Certificate requirements

| Requirement | Level 1 | Level 2 | Level 3 |
|---|---:|---:|---:|
| Footprint area, except wall | 9 | 16 | 25 |
| Inclusive height, except wall and tower | 4 | 4 | 4 |
| House beds | 1 | 2 | 3 |
| Barracks beds | 2 | 4 | 6 |
| Storage containers | 1 | 2 | 3 |
| Workshop, mine or camp workstations and job slots | 1 | 2 | 3 |
| Wall length × height | 5 × 3 | 9 × 4 | 13 × 5 |
| Tower height | 6 | 8 | 10 |
| Gate doors | 1 | 2 | 3 |

Ordinary buildings need a collision-bearing floor across the selection; a mine's bounds instead designate a digging zone. Houses and barracks need correctly paired beds, a door, a complete roof on the top plane and a closed collision-bearing perimeter between floor and roof. Towers need the roof and perimeter but not beds or a door. Walls need a continuous collision-bearing vertical plane along the longer axis at the minimum perpendicular coordinate; gates need four solid corner columns as well as doors. These are geometric checks, not a style requirement. Beds for breeding have their own rules; see [breeding](breeding.md).

Workshops, mines and camps use distinct matching work points in their bounds, with the first 1/2/3 in coordinate order providing slots. Destroying a workstation releases its slot; unloaded chunks alone do not. A workstation moved within the same building keeps the slot: when a jobless resident looks for work, a work point that lost its workstation moves to an unlisted matching one in the bounds (any vanilla job block or barrel; a moved crafting table needs re-certification). Storage recognizes chests, trapped chests and barrels, up to 64 registered points. Barracks provide two soldier slots per level; see [army](army.md). For workstation types and worker behavior, see [jobs](jobs.md).

## Natural zones and radius

Generated-village adoption can certify a natural level-one mine or lumber zone without a placed workstation or chest. It has one open entrance and one job slot. A lumber zone needs at least four logs; a mine needs at least 32 stone or ore blocks, a 5 × 5 horizontal area and two blocks of depth below its entrance. Bounds must be loaded, within the radius and nonoverlapping. Workers extract the actual finite resources; upgrading requires the ordinary geometry and workstation.

Natural-zone discovery probes outward over the saved radius, at most four candidates per 20-tick pass, using loaded terrain and avoiding generated-piece horizontal footprints. Founding mine candidates use a 15 × 15 footprint, with an entrance on the west edge near a Z edge; quarry depth follows the supported entrance height. The 5 × 5 certification minimum does not guarantee sufficient supplies.

The creation radius is a floor. Population growth raises the radius to at least four blocks per resident, capped at 256; it never shrinks. For example, 30 residents require radius 120. `/settlement info` displays it.
