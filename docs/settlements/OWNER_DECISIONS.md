# Settlement war, mount and equipment decisions

The owner approved these values for settlement war, mounts and mounted charge. The native core does not implement war declarations, mounts or charge: there is no war state, window or grace timer, soldiers fight only on foot with native melee, and the army order parser accepts only `follow`, `guard`, `patrol` and `return` (see [army](army.md)). Treat everything below as the design target to implement exactly, not as current behavior.

The supply recipes from the same decision (saddle, hay block) are implemented; see [production](production.md#startup-work-recipes).

## War

- Declared by the settlement's charter holder with a committed declaration and a delivered reason.
- Grace before damage: 72,000 simulation ticks.
- Windows: at most 8, each at most 72,000 ticks long, at least 12,000 ticks apart, absolute `[start, end)`.
- Targets: at most 8 lawful targets. A record is at most 4,096 UTF-8 bytes.
- Private, neutral and unknown targets are refused. After peace or revoke, no new damage commits.
- Victory only by a committed peace or surrender with a recorded outcome. Holding points never wins; a window that expires without an outcome stops its effects and has no winner.
- Contribution is voluntary only, by a real inventory transfer; nothing is taken automatically.
- Victory grants no separate right to breach or salvage.
- Logging out gives no immunity in a lawful conflict already admitted in the current window; no new offline targets are added.

## Mounts

- The horse is the combat mount. Donkey and mule are pack animals only and never charge.
- Each mount has its own native speed; none is declared a vanilla gait profile.
- No forced dismount at 2 blocks: collision limits movement, and a dismount only goes to a safe position.
- Lawful transit through a settlement is allowed; passage is not a right to damage.
- Open: the native profile for mounted feed, health, stamina, collider, seat and impulse.

## Charge

- Spear or pike profile only, after 40 ticks of sustained speed of at least 0.45 blocks per simulation tick.
- Bonus rises linearly from 0 to +3 HP between 0.45 and 0.60 blocks per tick and is capped at +3.
- One hit per target per charge; the mount re-accelerates after contact.

## Equipment

- The spear is two-handed.
- Fractional wear accumulates.
- One arrow is returned at most once.
- Swapping, reconnecting or redeploying never restores stamina.
- A native arrow range limit is defined.

## Forest site preparation — 2026-11-08

> Да, расчищать лес под стройку

- Funded construction may physically clear natural forest from its lot and access path; admission itself changes no blocks.
- Logs, saplings and other vanilla drops go to settlement storage without loss or duplication.
- A block must have no retained player placement record. It must also match this
  world's deterministic generator, and imported/unknown generator identities and
  persisted lumberjack sapling roots remain protected after restart, except for
  lumberjack trees on needed lots ([lots and terrain](#lots-and-terrain-inside-the-settlement--owner-2026-11-17)).
- Water, lava, steep terrain and other existing terrain refusals remain refusals
  outside the settlement. Inside the settlement this is superseded by
  [lots and terrain](#lots-and-terrain-inside-the-settlement--owner-2026-11-17).
- An identical player replacement at a former natural coordinate after history expiry may be felled with its drops conserved; see [the construction policy](construction.md#planning-and-streets).

## Daughter founding — owner approved 2026-11-15

Status: the owner approved `SettlementRecord.founding` with all five committee
amendments. Main requires an independent persistence review before integration.
This is not an approved war/faction model and must not be described as one.

- The existing owner UUID groups teaser settlements. Only completed native
  Spanish blueprint provenance establishes that owner's style.
- The durable addition is a serde-defaulted optional `founding` on a
  settlement, containing only `destination: [i32; 3]` and an existing `household`
  key. Stage, payment, materials, construction and culture are derived from
  existing native state; there is no persisted lineage or culture flag.
- At 24 residents, a whole eligible household of two or three members can
  commission an outside-radius outpost: three paid Spanish houses and a plaza.
  There must be at least two adults; the three-bed plan cannot receive a larger
  household. Missing or relocated members cancel the expedition.
- Site rights, overlap admission and recovery validation are explicit. Civil
  certificates are withheld until physical completion. Generic plugin structure
  preparation cannot reserve an expedition's site.
- One native WAL tick must publish all civil records, same-body membership,
  correctly reissued building IDs, unchanged inventories and transferred
  committed ledger provenance, or none of them. Admission creates no blocks,
  residents, inventory or duplicate receipts.
- Existing checkpoint/WAL rows decode absent `founding` as `None`. Downgrade
  after writing the new schema requires restoring the matching pre-upgrade
  backup, not opening the new world with a binary that rejects the field.

Implementation and gate evidence belong to the SpainCulture report. Public
claims require the actual pinned-server run, not this design decision.

## Two-way migration — owner direction 2026-11-16

- Physical housing, food, jobs, seven-day resident deaths and completed buildings
  determine attractiveness with weights 30/30/15/15/10. The outside world scores
  35; moves need a gain greater than ten after five points per hundred blocks.
- Whole households or household-less individuals move. Founding expeditions and
  reserved members are excluded. Pending walks and transit count against beds.
- Proven loaded routes use existing relocate/leave/join membership operations;
  other moves walk to the simulated edge before durable transit. Civil radius
  is not the simulation boundary. Outside outflow cannot remove carried stock.
- Outside entrants are ordinary adults, members from their spawn tick with
  `household = None`; manual bind retains its presence check.
- Homeless return is below useful work and native urgent targets. It shares the
  allocator's Home result and bed predicate, native paths, collision and doors.
- Main resolved the runtime-only Hold restart conflict on 2026-11-17: use one
  `1200 + entity-id % 40` cadence between **all failed attempts**, not 200 ticks
  for initial failures. Successful route legs remain immediate; no saved intent.
  Searches cap at 1024 nodes; no more than three initial failures before Hold.
- See [migration authority and physical return](../architecture/settlements.md#migration-policy-and-physical-return)
  for formulas, cadence, notifications and the native custody boundary.

## Food in loaded settlements — owner, 2026-11-17

> Оставить

- Loaded settlements keep eating bread from storage, like unloaded ones: one
  food point per resident per day, bread worth four points.

## Lumber replanting near access — owner approved 2026-11-17

> Согласен

- Planted lumberjack trees stay protected (2026-11-08 above), except on a lot that
  construction needs ([lots and terrain](#lots-and-terrain-inside-the-settlement--owner-2026-11-17)).
- A lumberjack does not plant where the root or future canopy would block
  access to streets, plots or reserved sites.
- Only settlements the canopy can reach are checked; an unrelated settlement
  never vetoes a planting.

## Construction queue order — owner, 2026-11-17

> а никто и не должен уступать домам мэйби? в каком порядке в очередь встали заказы, в том и остаются

- Construction projects keep the order in which they entered the queue; nothing
  jumps ahead because of housing.
- When the active slot frees, the earliest-queued project takes it. A house the
  planner chooses later enters the queue behind what is already queued.

## Lots and terrain inside the settlement — owner, 2026-11-17

> 1. б, забивать хуй на них, если нужно место под участок расчистить. в пределах
> поселения вообще игнорировать трудности ландшафта, строитель их может
> разравнять, вскопать, закопать или срубить

- Lumberjack planting rules stay as they are; there is no new restriction on
  planting along planned streets.
- A tree on a lot that construction needs is felled, including a planted
  lumberjack tree. Its drops go to settlement storage as for natural forest.
- Inside the settlement terrain is not a reason to refuse a lot: the builder
  levels, digs, fills or fells. This replaces the terrain refusals and the
  bell-elevation, frontage-step and footprint-height limits there.
- Unchanged: item conservation (excavated material goes to storage, fill uses
  real material), protection of player-placed blocks, and every rule outside the
  settlement.

> а че мешает лаву расчистить или ведра? создаём заказ на ведра, его исполняет
> (нету - крафтит сам, нет припасов - ну пиздец тогда, закапываем блоками),
> очищаем, всё. главное не утонуть в лаве случайно нахуй

> епта ну засыпать блоками тогда

- Water and lava on a needed lot are filled with real blocks (excavated soil or
  storage) from a safe stand; there are no bucket orders or bucket crafting.
- A builder never stands in, walks through or falls into lava, and never digs a
  cell next to lava beside its stand. After a lava source is filled, the builder
  waits for the remaining flow to drain before working there.
- Among candidate lots the one with the least preparation (cut, fill and
  liquids) is chosen, which reduces the risk of the queue head stalling on
  filling a lake.

> 1. Да, но пометить явно missing. 2. Да, но перепроверяйте целостность дорог.
> Если пересекает участок - восстановить в другом месте, чтобы была целостной
> 3. а.

- Filling water and lava on lots is **implemented** (owner D21, 2026-11-17:
  pulled before the freeze, so the "missing" status above is superseded): water
  and lava below a needed lot's authored floor are filled with real blocks from a
  safe stand and sources first. The builder-side drain wait, the pre-`clear`
  liquid stage, the lava-neighbour veto and the safe-stand rule are wired in the
  executor follow-up, not by the admission change alone; a lava lot is not safe
  to build until that lands. The rules, named refusals and the bounded cost are in
  [construction](construction.md). A fluid door or verge column
  still refuses the lot, now by name `LiquidAccess`; preparing it is backlog.
- Village paths and farmland on a needed lot may be removed, but the road
  network must stay connected: a road crossing the lot is re-laid around it.
- A grown planted lumberjack tree on a needed lot is felled as a whole tree,
  found by trunk connectivity from its registered root. Accepted risk: a player
  log touching that trunk, whose history has expired, is felled too, with its
  drops conserved.

## Parallel construction — owner, 2026-11-17

> 1. по числу строителей  2. если есть заявка на которую есть ресурсы - поднимаем
> её вверх. иначе оставляем порядок очереди как есть  3. да

Status: design decided; implementation is pulled before the freeze by D21
(2026-11-17 ~22:30, superseding the first "after the freeze" answer). The
narrow interface lands first: code outside the construction module reads the
record's projects and pending orders only through
`crates/mc-core/src/settlements/construction_access.rs`, and the code-health
ratchet ceiling for direct field accesses is 0 (owner D9.3, 2026-11-17). Until
the parallel behaviour lands,
[construction queue order](#construction-queue-order--owner-2026-11-17) holds.

- A settlement builds as many projects at once as it has available builders.
- An order whose materials are already in storage moves ahead of unfunded orders;
  otherwise the queue keeps its entry order.
- A daughter outpost is built in parallel with its parent's projects.
- The settlement record changes shape with no adapter: an older world with a
  settlement refuses to open. Whether a specific world (such as the test server)
  is kept, and how, is an owner decision before deploy.
- It is built behind a narrow settlement-construction interface: code outside
  the construction module does not read or write the record's project or pending
  order directly (owner, 2026-11-17, with the code-health ratchet that counts
  such accesses).

## Upgrade clears its own new cells — owner D18, 2026-11-17

> Б до заморозки

- An upgrade clears natural forest on its own new cells before the freeze,
  merged together with D7 Slice A.
- This is the 2026-11-08 [forest site preparation](#forest-site-preparation--2026-11-08)
  decision applied to upgrades, with the same fence:
  - the block matches this world's generator;
  - no retained player placement record;
  - persisted lumberjack roots protected, except the D10 needed-lot rule;
  - a whole planted tree is felled by trunk connectivity (the existing proof);
  - drops go to settlement storage with no loss or duplication.
- The upgrade rule refuses only what that fence protects: player-touched cells,
  persistent leaves, unknown or imported generator evidence, a protected root
  halo, and the nearby-live-player veto when block history is disabled.
- The order path stays FIFO; a refusal leaves the owner order pending.

