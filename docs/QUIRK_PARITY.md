# Vanilla differential oracle (quirk, farm and gameplay parity)

Krasnokamen must behave like vanilla where players can see it: the quirks they rely
on (TNT duping, quasi-connectivity, update order), farm output rates, and ordinary
gameplay (block placement and breaking, containers, smelting, redstone timing,
entities, state across a restart). This gate proves it against **real Mojang
vanilla**, not memory or wiki text, and is how differential tests replace unit
tests (owner D128): a behaviour covered here needs no unit test that restates it.

Real vanilla runs **once per case, at record time**. Every gate run replays
Krasnokamen against that recording without starting vanilla.

```sh
# the gate: verify the corpus, run Krasnokamen only, compare (needs a built mc-core test binary)
python3 -m tools.harness run quirk-parity [-- --scenario ID ... --family F ...]
# record every scenario not recorded yet (real vanilla, twice per case)
python3 -m tools.harness run quirk-parity -- record [--scenario ID ...] [--fuzz-seed N --fuzz-cells C --fuzz-style S]
# a recorded case changes only through an explicit, versioned re-record with a reason
python3 -m tools.harness run quirk-parity -- re-record --scenario ID --reason "why the recording must change"
# corpus integrity only (no servers)
python3 -m tools.harness run quirk-parity -- verify
```

The module runs the same way directly: `python3 -m tools.harness.quirk_parity
[replay|record|re-record|verify] --out DIR ...`; every command takes
`--mc-version` (default `26.3`, the canonical release) and `--corpus DIR`.

## Toolchain and inputs

| what | default | override |
|---|---|---|
| Java 25 | `<work>/tools-cache/jdk/temurin-25/bin/java` (Eclipse Temurin from the Adoptium API, SHA-256 checked) | `--java`, `KRASNOKAMEN_ORACLE_JAVA` |
| vanilla server | `<work>/tools-cache/minecraft/<version>/server.jar` (the Mojang bundler, SHA-1 from Mojang's version manifest) | `--jar` |
| corpus | `<work>/oracle-corpus/<version>/` | `--corpus`, `KRASNOKAMEN_ORACLE_CORPUS` |

`<work>` is the owner host's work directory (`KRASNOKAMEN_WORK` elsewhere). Java
is never taken from `PATH`. Only recording needs Java and the jar; replay needs
neither. Recording 26.3 worlds also needs the local `data/vanilla` content cache
(`krasnokamen content import`): 26.3 saves a block in its default state as the
bare id, and the decoder expands it from that release's `reports/blocks.json`.

Mojang bytes never enter Git. The jar, the recorded worlds and the corpus stay
outside the repository; the corpus holds observations derived from vanilla runs
(block states, container totals, entity state), handled like the other derived
facts of [ADR 0022](decisions/0022-content-without-java.md).

## Record once, replay always

`record` runs every selected scenario that this tree has not recorded yet on two
fresh vanilla worlds. Both recordings must accept every command and pass the
scenario's `vanilla_expect` self-check; recordings that differ in shape (phases,
cells, trace lengths) are refused. Observable paths whose values differ between
the two recordings become **wildcards** (vanilla itself does not determine them);
every other path must match exactly or within the scenario's own epsilon. Seeded
farm variants are compared as production distributions instead: both recordings
of every seed are vanilla samples.

