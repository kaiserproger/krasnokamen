# Villages and villager jobs

This page explains generated village adoption, resident jobs, and resource workers.

See [construction and certification](construction.md), [production](production.md), [army](army.md), and [menus](menus.md).

## Autonomous villages

- A generated village becomes an ownerless settlement when its worldgen meeting piece and bell load. Its identity comes from the dimension and structure start chunk, not any nearby player-placed bell. Living generated villagers bind as their chunks load; absent or dead villagers do not remove the settlement. Reloading or claiming it does not create a duplicate.
- Adoption uses a nearby existing chest, or places one empty chest near the bell without consuming an item. The chest contains no fabricated supplies. Construction uses real materials; see [construction](construction.md).
- Natural mine and lumber-camp candidates need an entrance in an entity-simulated chunk and proven walkable routes both to settlement storage and from an eligible bound adult. Eligible adults include unemployed residents without a job and untraded miners or lumberjacks. Natural mines also need a safe first cut. Route proofs happen on later bounded planner passes; an unreachable candidate is not installed as a job.
- To claim an ownerless settlement, stand inside its radius and use `/settlement claim` or **Claim settlement** in `/settlement`. The first successful claim records the player's UUID; claiming does not replace residents or reset the settlement.
- Founding or claiming also binds eligible unbound villagers inside the radius at the start of the action. Population growth during binding does not expand that selection. The original entities are retained; villagers belonging to another settlement, dead villagers, wandering traders and converting villagers are left unchanged. Later unbound arrivals can be added with `/settlement bind <name>`.
- Settlement work depends on loaded, simulated chunks. Unloaded work waits; construction, development, recruitment and battles are not simulated as off-screen catch-up. Keep the village and work routes active.

## Allocating jobs

- Adults normally seek reachable work during day ticks **2000–8999**. Nitwits do not work. Unbound villagers can acquire ordinary workstations within 48 blocks; bound residents acquire only free slots in certified production buildings. Certification and upgrades make slots available to the normal allocator.
- The allocator prioritizes reachable lumberjack and miner slots over other jobs, including after storage is built. An unreachable candidate does not permanently block other choices.
- Bound farmers, shepherds, miners, lumberjacks, builders and soldiers look for slots matching their retained profession. Traded villagers also keep their profession; other eligible adults can retarget. Destroyed or claimed workstations release a job claim, but an unloaded workstation alone does not.
- A bound resident with an uncertified remembered job releases the claim while retaining profession and offers. The menu may report **“N workers need a certified workshop — open Buildings → Certify”**. Certify a matching building around the workstation to make its slots available; the warning also counts other jobless employed professions, not just residents from older worlds.

### Workstations

Ordinary workstation mappings are below. Bound residents additionally need the matching certified building and an available slot.

| Workstation | Profession |
| --- | --- |
| Composter | Farmer |
| Lectern | Librarian |
| Barrel | Fisherman |
| Smoker | Butcher |
| Blast furnace | Armorer |
| Brewing stand | Cleric |
| Cartography table | Cartographer |
| Fletching table | Fletcher |
| Cauldron, including filled variants | Leatherworker |
| Stonecutter | Mason |
| Loom | Shepherd |
| Smithing table | Toolsmith |
| Grindstone | Weaponsmith |

Builder workshops, mines and lumber camps can use a crafting table or barrel; mines also accept a smithing table and lumber camps a fletching table. Certified natural zones use their entrance as the work point. See [construction](construction.md) for certification and slots.

## Rest and routes

- At night a bound resident with a roofed home bed can sleep there. Residents claim only roofed beds as homes (and release a home whose roof is removed), so an open-air bed stays free for unbound villagers; a resident without a roofed bed draws no stored rations (see [breeding](breeding.md)). Other resting or idle residents seek an unoccupied roofed standing place near a reachable settlement bed. Panic and soldier duty take priority.
- As in vanilla, using a bed a villager sleeps in only wakes that villager; it stays up for 100 ticks, so the next use puts the player to bed.
- Rest takes precedence over hauling and economy travel: workers keep their carried items while going home. Visible hostiles and recent damage can cause panic; hostile sensing checks line of sight.
- Resident routes and short walks stay in entity-simulated chunks. Villagers can wade through water at their feet, but not head-height water or lava. Blocked routes require a usable path.
- Villagers open wooden doors along their routes, not iron or copper doors. They close doors they opened after passing, waiting for another villager occupying or approaching the doorway; they leave player-opened doors alone.

