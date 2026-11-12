# Breeding and food

Villagers need food and a suitable spare bed to raise a child, while farmers help supply that food.

See [jobs](jobs.md), [construction](construction.md), and [production](production.md) for work assignments, buildings, and storage production.

## Food and beds

Each parent must be an adult, awake, not panicking, and willing: at least 12 available food points. Bread is worth 4 points; carrots, potatoes and beetroot are worth 1 each; wheat itself is worth none. Food reserved for production is not available for breeding. Players can also drop food for villagers to pick up.

A birth needs an unclaimed, reachable, complete bed (both halves; no headroom is required, as in vanilla `VillagerMakeLove.takeVacantBed`), within the settlement's housing scope. A parent's own bed cannot be the spare bed. For settlement residents both the home bed and the spare bed must be roofed (solid collision anywhere above the bed head): residents shelter instead of sleeping in the open, so they never claim an open-air bed. Stored-food rations go only to residents with a home bed, so a settlement whose beds all stand in the open draws no rations and has no births. Unbound villagers keep vanilla open-air beds. Villagers can share food with nearby villagers who are short; sharing also respects production reservations.

Ration and spare-bed searches have their own 40-tick cadence. A search that exhausts the shared path budget cannot immediately repeat on every tick and starve the resident's work or delivery route.

## Farming

A settlement-bound farmer needs a job in a certified Farmer workshop to harvest; a saved Farmer profession alone is not enough. While working, awake farmers harvest mature wheat, carrots, potatoes and beetroot and replant them. The complete yield must fit in the farmer's inventory. Farming also requires the villager-farming mob-griefing rule.

A bound farmer sweeps the certified farm volume, checking at most 128 cells per decision, and navigates to the nearest eligible mature crop found in the sweep. Harvesting and replanting take place within 2 blocks of the crop. The scan restarts when the selected crop changes, after harvest or an empty sweep, or when the certificate changes. An unbound farmer instead scans locally within 4 blocks horizontally.

Farmers bake 3 wheat into 1 bread when harvesting. Bound farmers bake up to their three-bread ration, keep up to 3 wheat for each missing loaf, and deliver surplus produce to settlement storage. Production reservations can protect carried ingredients or defer a harvest or bake; see [production](production.md).

## Wool

Eligible adult residents shear adult sheep with real shears and carry the colored wool to storage. Unemployed residents and shepherds can do this as work; other eligible professions can also help when construction needs wool. They can obtain shears from storage or craft them from two iron ingots. Shearing requires mob griefing and a working resident; sheep searches use active chunks inside the settlement.
