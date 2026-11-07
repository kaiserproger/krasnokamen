# Quirk and farm parity gate

Solaris deliberately keeps vanilla behaviours that players rely on (TNT duping,
quasi-connectivity, update order, attribute swapping, diagonal sprint speed) and
must not change farm output rates (iron, villager breeder, mob, crop farms).
This gate proves that against a **real Mojang 26.1.2 server** run on the same
construction, instead of from memory or wiki text. Vanilla is the oracle; a
scenario that vanilla itself does not show is a failing scenario.

```sh
# whole gate (needs .analysis/server.jar = vanilla 26.1.2 bundler, Java 25, a built mc-core test binary)
python3 -m tools.harness run quirk-parity -- --family quasi-connectivity
python3 -m tools.harness.quirk_parity --out DIR [--scenario ID] [--family F] [--vanilla-only|--solaris-only]
python3 -m tools.harness.quirk_parity --out DIR --fuzz-seed 1 --fuzz-cells 256 --fuzz-style dense|mixed|bud|slime3d
```

Receipts: `DIR/receipt.json` (per scenario `status`: `pass`, `fail`,
`known-divergence`, `findings` for fuzz; diffs, vanilla self-check failures,
wall times), `DIR/vanilla-results.json`, `DIR/solaris/receipt.json` and the
`vanilla.log` / `solaris.log` of every run. The harness profile `quirk-parity`
(`tools/harness/profiles.py`) wraps the same module.

## How a scenario runs

Both servers execute the **same console command strings** (`setblock`, `fill`,
`summon`, `gamerule`, `time`, `weather`) from the scenario file:

* vanilla: a void superflat world, `tick freeze`, `forceload` of every box,
  `tick sprint N` per phase (completion is the server's own "Sprint completed"
  log line), then `save-all flush` and the saved Anvil files are decoded
  (`tools/harness/quirk_parity/anvil.py`: blocks, entities, container items).
* Solaris: the real native tick in-process
  (`crates/mc-core/src/quirk_parity_tests.rs`, `#[ignore]` test run by the
  harness with `SOLARIS_QUIRK_SCENARIOS` / `SOLARIS_QUIRK_RECEIPT`): the same
  commands through the console command path, `run_tick` per tick, same
  snapshot format. A command Solaris rejects but vanilla accepts is a failure.

Nothing sleeps: every wait is a specific server log line / tick count.

## Scenario file (`tools/harness/quirk_parity/scenarios/*.json`)

```json
{
  "id": "qc-piston-bud", "family": "quasi-connectivity", "description": "...",
  "box": [x1,y1,z1,x2,y2,z2],              // or "boxes": [[...],...] - independent cells on one timeline
  "prelude": true,                         // default: spawn_mobs/random ticks/time/weather frozen identically on both sides
  "seed": 1, "seeds": [1,2,3],             // seeds => one run per seed on both servers (rate scenarios)
  "setup": ["fill ...", "setblock ..."],   // before tick 0
  "settle_ticks": 1,                       // ticks run after setup, before the first snapshot (default 1)
  "phases": [{"name": "p", "commands": ["..."], "ticks": 20}],
  "trace": false,                          // snapshot every tick of every phase (small scenarios only)
  "compare": {"blocks": "exact", "entities": "counts", "containers": "exact",
              "phases": ["p"], "entity_types": ["minecraft:tnt"], "ignore_entity_types": [],
              "entity_tolerance": 0, "ignore_blocks": ["minecraft:fire"],
              "entity_positions": {"types": ["minecraft:tnt"], "epsilon": 0.05}},
  "vanilla_expect": [                      // the oracle self-check: the quirk exists in vanilla
    {"phase": "p", "check": "block", "pos": [x,y,z], "equals": "minecraft:piston[extended=true,facing=east]", "cell": 0},
    {"phase": "p", "check": "count", "key": "minecraft:tnt", "min": 1, "max": 1, "cell": 0}],
  "rate": {"metric": {"kind": "container_items", "item": "minecraft:iron_ingot"},   // farm rate scenarios
           "baseline_phase": "warmup", "phase": "run", "tolerance_pct": 5, "min_vanilla": 1},
  "known_divergence": "one-line reason + repro; the marker itself fails the gate once Solaris matches",
  "allow_solaris_command_errors": false
}
```