## Miners

- Certified mines assign miners. During work hours, with `mob_griefing` enabled, miners dig a staircase and 1×2 branches; the saved cursor tracks their lane, level and flight. They reuse the open staircase for higher levels and start deeper flights beside the preceding one, alternating sides. Closed lanes lead to another lane or level; exhausted mines send remaining resources to storage.
- Built and natural mines use the certified horizontal footprint. Their working volume can extend up to **48 blocks below the certified floor**, subject to the settlement sphere. A natural zone outside the sphere keeps its certified depth instead.
- Unloaded neighboring blocks pause a cut. Fluid, waterlogged blocks, underwater vegetation, powder snow and unsafe cave faces block a lane; a qualifying dry connected tunnel of the same mine is not treated as a cave. Natural mine alternatives use surface entrances within the certified bounds.
- A hostile near the next cut makes the miner wait; if it remains, or the worker cannot reach the cut, the lane is abandoned after **2,400 ticks** without progress. Changes to mine bounds reset the cursor; a level-only upgrade retains it. A project targeting the miner's building pauses digging while allowing delivery.
- A miner returns to storage when one stack reaches 64 items or the mixed load totals 64, at day end, when the mine is exhausted, or when the next item cannot fit. Full or unreachable storage does not delete the carried resources.

## Lumberjacks

- Certified lumber camps assign lumberjacks. With `mob_griefing` enabled during work hours, they search simulated columns within the settlement radius for mature log trunks with natural, nonpersistent leaves, cut them and replant the matching sapling (a propagule for mangroves) if the soil and space remain suitable.
- The saved search cursor advances and wraps; a natural camp starts its sweep at its certified grove footprint. A search attempt examines at most **128 simulated columns**. Trees in inactive chunks are not targets.
- An unreachable tree is released after a completed failed approach, or after **2,400 working ticks** outside cutting reach, and the sweep continues. Arrival, deposit and off-shift time reset that work window.
- Certified structures protect their footprints; only the worker's own natural lumber-camp zone permits cutting within its bounds. The crown and protections are checked again before each cut.
- Lumberjacks return when one stack reaches 64 or the mixed load totals 64, at day end, when another log cannot fit, or when their delivery closes a construction candidate's funding gap. An already-funded candidate does not trigger a trip after each log. Full or unreachable storage retains the carry.

Miners appear to clients as toolsmiths and lumberjacks as fletchers, with the corresponding vanilla trade pools.

## Labels and nameplates

Labels are presentation only: they are computed from live state and never saved.

- **Building labels** hang at a building's entrance and show the home occupancy (`Дом · жильцов n/m`), each workshop with its profession and workers (`Мастерская` or, for farmers, `Ферма`), and at the bell a summary with the settlement name, owner, resident count, working and unemployed residents. An ownerless village shows its autonomous state and the hint `/settlement create <name>`. The owner appears by current player name; if the name is unknown the saved name or UUID is shown. The chat summary shows the same words and counts.
- Labels are visible within 48 blocks of the viewer once the chunk is loaded.
- **Villager nameplates** read `Name · profession · activity` (activity: work, rest, meeting, stroll, play, panic, sleep). Children show `ребёнок` instead of a profession until they grow up. Generated plates follow the villager brain and are never saved as name tags; a name tag you apply is kept and shown instead.
- A resting or idle villager can sit on a nearby roofed bottom stair with room above it. The seat is presentation only and disappears when the villager leaves or changes activity.
- Label and nameplate text is Russian regardless of the client language.
