# Constantinople fixture

A deterministic imported-city functional fixture: population, street walking and
native save/reopen. It gives **no throughput or performance verdict**. Keep every
third-party archive, world, census/plan payload and receipt in ignored
`.analysis/`; never commit map or Mojang bytes. Large scans and the import itself
take Main's GO; heavy ones run inside `tools/dev/mem-run.sh -m 6G`
([heavy commands](heavy-commands-and-memory.md)). The map input is a third-party
map; this repository does not ship it.

```sh
python3 -m tools.coreperf.constantinople census --world CANONICAL_ANVIL_EXPORT \
  --output .analysis/maps/constantinople/census.json
python3 -m tools.harness run constantinople -- --plan DIR --world NATIVE_WORLD \
  --server-bin ABS_BIN --bots-bin ABS_BOTS --vanilla-data-dir ABS_DATA --seed SEED
```

`python3 -m tools.coreperf.constantinople --help` lists the subcommands
(`census`, `preservation`, `plan`, `smoke`, `native-smoke`).

## Offline subcommands

- `census` scans a stopped native world's `krasnokamen world export-anvil` output
  (`--counts-only` skips per-cell `terrain.json` for large comparison worlds).
- `krasnokamen --config FILE world census --source STOPPED_NATIVE --receipt OUT.json`
  counts native entity roots and passengers, villagers, stored chunks,
  settlements and persisted memberships/households directly. The config's
  seed/generation identity must match, a journal tail newer than the checkpoint
  is refused, and receipts lie outside the world.
- `krasnokamen --config FILE world probe --source STOPPED_NATIVE --receipt OUT.json
  --block X,Y,Z` (repeatable `--block`) reads one stopped world read-only under
  the same identity/journal-tail rules and reports each requested cell's stored
  block state and typed container slots plus every stored player inventory and
  every stored item entity, so custody and duplication questions are answered
  from the saved image instead of a live counter.
- `preservation --source STOPPED_IMPORT_INPUT --world CANONICAL_ANVIL_EXPORT
  --output .../preservation.json` compares import input with the stopped native
  export across every dimension (entity/passenger and block-entity counts, stored
  container and equipment totals including nested containers, each player's
  inventory, ender chest and equipment). A mismatch writes a FAIL receipt and
  exits nonzero. It proves physical-item count preservation, not byte-exact or
  cosmetic/RNG parity.
- `plan --terrain PATH --output DIR --villagers N --animals 64 --bots K --seed 1453`
  treats N as the target total (tops up existing villagers) and emits seeded
  exclusive home/job pairs, planting commands, green-area placements, a cardinal
  street graph and closed bot routes.

## `run constantinople`

The profile reuses the catalog-load process ownership ([catalog load](catalog-load.md))
with an empty plugins directory and native console commands:

1. Move spawn to each receipted quarter-grid centre, `settlement create`, and
   explicitly `settlement bind` each cohort UUID. Neither settlement creation nor
   membership is automatic, and bind alone forms no household.
2. Restore the fixture spawn and wait 2400 actual game ticks on the dashboard's
   event-driven clock endpoint.
3. Stop cleanly, run the native census, reopen and re-check.

The probe binary (`player-scale-bots` read-only native snapshot mode) never
connects a player. Each stopped phase verifies every authored villager UUID and
exact settlement assignment; the reopen census must match the stopped run
(`--reopen-only` re-checks without repeating create/bind). Native snapshot
decoding treats `command_storage` as its named NBT compound: rebuild the probe
after changing native global encodings rather than using an older binary that
interprets this domain as JSON.