`replay` (the default command and the profile's gate):

1. verifies the corpus (below); any integrity problem fails before Krasnokamen runs;
2. requires every scenario file to be recorded with an unchanged scenario hash;
3. runs the recorded scenarios through Krasnokamen's native tick in-process
   (`crates/mc-core/src/quirk_parity_tests.rs`, the `#[ignore]`d test the
   harness drives with `KRASNOKAMEN_QUIRK_SCENARIOS` / `KRASNOKAMEN_QUIRK_RECEIPT`);
4. compares observable state, never log text, and groups the differences of each
   independent timeline (each cell of a multi-box scenario) under the **first
   differing path**, in time order: phases, a phase's per-tick trace before its
   end snapshot, then blocks, containers, entity counts, entity positions and
   fields. The group's **mismatch id** is the first 12 hex digits of
   `sha256("<case id>\n<path>")`, so the same divergence keeps its id across runs.

`DIR/receipt.json` lists per case `status` (`pass`, `fail`, `known-divergence`,
`findings` for fuzz), diffs, `mismatches` (id, path, both values, rows that
follow from it, `new`/`known`), the corpus integrity summary and every
Krasnokamen command rejection; the top-level `mismatches` list collects them
all. `DIR/krasnokamen/` holds the replayed scenario files, the driver's own
receipt and `krasnokamen.log`. Through the harness the case verdicts and
mismatch ids are also in `result.json` under `details.quirk_parity`.

Nothing sleeps: vanilla waits on its own log lines ("Sprint completed",
"Saved the game"), Krasnokamen on tick counts.

## Corpus and cheat guards

```
<corpus>/<version>/manifest.jsonl         append-only; each line names one case version, its file's
                                          SHA-256, its scenario hash and the SHA-256 of the line before
<corpus>/<version>/cases/<case>/vNNN.json one immutable file per recorded version: the resolved scenario,
                                          both recordings, wildcards, jar and Java identity
<corpus>/<version>/manifests/             the whole manifest as it stood before each re-record
tools/harness/quirk_parity/corpus-lock.jsonl   (Git) one line per case version this tree replays
```

- **Append-only corpus.** Replay fails when a locked case version is missing or
  its file no longer has the recorded hash, when a manifest line was edited,
  removed or reordered (hash chain), or when an older version's lock line
  disappeared. Adding cases is free. A case changes only through `re-record
  --reason`, which writes a new version, keeps the old version and its lock line,
  and copies the old manifest to `manifests/`. Cases recorded from another tree
  are anchored (lock line appended) without re-running vanilla when their
  scenario hash matches.
- **Scenario hash.** Everything both servers execute and the gate compares is in
  it: setup, phases, boxes, seeds, compare rules, tolerances, self-checks. Changing
  any of them, a tolerance included, needs a re-record with a stated reason.
- **Fix and promotion commits** (`ledger promote`, `tools/harness/quirk_parity/guard.py`):
  for every core fix (commit or `patch:`) the promotion counts `#[test]`-family
  attributes, `assert!`-family macros and `#[ignore]` attributes in the fix's own
  diff of `crates/**/*.rs` (comments excluded, all crates together so moved tests
  count once) and refuses it when tests or assertions drop or `#[ignore]` grows,
  or when the fix removes a `corpus-lock.jsonl` line. The baseline is the tree
  before the fix; its census, the fixed tree's census and the delta are recorded
  in `promote.json` as `test_guard`, so no tracked count file needs editing.
  Test-prune commits are separate, listed and reviewed commits, never a row's fix.

The guards' unit suites, each with a negative control (edited or removed case,
dropped test, dropped assertion, added `#[ignore]`, removed lock line):
`python3 -m unittest tools.harness.test_quirk_oracle tools.harness.test_quirk_compare`.

## Known differences

Fix the implementation, never the recording or a tolerance. A difference that is
intended, or known and not fixed yet, is listed in
[`known-diffs.json`](../tools/harness/quirk_parity/known-diffs.json) per case:
the reason (with repro or ledger row) and the exact mismatch ids it covers. Those
mismatches report `known-divergence`; any other mismatch of the case fails, and
so does a listed id that no longer occurs, so the entry is removed when the gap
closes. Never make Krasnokamen stricter than vanilla to remove a difference.

## Scenario file (`tools/harness/quirk_parity/scenarios/*.json`)

```json
{
  "id": "qc-piston-bud", "family": "quasi-connectivity", "description": "...",
  "box": [x1,y1,z1,x2,y2,z2],              // or "boxes": [[...],...] - independent cells on one timeline
  "prelude": true,                         // default: spawn_mobs/random ticks/time/weather frozen identically on both sides
  "seed": 1, "seeds": [1,2,3],             // seeds => one case per seed (rate scenarios)
  "setup": ["fill ...", "setblock ..."],   // before tick 0
  "settle_ticks": 1,                       // ticks run after setup, before the first snapshot (default 1)
  "phases": [{"name": "p", "commands": ["..."], "ticks": 20},
             {"name": "q", "restart": true, "ticks": 1}],   // restart: clean stop and reopen before the phase
  "trace": false,                          // snapshot every tick of every phase (small scenarios only)
  "compare": {"blocks": "exact", "entities": "counts", "containers": "exact",
              "phases": ["p"], "entity_types": ["minecraft:tnt"], "ignore_entity_types": [],
              "entity_tolerance": 0, "ignore_blocks": ["minecraft:fire"],
              "entity_positions": {"types": ["minecraft:tnt"], "epsilon": 0.05},
              "entity_fields": {"types": ["minecraft:pig"], "fields": ["health", "fuse"], "epsilon": 0.01}},
  "vanilla_expect": [                      // record-time self-check: vanilla shows what the scenario encodes
    {"phase": "p", "check": "block", "pos": [x,y,z], "equals": "minecraft:piston[extended=true,facing=east]", "cell": 0},
    {"phase": "p", "check": "count", "key": "minecraft:tnt", "min": 1, "max": 1, "cell": 0}],
  "rate": {"metric": {"kind": "container_items", "item": "minecraft:iron_ingot"},   // farm rate scenarios
           "baseline_phase": "warmup", "phase": "run", "tolerance_pct": 5, "min_vanilla": 1},
  "allow_krasnokamen_command_errors": false
}
```

