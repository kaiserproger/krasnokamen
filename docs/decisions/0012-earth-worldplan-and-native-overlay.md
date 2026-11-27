# ADR 0012 — Earth WorldPlan, immutable base and native overlays

**Date:** 2026-10-01  
**Status:** Proposed; not wired into the server. Adoption needs its own owner
decision and acceptance. Revisited against the code on 2026-11-27.

## Current truth

- Serving terrain is the vanilla-like generator of
  [ADR 0008](0008-overworld-density-router.md). No server, CLI or core path
  uses an Earth plan: `rg 'earth::' crates/mc-core crates/mc-edge` finds nothing.
- The prepared-planet library survives from the retired v1 prototype in
  `crates/mc-worldgen/src/earth/` (about 15,800 lines with tests: definition,
  tectonics, climate, hydrology, erosion, region plans and cache,
  materialization, `EarthRuntime`). Its only consumer is the offline image tool
  `worldgen_mosaic` (`crates/mc-worldgen/src/bin/worldgen_mosaic.rs:47` →
  `render_earth_mosaic`, `crates/mc-worldgen/src/mosaic.rs:100`).
- The v1 commands `world prepare`, `world bounds set`, `world inspect --check`
  and `world prepare-region` do not exist in the current operator CLI
  (`crates/mc-edge/src/operator_cli.rs`, `WorldCommand`).

## Proposal

An Earth-like world has three layers and one runtime owner:

1. an immutable, validated world definition and identity plus a prepared
   physical base, published atomically;
2. region plans derived lazily from that base: a bounded, discardable cache,
   never a durability mechanism;
3. a native persistent overlay owned by the existing core store and tick WAL
   ([ADR 0013](0013-core-model-v2.md)).

Adoption must keep these rules: no Earth or Anvil serving fallback; a
definition or identity mismatch refuses to open the world; durability ordering
is the core's and is not strengthened or weakened silently; the planner never
becomes an entity, plugin or gameplay authority.

## Open

Whether to adopt the proposal, and whether to keep the unwired library until
then, is an owner decision; this record does not settle it. The full W0–W5
proposal with the v1 prototype notes and measurements is in Git:
`git show 973e6e60e:docs/decisions/0012-earth-worldplan-and-native-overlay.md`.
