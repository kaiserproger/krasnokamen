# Project status

Where Solaris stands and what to work on next. Update this page when a release
ships, a known 1.0 gap opens or closes, or a transition opens or closes;
day-to-day progress belongs in commits and the [bug ledger](bugs/README.md),
not here. Last updated 2026-11-07.

## Release

- Published: **1.0.0-rc.5 "Pauline"** (core tag `v1.0.0-rc.5`, native client
  release with matching server and plugins). Notes: [RC5](releases/v1.0.0-rc.5.md);
  all tags: [release notes](releases/README.md).
- RC5 checks on the published assets: checksums match; a cold Linux install of
  the client entered a new world, relaunched and re-entered it. On a 4-core
  Windows VM without a GPU the first two Play attempts timed out
  (`SP-START-TIMEOUT-SLOW-MACHINE`); walking in the world and disconnecting
  were not checked there. No macOS client or package run exists (core lib tests
  only).

## Priorities

From the [owner contract](development/owner-contract.md), in order:

1. Ordinary vanilla-client gameplay and multiplayer.
2. The production plugin API and adapters.
3. Measured optimization, ECS, ownership and autoscaling.
4. Rare edges.

A lower item moves up only when it blocks ordinary play, saves or plugins.

## Work queue

The owner's open GitHub issues are the queue:

```sh
gh issue list -R kaiserproger/solaris --author kaiserproger --state open
```

Known bugs, with their status and regression checks, are rows in
[`bugs/ledger.toml`](bugs/README.md).

## Open transitions

