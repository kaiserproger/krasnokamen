# returned — generator feature gap, not a fixable defect

The row records a limitation, not a broken path: a selected pack carrying custom
biomes, dimension types, noise settings or features is refused by name
(`1d8e57dd8`; the fail-closed half is DATAPACK-WORLDGEN-OVERRIDES), and the
server never silently claims to generate that terrain. Honouring it means the
worldgen generator consumes custom `worldgen/biome`, `dimension_type`,
`noise_settings` and `feature` resources, plus the terrain epoch/identity rules
around them — a design epic, not a bounded owner-module patch, and it changes
what the server promises rather than a defect to correct.

No regression ref exists (the row has `regression = []` by construction), so
there is nothing to red/green without first building that feature. Owner/Main
decision needed on whether general terrain customization is in scope; until
then the documented refusal is the intended behaviour.