`rate.direction: "drop"` measures a metric that shrinks (consumption/harvest) as
baseline minus end value.

Metric kinds: `container_items` (all hoppers/chests/barrels/...), `item_entities`,
`entity_count`, `block_count` (by block name), `block_state_count` (`equals`: exact
`name[sorted props]`, e.g. `minecraft:wheat[age=7]`). Rate comparison: mean production per seed,
Solaris vs vanilla; passes inside `tolerance_pct` of the vanilla mean or two
standard errors of the seed noise, whichever is wider.

Blocks are compared as `name[sorted props]` (non-air only); entities by count
per type (item entities per item id, weighted by stack size); randomised
quantities (TNT launch direction, explosion block selection) are compared as
counts, not positions. Keep exact-state scenarios shorter than any fuse that
ends in an explosion (80 ticks).

## Scenario families

| family | scenarios | what must match vanilla |
|---|---|---|
| quasi-connectivity | `qc-*` | piston/dropper/dispenser powered through the block above, only reacting on a neighbour update |
| tnt-duping | `tnt-*` | BUD TNT duplication through piston push order (destroy-list/shape-update), controls |
| update-order | `fuzz-*` | seeded piston/TNT/rail/carpet/coral/observer contraptions: every vanilla duplication cell must match, other differences are `findings` |
| zero-tick | `zerotick-*` | 0-tick behaviours that still exist in 26.1.2 |
| player | `player_scenarios/*` | diagonal sprint speed acceptance, attribute swapping (see `tools/harness/quirk_parity/player.py`) |
| farm-* | `farm-*` | production per hour over several seeds within tolerance |

Rules: never "fix" a divergence by making Solaris stricter than vanilla. A
Solaris divergence is either fixed with a regression test (small fix) or
recorded as `known_divergence` with an exact repro in the receipt/report.
Mojang bytes never enter Git: the jar and worlds stay under `.analysis/`.

## Known divergences

The tracked scenarios mark these divergences and contain their exact repros:

- [TNT duper](../tools/harness/quirk_parity/scenarios/tnt-duper-found-bud.json):
  instant piston movement lacks vanilla's moving-piston window.
- [TNT destroy-list](../tools/harness/quirk_parity/scenarios/tnt-bud-push-destroy-list.json):
  fluid and waterlogging differ around the push.
- [Mob kill chamber](../tools/harness/quirk_parity/scenarios/farm-mob-kill-chamber.json)
  and [gold farm](../tools/harness/quirk_parity/scenarios/farm-gold-nuggets.json):
  airborne wander can move mobs off the collection pad.
- [Villager breeder](../tools/harness/quirk_parity/scenarios/farm-villager-breeder.json):
  farmer harvest cadence differs.

A stale `known_divergence` marker fails once Solaris matches. See also the
[native test driver](../crates/mc-core/src/quirk_parity_tests.rs).

## Native block interaction regressions (26.1.2)

`mc-core`'s `interact_parity_tests` covers axe stripping/scraping/unwaxing,
held-use brushing of suspicious sand/gravel, decorated-pot and chiseled-bookshelf
custody, bounded sponge absorption, dragon-egg teleportation, eternal-fire
supports, spawn protection, and painting placement/support removal. These are
focused native regressions, not a claim that the graphical-client parity gate
has exercised every interaction.

Brush loot is revealed into retained block-entity NBT before completion; book
and pot contents use native inventory custody and hydrate from retained NBT.
Painting variants/directions are retained in entity metadata and native saves;
the protocol regression pins the registry-holder wire encoding to local
26.1.2 `javap` evidence.

`server.spawn_protection` (vanilla `spawn-protection`, default 16) protects the
Overworld spawn square only when the persisted operator list is nonempty.
Operators bypass it even when other operators are offline; zero disables it.

