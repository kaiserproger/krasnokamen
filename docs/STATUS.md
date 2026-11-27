# Project status

Where Krasnokamen stands. Update this page when a release ships, a known 1.0
gap opens or closes, or a transition opens or closes; day-to-day progress
belongs in commits and the [bug ledger](bugs/README.md). Last updated
2026-11-27 (counts measured on `973e6e60`).

## Release

- Published: **1.0.0-rc.5 "Pauline"** (core tag `v1.0.0-rc.5`, native client
  release with matching server and plugins); no later tag exists. Notes:
  [RC5](releases/v1.0.0-rc.5.md), all tags: [release notes](releases/README.md).
- RC5 checks on the published assets: checksums match; a cold Linux client
  install entered a new world, relaunched and re-entered it. On a 4-core
  Windows VM without a GPU the first two Play attempts timed out
  (`SP-START-TIMEOUT-SLOW-MACHINE`); walking and disconnecting were not checked
  there. No macOS client or package run exists (core lib tests in CI only).
- Next: an RC6-or-better public debut against the frozen
  [announcement criteria](DEFINITION_OF_DONE.md#finite-announcement-acceptance)
  (owner D91). Tags and publication need the owner's explicit authorization.

## Priorities

From the [owner contract](development/owner-contract.md), in order:

1. Ordinary vanilla-client gameplay and multiplayer.
2. The production plugin API and adapters.
3. Measured optimization, ECS, ownership and autoscaling.
4. Rare edges.

A lower item moves up only when it blocks ordinary play, saves or plugins.
Current owner focus (D123/D124, 2026-11-23/27): close the open ledger rows in
batches before the 1 December push of `main`; split `mc-core` into real crates
([ADR 0021](decisions/0021-build-time-and-crate-split.md)) with measured
incremental-rebuild effect; file-by-file core code-quality cleanup; documentation
consolidation.

## Work queue

The owner's open GitHub issues are the queue:

```sh
gh issue list -R kaiserproger/krasnokamen --author kaiserproger --state open
```

Known bugs, with status and regression checks, are rows under
[`bugs/rows/`](bugs/README.md): 198 open (82 client, 73 gameplay, 43 harness)
and 548 fixed on `973e6e60` (`python3 -m tools.harness ledger list --status open`).

## Open transitions

A transition replaces one mechanism with another and is open while both are in
the tree. A row exists only where the target is decided (an accepted ADR,
AGENTS.md or an owner decision); proposals and supported surfaces are listed
below the table and carry no rule. This is the only register: the change that
opens a transition adds its row, the change that deletes the old side deletes
the row. How a change treats an area with an open transition:
[adding a feature](development/adding-a-feature.md#open-transitions). The
commands below the table reproduce the counts.

| Transition (old → new) | State and evidence | Done when | Owner doc |
|---|---|---|---|
| Harness server launch: direct `subprocess.Popen`/asyncio spawn → `runtime.start_server` | 178 modules in `tools/harness/backends/`; 31 start the server through `runtime.start_server` (35 call sites). 8 call sites in 4 modules still start `krasnokamen` with `Popen` (`client_probes.py` 4, `village_autonomy_qa.py` 2, `seed_review.py` 1, `native_scale.py` 1 through its generic `spawn`), and `catalog_load.py` (also used by `catalog_spain.py`) starts it through `asyncio.create_subprocess_exec`. `runtime.start_server` itself is only `subprocess.Popen`: the sites share `wait_port` and `stop_process`, not a launcher. | One launch function owns start, readiness and teardown of the server, every backend calls it, and no backend runs `Popen` or `create_subprocess_exec` on the server binary. | [AGENT_TOOLING](AGENT_TOOLING.md#receipts-and-failure-diagnosis) |
| Release-binary pinning: the ledger's `.analysis/ledger-builds` registry → the `tools/dev/pin-release` manifest | `regressions._release_server`, `ledger probe` and `promote` consume `pin-release` manifests and verify binary checksum and exact tree against the pin's ledger registration; there is no private release build. `pin-release` still writes `.analysis/ledger-builds/<sha256>-<head>-<diff>.json` and the ledger's `_register_server` still binds receipts to `tree_fingerprint` through it. | The registry and `_register_server` are removed after pin manifests carry the ledger fingerprint and promotion checks the manifest directly. | [tools/dev README](../tools/dev/README.md), [bug ledger](bugs/README.md#the-fixed-rule-single-home) |
| Typed block and item identity: `.path() ==` comparisons and inline `"minecraft:` literals → kinds resolved once at load (target stated in the ratchet source, owner-approved 2026-11-07) | No shared `BlockKind` or `ItemKind` exists (only module-local kinds such as `vehicles::VehicleKind`). The enforced ceilings in `crates/xtask/src/code_health/ratchet.rs` are 2669 literals and 113 comparisons (original baselines 2441 and 88 at `c09e1391`); exceeding either fails the gate. Local cutovers (block tags, upper-tip states resolved once at load) are not a shared kind. | A typed kind exists, new code uses it, both counts are at or below their baselines and no new literal enters. No ADR owns this yet: write it before the first kind lands. | the ratchet source; [ADR 0016](decisions/0016-mob-capabilities.md) for mob types |
| Mob behaviour chosen by type id or name → capability flags ([ADR 0016](decisions/0016-mob-capabilities.md)) | Layers 1 and 2 (plugin registration) and sensors step 1 are implemented. `mc-core` production code still has 8 `== *X_TYPE` comparisons and 12 `matches!(<name>, "minecraft:…")` tests (grep count, includes legitimate uses). | No behaviour shared by more than one mob type is selected by comparing its id or name; each is a flag in `vanilla_capabilities`. | [ADR 0016](decisions/0016-mob-capabilities.md) |
| `mc-core` structure: features inside the big module cycle → each new feature its own module or crate with a narrow interface and one host registration point (owner, 2026-11-16); existing code split into independent crates (owner D124, 2026-11-27) | New features follow it since 2026-11-16. The code-health ratchet counts top-level modules in the largest cycle and fails above its ceiling of 186 (it checks neither edges nor submodule cycles); a submodule's references count as its host's, so the count is checked on the integrated tree. The registration point serves review and parallel lanes; it does not break a cycle. D124 starts the real crate split now, along ADR 0021, with measured incremental-rebuild effect. | Existing features are extracted along ADR 0021 and the big cycle no longer exists. | [ADR 0021](decisions/0021-build-time-and-crate-split.md), [adding a feature](development/adding-a-feature.md#new-features-are-modules) |

```sh
grep -l 'runtime.start_server(' tools/harness/backends/*.py | wc -l   # Popen sites: grep -n 'subprocess.Popen(' there
cargo run -p xtask -- code-health                                     # ratchet values and trend
grep -rnE '== \*[A-Z_]+_TYPE|\*[A-Z_]+_TYPE ==' crates/mc-core/src --include=*.rs | grep -v _tests.rs | wc -l
grep -rnE 'matches!\([^)]*name[^)]*, *"minecraft:' crates/mc-core/src --include=*.rs | grep -v _tests.rs | wc -l
```

Undecided proposals (no rule attached; each becomes a row only when the owner
decides it):

- Tick system graph ([ADR 0017](decisions/0017-tick-system-graph.md)): a
  one-system prototype (`EntityStep`, `crates/mc-core/src/tick/system_graph.rs`,
  run once from the entity phase). The ADR is not approval to migrate the rest of
  the tick; each later step needs the owner's approval and a measured benefit.
  The existing tick is the current path, not an old side.
- Earth WorldPlan ([decision G4](../DECISIONS.md#world-generation)):
  proposed, not ported. `crates/mc-worldgen/src/earth.rs` and `earth/` (15.8k
  lines with tests) still compile into `mc-worldgen`; only `mosaic.rs` and
  `bin/worldgen_mosaic.rs` use them. Port or delete is the owner's call.
- [ADR 0016](decisions/0016-mob-capabilities.md) Layer 3 (Wasm capability
  code), operator data-directory blueprints and persistent sensor subscriptions.
- Content without Java ([ADR 0022](decisions/0022-content-without-java.md)):
  the native RegistryData encoder `mc_data::registry_network` is proven byte for
  byte against vanilla captures but not wired into `content import`; waits on
  the owner's four questions in the ADR. The Java import and the singleplayer
  content bundle are the current path, not an old side.
- Off-screen settlement simulation ([ADR 0023](decisions/0023-offscreen-settlement-simulation.md)):
  accepted; migration transit and flows, game-time economy clocks and deferred
  production/consumption are integrated (slices 1–3). Construction labour and
  birth credit (slices 4–5) remain planned; their current paths still require
  simulation.

Supported surfaces (standing contracts, not transitions; only an owner-declared
sunset would make one):

- Client window: **every release from 26.1.2 through 26.3** (775/776/777),
  canonical 26.3. [ADR 0015](decisions/0015-multiversion-protocol-translation.md)
  owns the adjacent chain in `crates/mc-edge/src/translation/`; `content import`
  installs every endpoint cache and the independently owned step snapshots
  (startup never downloads; [operations](OPERATING.md#client-protocol-compatibility)).
  A version bump adds a step and never drops existing clients. Backfill to 1.7
  is a future constraint, not a compatibility claim.
- Loader ([ADR 0010](decisions/0010-krasnokamen-loader-contract.md), accepted,
  staged): the Loader repository's `main` (`d508697`, same as `origin/main`)
  defaults to `minecraftVersion=26.3` (jars `1.0.0-rc.2`) and still builds
  26.1.2 for Fabric/NeoForge with `-PminecraftVersion=26.1.2`, while
  [KRASNOKAMEN_LOADER](KRASNOKAMEN_LOADER.md) documents 26.1.2 only and
  `LOADER-SOUND-MIXIN` stays open. Whether 26.3 replaces 26.1.2 is the owner's
  decision; until then both are supported.

Other ADR states: [ADR 0018](decisions/0018-in-game-computer.md) (in-game
computer) and [ADR 0019](decisions/0019-voice-chat.md) (voice chat) are
accepted designs with draft implementations and pending real-client QA.
[ADR 0020](decisions/0020-entity-checkpoints-at-one-million.md) is cut over
(live checkpoints write only changed entity rows, `changed_checkpoint_data_for`
in `core/capture.rs`; the materialized capture serves offline conversion, Anvil
export and tests by design) but was accepted with its strict latency criterion
unmet: the 1–2-core profile, the exact-UUID offline fold after reopen and SIGKILL
after a committed checkpoint were never run (last revalidated on `73761dc5`,
2026-11-14). The plugin API ships as `krasnokamen:plugin@0.9.0`;
[ADR 0024](decisions/0024-engine-and-gameplay-packages.md) is accepted, not
implemented.

## Known gaps before 1.0

From the independent whole-core review of RC5 (2026-11-07), updated. Each needs
a fix or an owner decision recorded in the release notes.

| Gap | State |
|---|---|
| The bug ledger as a 1.0 gate | 198 rows open; 1.0 needs them fixed or dispositioned and `regressions-client` run on the release candidate |
| Plugin API `krasnokamen:plugin@0.9.0` has no stability promise | owner decision |
| Scoreboards of ~500+ scores and functions sending 512+ chat lines per tick disconnected recipients (per-tick outbox cap) | fixed after RC5 (`700ae656b`, `OUTBOX-TEXT-FLOOD-KICK`); unreleased |
| Phantoms and pillager patrols never spawned in survival | fixed after RC5 (`886c556f9`); unreleased |
| A world whose first start stopped before the initial checkpoint could never open | fixed after RC5 (`WORLD-FIRST-START-INTERRUPTED`: such a world reopens as a new world); unreleased. An RC4 world in that state (no first-start marker) is not separately proven |
| Conduit, beacon and shrieker block entities panicked the tick on corrupt data | fixed after RC5 (`CONDUIT-RUNTIME-PANIC`, `BEACON-RUNTIME-PANIC`, `SHRIEKER-HYDRATE-PANIC`); unreleased |

Unconfirmed risks, each with the check that would settle it:

- Windows: a WAL batch file briefly locked during rename (antivirus, sync
  tools) stops the world. Check: a ≥1 h Windows soak, singleplayer and hosted,
  Defender on, world in an indexed or OneDrive folder.
- A power cut may leave recent journal batches missing. Each batch file is
  fsynced before rename; on Unix the directory is fsynced every
  `journal_fsync_every_batches` appends (default 10) and before a checkpoint; on
  other platforms directory fsync is a no-op. Whether missing directory entries
  can leave a gap that blocks recovery is unverified; do not assume only a
  trailing suffix is lost. Check: power-cut tests on ext4, XFS and Windows.
