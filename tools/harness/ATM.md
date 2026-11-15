# ATM local carry and graphical structure gate

This is experimental adapted **data**, not NeoForge/modpack compatibility. No
mod code executes in the server. Keep jars, licences, converted packs, worlds
and footage local under `targets/`; never stage or redistribute them. Publication
requires the owner's approval. The permitted local-evaluation licences include
MIT/Apache/BSD/CC0/CC-BY and LGPL/GPL/MPL, but not NC, ND, AI-use restrictions or
missing licences. The jar's metadata wins over the metadata-only ATM census.

## Carry

Build `horizon-port-pack` from Horizon commit
`0a4dc7ca2923da8f3629c57706e44ad4bc59915f` in a private checkout/target directory.
On the owner host use Main's absolute `tools/dev/mem-run.sh -m 6G` for builds and
harness commands. No server or client release build is performed by this tool;
Main supplies those pins separately.

Create a local JSON input manifest with a nonempty `packs` array. Each row has:

- `id`, `version`: exact ATM census entry and jar metadata version;
- `jar`: absolute locally downloaded jar path;
- `sha256`, `source_url`: pinned jar digest and provenance;
- `licence_entry`: jar's `META-INF/neoforge.mods.toml` (or `mods.toml`);
- `licence_sha256`: digest of that exact metadata entry;
- `licence`: normalized licence, checked against metadata (MVS 5.1.3 is
  `LGPL-3.0`, not the census's MIT);
- `structures`: exact namespaced building IDs to generate, at most 12 across
  the selection. Select overworld buildings with constructed block palettes;
  the current scenario does not travel to Nether/End.

```sh
python3 -m tools.harness.atm_carry \
  --inputs /absolute/targets/atm/inputs.json \
  --horizon /absolute/targets/atm/build/debug/horizon-port-pack \
  --horizon-root /absolute/targets/atm/horizon \
  --output /absolute/targets/atm/converted
```

The output directory must be new. A rerun uses a new directory: it never
merges stale files from an older conversion into the pack. Conversion runs
Horizon `check` (1.21.1 recipes/loot/custom worldgen) followed by
`upgrade-datapack` (remaining 26.3 registry changes, including structure spawn
counts). `upgrade-datapack` alone is **not** a 1.21.1 converter. SHA mismatches,
licence mismatches, invalid JSON, wrong source format, lost/unsupported rules,
dropped processors or missing source data fail with retained receipts.

Outputs per pack:

- `datapack-26.3`: format 121 data, including explicitly recorded Horizon
  extensions. **Not a vanilla-only data pack.**
- `server-adapted`: converted 26.3 gameplay data with original worldgen, NBT,
  tags and custom library data preserved byte-for-byte for Krasnokamen's
  internal 26.1.2 codec. It is intentionally described as **mixed grammar**,
  not disguised as vanilla 26.3 compatibility. The server must implement the
  original custom Moog structure/placement/pool/processor semantics.
- `conversion.json`: exact source/converted file hashes and counts, jar
  licence/dependencies, Horizon reports, required extensions and limitations.
- root `result.json`: manifest consumed by the graphical gate. A successful
  carry is `converted-unproven`, never gameplay acceptance.

The converter does not install client assets, custom block/item registrations
or execute Java dependencies. The original assets remain in Horizon's carry
folder for accounting. Unsupported registration requirements reject the
vanilla-client structure path. NBT versions are preserved by Horizon; the
receipt does not claim a DataFixer/oracle comparison or full feature fidelity.

### Explicit partial fixtures

`python3 -m tools.harness.atm_partial --full /absolute/full/result.json --output
/absolute/targets/atm/partial` preserves the full carry and emits separately named
no-Nether and no-Nether/no-loot variants. Every exclusion records path, original
SHA-256 and reason. `--crystal-only` instead retains exactly the crystal structure,
set, two pools and two NBT templates; all other data files are listed as excluded.
LootTable references inside templates remain unchanged. No-loot variants are
**generation tests only**: the Horizon runner stops after native-start evidence
with `generated-only`, without entering its photograph session; the native
photograph scenario refuses them. Neither status is footage or loot acceptance.

### Horizon offscreen client

The independent Horizon GPU client uses the same native export/start/block
evidence, without the native client's MCP wrapper:

```sh
python3 -m tools.harness.atm_horizon \
  --server /absolute/release-pin/krasnokamen \
  --client /absolute/horizon-client --ctl /absolute/horizon-ctl \
  --resources /absolute/horizon-resources/26.3 \
  --cache /absolute/vanilla/26.3 \
  --manifest /absolute/targets/atm/converted/result.json \
  --output /absolute/targets/atm/run
```

It owns a private Xvfb and runs Horizon `--offscreen` (GPU rendering, not a
headless protocol bot). Use `--vanilla-root /absolute/vanilla/26.3` instead of
`--manifest` to calibrate with a vanilla village; this is explicitly not ATM
evidence. `--without-packs --candidates /absolute/positive/candidates.json`
selects the same pack-absent negative control. Wrap graphical commands in Main's
absolute `scene-slot.sh` followed by `mem-run.sh -m 6G` on the owner host.

## Fresh world, real client, generated starts

Set the normal native graphical environment documented in
[real-client gates](../../docs/development/real-client-gates.md), including
immutable `KRASNOKAMEN_SERVER_BINARY`, `KRASNOKAMEN_CLIENT_BINARY` and the server
content cache. The pin must include the namespace loader, generator semantics,
startup/recovery pack lifecycle and namespaced `/locate` admission.

```sh
export KRASNOKAMEN_ATM_MANIFEST=/absolute/targets/atm/converted/result.json
python3 -m tools.harness run scenario native-atm-structures \
  --client native --windowed --timeout-seconds 600
```

The harness owns/reaps both server/client sessions and their private Xvfb.
Each run uses seed 20261115 and a fresh world. It verifies every converted pack
hash before copying packs into `<world>/datapacks/` before startup. No fixture
places a structure or a block. It visits `/locate` candidates with a real
client, saves and stops, exports the actual native world through the pinned
server's offline `world export-anvil`, and requires:

1. a persisted start with the **exact selected ID**, native anchor and bounds;
2. at least eight constructed blocks of at least two kinds from that
   structure's template palette inside its generated bounds;
3. after reopening the same world, those sampled blocks received by the real
   client at the saved coordinates;
4. a completed actual-window GPU screenshot aimed at the generated bounds.

`candidates.json`, `generated-sites.json`, the export/receipt, both sessions'
logs and `photographs/structure-*-coordinates.json` retain the causal evidence.
The normal harness `result.json` retains commands, exit codes, pins and errors.
Inspect the retained PNGs yourself. Sampled palette matches do not prove every
block, processor, loot table, mob or structure in a mod works. Only selected
IDs are covered. Exact IDs plus generated starts prevent an unrelated vanilla
building from satisfying the check. No screenshot means no graphical claim.
Run at least three fresh repetitions; retain each receipt separately.

## Negative control

Use the identical pin, manifest and seed with a prior successful run's
`candidates.json`. No packs are installed in the new control world; it visits
the same coordinates, exports, and must **fail at missing persisted selected
starts**, not pass on unrelated terrain or a `/locate` prediction:

```sh
export KRASNOKAMEN_ATM_WITHOUT_PACKS=1
export KRASNOKAMEN_ATM_CANDIDATES=/absolute/previous-run/candidates.json
python3 -m tools.harness run scenario native-atm-structures \
  --client native --windowed --timeout-seconds 600
```

A launch/connection/export failure is not a meaningful negative control.
Unset both control variables before another positive run. The unit regression
`python3 -m unittest tools.harness.test_atm_carry` also proves absent/wrong starts
and terrain-only blocks fail, using original synthetic fixtures, not pack bytes.
This is offline tool coverage, not a substitute for the real-client runs.

Only after live evidence and visual review may the caption say
“adapted data from N ATM10 mods, EXPERIMENTAL”, with N the number of actually
proved packs. Never use “N of 495”: 495 is the metadata jar census.