Both servers execute the same console command strings. Vanilla: a void superflat
world, `tick freeze`, `forceload` of every box, exactly N ticks per phase, then
`save-all flush` and the saved Anvil files are decoded
(`tools/harness/quirk_parity/anvil.py`: blocks, entities, container items). From a
frozen world `tick sprint N` runs N + 1 ticks (the sprint check reads the frozen
tick's stale state and does not count the first unfrozen tick), so the driver
sends `tick sprint N-1`, or `tick step 1` for one tick (an auto command block at
the bottom of the first box's column says when that tick ran, since console
commands run between ticks and the step logs nothing), and fails unless
`time query gametime` advanced by exactly N. A restart phase stops vanilla with
`stop` (every loaded chunk saved) and reopens the same world; vanilla does not
persist `tick freeze`, so restart scenarios must have settled before the restart.
Krasnokamen: the same commands through the console command path, `run_tick` per
tick, the same snapshot format. A command Krasnokamen rejects but vanilla accepts
is a failure; fill a container through `setblock` SNBT unless the scenario tests
commands that follow `setblock` in the same tick (`container-command-same-tick`).

`rate.direction: "drop"` measures a metric that shrinks (consumption/harvest) as
baseline minus end value. Metric kinds: `container_items` (all hoppers, chests,
barrels...), `item_entities`, `entity_count`, `block_count` (by block name),
`block_state_count` (`equals`: exact `name[sorted props]`). Rate comparison: mean
production per run, Krasnokamen vs every vanilla recording of every seed; passes
inside `tolerance_pct` of the vanilla mean or two standard errors of the noise,
whichever is wider. Every recording of every seed must pass `vanilla_expect`.

Blocks are compared as full `name[sorted props]` states (non-air only); entities
by count per type (item entities per item id, weighted by stack size);
`entity_positions` and `entity_fields` compare sorted per-type lists within their
epsilon. Randomised quantities (TNT launch direction, explosion block selection)
belong in counts, not positions; keep exact-state scenarios shorter than any fuse
that ends in an explosion (80 ticks). Missing or extra phases, unequal cell lists
and missing or unequal traces are differences, never silently skipped.

## Scenario families

| family | scenarios | what must match vanilla |
|---|---|---|
| quasi-connectivity | `qc-*` | piston/dropper/dispenser powered through the block above, only reacting on a neighbour update |
| tnt-duping | `tnt-*` | BUD TNT duplication through piston push order (destroy-list/shape-update), controls |
| update-order | `fuzz-*` (recorded from `--fuzz-seed`) | seeded piston/TNT/rail/carpet/coral/observer contraptions: every vanilla duplication cell must match, other differences are `findings` |
| zero-tick | `fuzz-timing-*` | 0-tick behaviours that still exist |
| redstone | `redstone-*` | lamp, repeater, torch, comparator and dust timing per tick |
| blocks | `block-*`, `fluid-flow`, `gravity-blocks` | tool-less drops, attached blocks popping, neighbour shape updates, fluid spread, falling blocks |
| containers | `container-*` | hopper transfers, locking and item pickup |
| smelting | `smelting-*` | furnace, blast furnace and smoker cook times, fuel and automation |
| entities | `entity-*` | fall damage, item landing and merging, TNT fuse, minecart on powered rails |
| persistence | `save-restart` | blocks, containers and entities across a clean stop and restart |
| farm | `farm-*` | production over several seeds within tolerance |

Player-quirk probes (`player_scenarios/*`, diagonal sprint speed and attribute
swapping) run through `tools/harness/quirk_parity/player.py` and the
`matrix-random --differential` profile against a live vanilla server; they are not
part of the recorded corpus.

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