A transition replaces one mechanism with another and is open while both are in
the tree. A row exists only where the target is decided (an accepted ADR,
AGENTS.md or an owner decision); proposals and supported surfaces are listed
below the table and carry no rule. This is the only register: a change that
opens a transition adds its row, and the change that deletes the old side
deletes the row. How a change must treat an area with an open transition:
[adding a feature](development/adding-a-feature.md#open-transitions). Counted
2026-11-07 on the integration worktree; the commands below the table reproduce
the counts.

| Transition (old → new) | State and evidence | Done when | Owner doc |
|---|---|---|---|
| Harness server launch: direct `subprocess.Popen` and asyncio `Session.spawn` → `runtime.start_server` | 76 modules in `tools/harness/backends/`; 18 start the server through `runtime.start_server` (22 call sites). 7 call sites in 4 modules start `solaris-v2` with `Popen` directly (`client_probes.py` 3, `village_autonomy_qa.py` 2, `seed_review.py` 1, `native_scale.py` 1), and `catalog_load.py` (used by `catalog_spain.py`) starts it through asyncio. `runtime.start_server` is only `subprocess.Popen` (`runtime.py:317-319`): the sites share `wait_port` and `stop_process`, not a launcher. | One launch function owns start, readiness and teardown of the server, every backend calls it, and no backend runs `Popen` or `create_subprocess_exec` on the server binary. | [AGENT_TOOLING](AGENT_TOOLING.md#receipts-and-failure-diagnosis) |
| Release-binary pinning: `regressions._release_server` and the ledger's `.analysis/ledger-builds` registry → `tools/dev/pin-release` | Three mechanisms key a release server by tree identity. `pin-release` writes immutable `<work>/pins/<sha12>[-<diff8>]/manifest.json`. `regressions._release_server` (`regressions.py:238-279`) builds its own release server per run and hard-links it as `solaris-v2-<sha256>` into the run's artifact dir. `ledger.py:415-440` (`_register_server`, `_server_built_from`) writes `.analysis/ledger-builds/<sha256>.json` with a second tree fingerprint (`tree_fingerprint`, `ledger.py:386`). `ledger probe --client` ignores `SOLARIS_SERVER_BINARY` ([bug ledger](bugs/README.md#probing-rows-before-assignment)). | `ledger probe` and `promote` take their server from `pin-release` (one pin, one manifest, one fingerprint); `_register_server`, `.analysis/ledger-builds` and the private release build are deleted; `promote` checks the pin manifest. | [tools/dev README](../tools/dev/README.md), [bug ledger](bugs/README.md#the-fixed-rule-single-home) |
| Typed block and item identity: `.path() ==` comparisons and inline `"minecraft:` literals → kinds resolved once at load (target stated in the ratchet source, owner-approved 2026-11-07) | No shared `BlockKind` or `ItemKind` exists (only module-local kinds such as `vehicles::VehicleKind`). The code-health ratchet (`crates/xtask/src/code_health/ratchet.rs`, diagnostic, never fails) counts `mc-core` production code: 2646 `"minecraft:` literals (baseline 2441 at `c09e1391`) and 93 `.path() ==` (baseline 88). Both grew. | A typed kind exists, new code uses it, both counts are at or below their baselines, and no new literal enters. No ADR owns this yet: write it before the first kind lands. | none; the ratchet source and [ADR 0016](decisions/0016-mob-capabilities.md) for mob types |
| Mob behaviour chosen by type id or name → capability flags ([ADR 0016](decisions/0016-mob-capabilities.md)) | Layers 1 and 2 (plugin registration) and sensors step 1 are implemented. `mc-core` production code still has 6 `== *X_TYPE` comparisons and 9 `matches!(<name>, "minecraft:…")` tests (grep count, includes any legitimate uses). | No behaviour shared by more than one mob type is selected by comparing its id or name; each is a flag in `vanilla_capabilities`. | [ADR 0016](decisions/0016-mob-capabilities.md) |

```sh
grep -c 'runtime.start_server(' tools/harness/backends/*.py          # per file; Popen: grep -n 'subprocess.Popen' there
cargo run -p xtask -- code-health                                     # ratchet trend (diagnostic)
grep -rnE '== \*[A-Z_]+_TYPE|\*[A-Z_]+_TYPE ==' crates/mc-core/src --include=*.rs | grep -v _tests.rs | wc -l
grep -rnE 'matches!\([^)]*name[^)]*, *"minecraft:' crates/mc-core/src --include=*.rs | grep -v _tests.rs | wc -l
```

Undecided proposals (no rule attached; each becomes a row only when the owner
decides it):

- Tick system graph ([ADR 0017](decisions/0017-tick-system-graph.md)): a
  prototype with one system, `EntityStep`
  (`crates/mc-core/src/tick/system_graph.rs`, called once from
  `tick/entity_phase.rs:1092`). The ADR is not approval to migrate the rest
  of the tick; each of steps 2–5 needs the owner's approval and a measured
  benefit. The existing tick is the current path, not an old side.
- Earth WorldPlan ([ADR 0012](decisions/0012-earth-worldplan-and-native-overlay.md)):
  "Proposed … not ported". `crates/mc-worldgen/src/earth.rs` and `earth/`
  (15.8k lines including tests) still compile into `mc-worldgen`; only
  `mosaic.rs` and `bin/worldgen_mosaic.rs` use them, and `mc-core`, `mc-edge`
  and `mc-world` do not reference `earth`. Port or delete is the owner's call.
- [ADR 0021](decisions/0021-build-time-and-crate-split.md) step 2 (settlements
  crate pilot) is designed, not started, and recommends against splitting
  while 189 of 421 top-level `mc-core` modules form one dependency cycle (the
  ADR's 2026-11-05 count, not recounted here). Step 3 is the plan to narrow it.
- [ADR 0016](decisions/0016-mob-capabilities.md) Layer 3 (Wasm capability
  code), operator data-directory blueprints and persistent sensor
  subscriptions.
- Content without Java ([ADR 0022](decisions/0022-content-without-java.md)):
  Mojang bytes read from the jars the user downloads from Mojang, fact tables
  from the one pinned facts bundle, RegistryData from the native encoder
  `mc_data::registry_network` (implemented and proven byte for byte against
  vanilla captures, not wired into `content import`), packet names from
  Solaris tables; the singleplayer content bundle would retire. Waits on the
  owner's four questions in the ADR; the Java import and the content bundle
  are the current path, not an old side.

Supported surfaces (standing contracts, not transitions; only an
owner-declared sunset would make one):

- 26.1.2 (protocol 775): AGENTS.md and
  [ADR 0015](decisions/0015-multiversion-protocol-translation.md) keep it
  served through a retained legacy content cache
  (`crates/mc-edge/src/legacy/`, 14 files, about 4.7k lines including tests;
  `mc-data` `legacy_26_1_2.rs` and `enchanting_26_1_2.rs`;
  [operator steps](OPERATING.md#retaining-2612-clients-after-upgrading-to-263)),
  and owner testing runs on a 26.1.2 client. Fixes there are always allowed.
- Loader on 26.1.2 ([ADR 0010](decisions/0010-solaris-loader-contract.md),
  "Accepted, staged implementation"; [SOLARIS_LOADER](SOLARIS_LOADER.md)): a
  supported surface with an open Fabric 26.1.2 blocker (`LOADER-SOUND-MIXIN`).
  The record disagrees on the target: the Loader repository's `origin/main`
  has `minecraftVersion=26.3` (`a75f107`, jars `1.0.0-rc.2`), while the owner
  host's local `solaris-loader` `main` (`9457da3`, 4 commits behind), SOLARIS_LOADER
  and the ADR's "native 26.1.2 networking APIs" say 26.1.2. Whether 26.3
  replaces 26.1.2 is the owner's decision; until then both are supported.

Other ADR states, for completeness: [ADR 0018](decisions/0018-in-game-computer.md)
(in-game computer) and [ADR 0019](decisions/0019-voice-chat.md) (voice chat)
are accepted designs with draft implementations and pending real-client QA.
[ADR 0020](decisions/0020-entity-checkpoints-at-one-million.md) is cut over
(live checkpoints write only changed entity rows, `changed_checkpoint_data_for`
in `core/capture.rs`; the materialized capture that remains serves offline
conversion, Anvil export and tests by design) but was accepted with the strict
latency criterion unmet: the 1–2-core profile, the exact-UUID offline fold
after reopen and SIGKILL after a committed checkpoint were never run.

## Known gaps before 1.0

From the independent whole-core review of RC5 (2026-11-07). Each needs either a
fix or an owner decision recorded in the release notes.

| Gap | State |
|---|---|
| A scoreboard with about 500 or more scores disconnected every joining player (per-tick outbox cap 512) | Fixed after RC5 (`fix: batch scoreboard updates for reliable delivery`); not released yet |
| The bug ledger is not a reliable 1.0 gate: rows fixed in code are still `open`, some fixes have no registered regression check, and `regressions-client` has not run for the RC5 cycle | Open: reconcile the ledger and run `regressions-client` before 1.0 |
| Phantoms and pillager patrols never spawned in survival in RC5 | Fixed after RC5 (`feat(core): phantoms, patrols, ...`); not released yet |
| The plugin API ships as `solaris:plugin@0.8.0` with no stability promise | Owner decision |
| A world whose RC4 first start was stopped (database without checkpoint, no first-start marker) refuses to open forever; the world never had a player | Documented in the RC5 notes; no tool offers a fix |

Unconfirmed risks from the same review, each with the check that would settle it:

- Windows: a briefly locked WAL batch file (antivirus, sync tools) during the
  rename stops the world. Check: a 1 h or longer Windows soak, singleplayer and
  hosted, with Defender on and the world in an indexed or OneDrive folder.
- A function that sends more than about 500 chat lines in one tick may
  disconnect every recipient (same outbox cap). Check: 600 `tellraw @a` lines
  with two players online.
- Conduit and shrieker still panic on a bad block entity if some write path
  skips chunk validation (piston, structure, plugin).
- A power cut may leave recent journal batches missing. Each batch file is
  fsynced before rename; on Unix the directory is fsynced every
  `journal_fsync_every_batches` appends (default 10) and before a checkpoint.
  On non-Unix, directory fsync is a no-op. Whether missing directory entries
  can leave a gap that prevents recovery is unverified; do not assume that only
  a trailing suffix is lost. Check: power-cut tests on ext4, XFS and Windows.
