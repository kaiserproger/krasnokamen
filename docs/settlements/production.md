# Production and economy

Settlement production uses residents, workstations and real inventories to make and deliver goods.
See [menus](menus.md), [construction and workshops](construction.md), and [jobs](jobs.md).

## Production orders

Open **Production orders** from the settlement menu and select **New order**. Choose a crafting table inside a certified workshop, a recipe, quantity and a free resident. The order list shows progress and, for unfinished jobs, **Pause**, **Resume** and **Cancel** controls. The resident uses ingredients in their carry and equipment and fetches missing inputs from settlement storage. Fetching requires physical reach; the resident must return to the workstation to craft.

**Craft missing ingredients** is unchecked by default; without it the order is one plain job whose inputs must already be in the resident's carry or in town storage. The planner compares the required ingredients with the resident's carry and equipment and unreserved town storage, counting a double chest once. It recursively plans recipes for shortfalls, preferring a recipe whose ID starts with its product's name over a by-product recipe; recipes that rework their own input are not producers. Planning neither moves nor reserves stock, so stock lost before a step runs can still leave it waiting for input.

The order is admitted as a chain for one resident: intermediate jobs such as `ui-1-1` and `ui-1-2`, then the final job `ui-1`. Each waits for its predecessor, shown as `waiting for {job}`. Intermediate outputs remain available for the next step; if carry needs space, surplus can be stored and fetched later. Cancelling any chain step, including a finished step, cancels its unfinished steps; completed receipts remain.

Planning refuses unknown recipes, missing raw materials, cycles, a required tool the resident does not hold, unavailable stations, oversized plans and reused order IDs. It reports a reason and admits no partial chain. A missing material can produce a cycle refusal, for example when hay and wheat depend on one another without enough starting stock.

## Delivery and carry protection

A standalone or final chain job records a component-exact delivery obligation for each portion's authored outputs, not recipe remainders such as returned buckets. It must store that portion before crafting the next. Intermediate jobs do not create this output obligation. Before an obligation exists, the worker may store surplus carry to make space.

The delivery transfer selects only owed outputs, leaving pre-existing matching units and unrelated food, tools and cargo in carry. Partial store commits pay only the quantity actually moved. If no storage can accept output, the job waits with `no output storage`; an unreachable storage approach can leave it at `awaiting output storage reach`. A standing cell is not proof of a route. Items are not dropped to satisfy delivery.

Active jobs protect the carried inputs needed for the next portion from ordinary deposit, eating, sharing and autonomous bread conversion; surplus remains available. Manual pause releases ordinary input protection, but not outstanding output protection or worker ownership. When output shares components with existing carry, the whole mixed pool is protected until delivery, while the pre-existing count is retained.

Matching ground-item pickup, crop or net-seed harvesting, and autonomous bread conversion are deferred while they would add to an owed output pool, including during pause. Unrelated items and crops remain eligible. If an owed pool suffers unexplained loss, substitution or gain, the job waits with `delivery cargo changed; cancel required`; adding items cannot fabricate fulfillment. Cancellation stops delivery, releases protection and leaves unpaid debt visible; it does not refund or fulfill it.

Production `done` and `completed` count crafting, not delivery. A job remains unfinished while its delivery is pending; only committed storage transfers pay its obligation. Navigation to supply is derived again after restart; see [settlement architecture](../architecture/settlements.md) for persistence.

## Production commands

The following use `/settlement production <name>` as the prefix. `status` is public; the other operations, including readbacks, require the owner or an operator. Coordinates are single comma-separated `x,y,z` tokens.

```text
/settlement production <name> status
/settlement production <name> start <job> <worker-UUID> <recipe> <count> <building> <station-x,y,z> <stand-at-x,y,z>
/settlement production <name> pause <job>
/settlement production <name> resume <job>
/settlement production <name> cancel <job>
```

`start` admits a single job. The worker must be bound to the settlement; the live station must match the recipe and lie inside the building, with a valid adjacent stand. The building token can identify a native production building or a district certificate; a matching district certificate takes precedence. Use `/settlement building list` for native buildings.

The core command handler also recognizes these operations, but the Minecraft client command tree does **not** advertise their literals; do not assume they are available through the vanilla client command interface:

```text
/settlement production <name> order <job> <worker-UUID> <recipe> <count> <building> <station-x,y,z> <stand-at-x,y,z>
/settlement production <name> carry <worker-UUID>
/settlement production <name> job <job>
```

`order` plans the autocraft chain and returns `Autocraft order <job> planned: ...` or `Autocraft refused: <reason>`; native admission errors use the normal action refusal. `carry` returns the worker, tick, inventory fence and physical slots as JSON. `job` returns recipe, worker, progress, pause/cancel state, receipts, chain `order` and `after`, delivery obligation and transfer sequence, and `delivery_complete`. Neither readback transfers items. `delivery: null` means untracked, not proof that historical output was delivered.

## Startup work recipes

These are the recipes defined in the startup settlement recipe catalog; all use a crafting table. Counts and durations are per craft. Tool outputs have authored components; the repair and salvage recipes require the specific component state in the catalog.

| Recipe ID suffix (`krasnokamen-settlements:`) | Inputs → output | Ticks |
| --- | --- | ---: |
| `oak_planks` | 1 oak log → 4 oak planks; requires a wooden axe, 1 wear | 200 |
| `stick` | 2 oak planks → 4 sticks | 80 |
| `wooden_pickaxe` | 3 oak planks + 2 sticks → 1 wooden pickaxe | 240 |
| `wooden_hoe` | 2 oak planks + 2 sticks → 1 wooden hoe | 240 |
| `wooden_axe` | 3 oak planks + 2 sticks → 1 wooden axe | 240 |
| `stone_pickaxe` | 3 cobblestone + 2 sticks → 1 stone pickaxe | 320 |
| `bread` | 3 wheat → 1 bread | 120 |
| `sugar` | 1 sugar cane → 1 sugar | 80 |
| `cake` | 3 milk buckets + 2 sugar + 1 egg + 3 wheat → 1 cake; returns 3 buckets | 320 |
| `repair_wooden_pickaxe` | 1 damaged wooden pickaxe + 1 oak plank → 1 repaired wooden pickaxe | 240 |
| `salvage_wooden_pickaxe` | 1 wooden pickaxe → 1 stick | 120 |
| `saddle` | 3 leather + 1 iron ingot → 1 saddle | 240 |
| `hay_block` | 9 wheat → 1 hay block | 80 |
| `open_hay` | 1 hay block → 9 wheat | 40 |

## Economy domains and worker ownership

Each domain uses `/settlement <domain> <name> <action> ...`. `status` is public; other actions require the owner or an operator. `/help settlement <domain> <name>` shows the advertised action syntax.

| Domain | Actions | Purpose |
| --- | --- | --- |
| `production` | `status`, `start`, `pause`, `resume`, `cancel` | Physical worker recipes; core also handles `order`, `carry`, `job` as described above. |
| `commerce` | `status`, `quote`, `accept`, `cancel`, `reserve`, `payout` | Trades and delivery escrow. |
| `payroll` | `status`, `employ`, `stop`, `dismiss`, `pay` | Funded worker contracts. |
| `caravan` | `status`, `open`, `pickup`, `unload`, `cancel`, `block` | Physical container-to-container transport. |

Production and caravan worker ownership is indexed separately from navigation and persists across manual pause. Competing production, builder assignment and work, soldier recruitment and orders, and caravan admission and transfers respect this ownership. The indexes are derived and rebuilt during recovery and policy updates; they are not inventory or reservation authority.

A loaded, blocked caravan can be cancelled explicitly. Its carried items, cargo, history, fences and blocked reason remain unchanged; cancellation releases the worker without creating a delivery or refund.

## Upgrading production jobs

Before enabling output delivery on an existing world, finish or cancel each unfinished standalone or final production job with the previous server, and retain a restorable pre-upgrade world backup. The new server refuses recovery of an unfinished legacy final job without a delivery obligation; it cannot reconstruct which carried items were that job's output. Intermediate chain jobs are not output-delivery jobs. Previously completed jobs have no inferred delivery, so their old output is not retroactively proven stored. If recovery refuses, stop the new server, restore the pre-upgrade backup, and use the prior server to resolve the identified job, then cleanly shut down, back up, and reopen with the upgraded server. Do not strip fields or invent obligations. Once the new server has persisted delivery fields or new cancellation states, the prior server may reject its checkpoint or WAL; rollback requires restoring the complete pre-upgrade backup, not merely replacing the binary.
