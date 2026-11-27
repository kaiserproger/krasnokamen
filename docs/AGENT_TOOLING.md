# Agent tooling

[CONTRIBUTING](../CONTRIBUTING.md#validation) holds the L0–L2 levels; this page is the
wiring around the validation harness. `python3 -m tools.harness list` (profiles)
and `python3 -m tools.harness explain` (scenarios) are authoritative; prose here
never overrides them. Never invoke `tools/harness/backends/` directly: those
engines are private, and only a profile run leaves a canonical receipt.

Related: [real-client gates](development/real-client-gates.md) (native client,
graphical gates, render oracle), [heavy commands](development/heavy-commands-and-memory.md)
(the owner-host runner), [catalog load](development/catalog-load.md),
[performance windows](development/performance-windows-and-profiling.md).

## Prerequisites

Run from the repository root with Python 3, the pinned Rust toolchain and
cargo-nextest (CI pins 0.9.143); builds are in
[contributor setup](../CONTRIBUTING.md#prerequisites). Linux systemd user scopes
provide the [resource bounds](#resource-bounds); without one the harness warns
and runs unbounded.

- Java profiles: Java **25**, an executable sibling Loader `gradlew`, the client
  jar in `KRASNOKAMEN_CLIENT_JAR` (CI takes it from Loader `gradle.properties`
  with the Mojang SHA-1), Xvfb for graphical profiles.
  `KRASNOKAMEN_LOADER_ROOT` overrides the sibling Loader; `standard-pack` reads
  `KRASNOKAMEN_DEFAULT_PLUGINS_ROOT`, never an in-core copy.
- Native profiles: a built client and prepared data root, per
  [client prerequisites](development/real-client-gates.md#prerequisites).
- Real-client and scale profiles: a release server in
  `KRASNOKAMEN_SERVER_BINARY` (owner host: a `pin-release` pin, never pooled
  targets; elsewhere `cargo build --release --locked -p mc-edge --bin krasnokamen`)
  and the [environment block](development/real-client-gates.md#3-environment-block).
- Free loopback ports: MCP default 39095, scenario servers commonly 25565/25567,
  per-run bridge ports. Bind failures fail closed; readiness waits on
  inotify/pidfd/log/MCP events, never sleeps.
- Mojang bytes, `data/vanilla*` and `.analysis/` never enter Git.

## Harness entrypoint

```sh
python3 -m tools.harness list
python3 -m tools.harness explain [scenario-id]
python3 -m tools.harness run PROFILE [--release] [--timeout-seconds N] [--client java|native] [--platform fabric|neoforge|forge] [--plugins-dir PATH] [--windowed] [--server-bin PATH] [backend args]
python3 -m tools.harness run scenario <id>
python3 -m tools.harness client [--check] [--platform fabric|neoforge|forge]
python3 -m tools.harness mcp [--server-address host:port] [--exercise-input] [--disconnect] [--scenario-id ID]
python3 -m tools.harness plan [--base REV] [--paths PATH...] [--json]
python3 -m tools.harness ledger {probe,promote,graphical-control,show,list,edit,split}
```

`tools/harness/__main__.py` owns the typed options; a misuse exits 2.

| Option | Boundary |
|---|---|
| `--server-bin` | immutable executable for `loader-live`, `protocol-matrix`, `java-matrix`, `native-matrix`, `matrix-random`; every other profile reads `KRASNOKAMEN_SERVER_BINARY` |
| `--plugins-dir` | existing directory for native, `java-v2-p5-towns`, `scene-shot`, `loader-live`, `graves` and the matrix profiles (a matrix stages its own set) |
| `--windowed` | native only, except `scene-shot` and `settlement-map-qa`, which also accept Java |
| `--client native` | only CLI allow-listed profiles (`list` shows support); `screens-voice-qa --client native --windowed` dispatches the native and Fabric legs |
| `--release` | changes `build` only |
| `--timeout-seconds` | per-stage command deadline (default none); overrides the backend scenario default where honoured (core-client/inventory/loader-live 600 s, playable 1500, replay 180) |

**Statuses:** `passed`, `failed`, `timeout`, `prepared`, `interrupted`. Only
completed execution passes; preparation and `--check` modes return `prepared`
even with exit 0. Unknown, unlaunchable or nonzero commands, absent scenario
evidence and every timeout fail closed. A timeout or SIGINT/SIGTERM/SIGHUP tears
down every stage descendant, including separate process groups, and records it.
Preflight rejects pooled `targets/slot-*` pins and invalid scene steps before
launch; native profiles also need `KRASNOKAMEN_CLIENT_BINARY` and data paths.

**Java launch.** `loader-live` has no client-ready timeout: a cold NeoForge
build compiles about 7300 sources and Gradle's launch task stays alive with the
game, so neither a build timeout nor process completion proves a launch. It
waits for the owned MCP listener and fails on process exit; cancellation tears
down the client group. Other Java callers keep 180 s. Shared Java launch
isolates `HOME` and JVM `user.home` to `<game>/.home` (provision Forge assets
there; the Gradle cache is separate), retries only explicit render-thread
observation timeouts inside the bootstrap deadline, still requires the real
title screen, and gives the 512 MiB game heap only to the game JVM, never to
compiler JVMs through `JDK_JAVA_OPTIONS`. Every harness-launched Java client gets
null audio (`ALSOFT_DRIVERS=null`, a private null ALSA config,
`PIPEWIRE_REMOTE=/nonexistent`, `PULSE_SERVER=unix:/nonexistent`, a private
`XDG_RUNTIME_DIR`); launch is refused (`MissingCapability java_audio:*`) if the
child environment would carry anything else, recorded in
`<game dir>/java-audio-isolation.json`. Named private audio captures monitor
only their own sink and retain `<sink>.f32le`.

## Profiles and scenarios

The catalogue is not copied here:

- `list`: every profile, its family and scope.
- `explain [id]`: exact profile/selector, required tests, prerequisites,
  assertion sources, proves/limits; read-only, launches and downloads nothing.
- `run scenario <id>`: the shared execution and receipt path with the catalogue
  selector; no free-form trailing arguments and no graphical→TCP fallback.

Unknown IDs, missing required tests or counts, empty selection or execution and
selected failures fail closed. `details.named_scenario` binds ID, evidence,
profile, selector, manifest/seed/fixture, prerequisites, sources, proves/limits,
repository revisions, command cwds, the selected/executed/passed/failed/ignored/
filtered counts and missing refs. Discovery is not execution: Cargo refs are
exact non-ignored function names ([registration](bugs/README.md)), not modules.
Ignored local-input cases need declared input and explicit execution; a normal
suite cannot claim them.

### Evidence boundaries

What a green receipt does **not** mean; never claim more.

- `correctness` is the L2 close gate (fmt, strict workspace Clippy,
  `code-health`, the full nextest suite, then registered `regressions`).
  `code-health` is a structural tripwire, not gameplay evidence.
- `core-client` is the server/own-chat continuity subset (Java default, native
  subset); its receipt excludes Wasm, economy and Loader-required rejection.
- `entities` summons its combat fixture with a fresh UUID and waits for that
  exact entity; natural pigs do not identify the fixture.
- `logical-native` (headless movement/look/rejected-use subset) and
  `native-scale` (non-exclusive CPU/RSS canaries) are functional-native, never
  `playable`, `regression` or `replay`; `logical-native --windowed` is rejected.
- `regression` rejects protocol bots, wire probes, `mc-test-harness` clients and
  mocks as participants. `playable` is the complete twenty-minute no-debug
  survival loop (`docs/playable/real-client-playable-loop.json`, fresh per-run
  world): never shorten it or substitute a debug scenario. `replay` is the core
  seed-81 action replay (`docs/real-client-regression/manifests/core-replay-seed-81.json`),
  not a regression shortcut. These three append `--run` unless a mode token is
  present: `--check` validates the Gradle runClient adapter, `--prepare` creates
  a run directory and observation templates, `--validate-run <dir>` checks an
  existing run's shape. For `--check` and `--run`,
  `KRASNOKAMEN_REAL_CLIENT_AGENT_SCENARIO` must name exactly one scenario in
  `KRASNOKAMEN_REAL_CLIENT_MANIFEST`; an undeclared debug scenario is not
  evidence for a no-debug manifest.
- `oracle` without `--run` only inspects readiness (`prepared`, or
  blocked/degraded when artifacts are absent); `oracle --run` compares against
  the local artifacts each manifest names. `quirk-parity` compares a real vanilla
  server with Krasnokamen ([QUIRK_PARITY.md](QUIRK_PARITY.md)).
- `catalog-load` and `constantinople`: [catalog load](development/catalog-load.md),
  [Constantinople](development/constantinople-fixture.md). Smoke runs and
  `ASSISTED_CHAIN_ONLY` are never capacity results.
- `seed-review` and `seed-contact-sheet` capture terrain evidence and never grant
  the owner's `ACCEPT`.
- Native `--windowed` receipts prove what their retained frames and predicates
  show; headless native receipts (`details.headless=true`) prove received state
  only ([evidence classes](development/real-client-gates.md#evidence-classes)).
- Debug numbers are not optimized performance evidence; retired
  `entity-scale`/`living-world-scale` receipts keep their original scope as
  history (use `catalog-load` for public-protocol workloads).
- Artifact builders (extract/dump scripts, fixture generators,
  `prepare-real-client-scenario.sh`) are not validation entrypoints.

### Crash recovery (`battle-crash`)

`battle-crash` drives the battle runner with a live kit, a separately
acknowledged `/give`, SIGKILL and recovery at the four WAL/checkpoint
boundaries. It needs `KRASNOKAMEN_SERVER_BINARY`, `KRASNOKAMEN_CLIENT_BINARY`,
`KRASNOKAMEN_BATTLE_CLIENT_ROOT` and `KRASNOKAMEN_BATTLE_WORLD`, uses an
explicitly empty plugin directory and retains each cut's crash copy, published
WAL and backup. Its initial recovery census is the first console input, before
any client joins.

- `--start-frozen` (an ordinary production option) freezes autonomous
  simulation before the first tick, not all mutation: joins, commands, plugins,
  chunk generation, generated-entity admission, WAL and checkpoints continue
  until `tick unfreeze`. Reconnected players are compared while frozen.
- `world crash-census --copy` may repair redb metadata in the world it opens:
  give it only the harness-created disposable copy, never a retained source or
  live world. Its holder census comes from the ordered checkpoint+WAL replay
  before live hydration; what that does and does not prove is in
  [crash census evidence](QA_AGENT_PROTOCOL.md#crash-census-evidence).
- Durable and console holder filters include case-sensitive projectile `item`
  and `FireworksItem` stacks besides the `Item` holder; the projection mirrors
  the `mc-core` capture encoders, so drift in compared fields fails as
  `changed`, but a key both filters exclude stays invisible.

## Validation plan (`plan`)

`python3 -m tools.harness plan [--base REV] [--paths PATH...] [--json]` is
read-only; run it inside the judged tree before you start and again before you
report. Input defaults to changed and untracked paths against HEAD (or
`--base`); `--paths` replaces it and accepts sibling paths. It unions the gates
with exact command, cited policy `why`, strictest run count (variable profiles
≥3), negative-control and reviewer requirements. Unknown paths and paths in
several risk domains need manual classification.

`tools/harness/plan.py` `RULES` restates policy and never creates it: change a
rule only together with the contributing, persistence, protocol, real-client, ledger
or sibling document it cites. `AREAS`, `BACKEND_PROFILES` and the scenario
catalogue supply mapped gates (otherwise `no mapped gate`); `VARIABLE_PROFILES`
follows [gate hygiene](QA_AGENT_PROTOCOL.md#gate-hygiene). A new dependency,
toolchain or prerequisite needs a decision record
([adding a feature](development/adding-a-feature.md#new-dependencies-and-components)),
not an extra reviewer.

## Test gate (nextest)

`run test`, also the `test` stage of `correctness`, runs:

```sh
cargo nextest run --profile default --ignore-default-filter --no-fail-fast --no-tests=fail --workspace --all-targets
```

No retries and no default-filter exclusions locally: the whole suite runs even
after failures; a missing binary, failed test or empty selection fails closed.
Trailing selectors use the same runner
(`python3 -m tools.harness run test -p mc-core --lib inventory`). Criterion
`harness = false` bench targets run in test mode; `cargo bench` remains the
measurement path. Per-test durations land in `test.log`.

- **Regressions reuse the run.** L2 snapshots the fresh nextest JUnit report as
  `test-junit.xml` (also on failure); registered Cargo regressions consume it
  instead of rebuilding: every exact package/function ref must have executed and
  passed, and missing, ignored/skipped or failed refs fail with their ledger row
  IDs. Stale reports are discarded first; L2 never opts into ignored tests.
  Standalone `run regressions` discovers and executes its Cargo refs itself;
  Python, client and harness refs keep their own execution.
- **Concurrency.** `RUST_TEST_THREADS`, else `NEXTEST_TEST_THREADS` (default the
  CPU count), sets the default; `--test-threads=N` before any `--` overrides it.
  Unsupported Cargo/libtest flags and non-executing flags such as `--no-run` are
  rejected, never rerouted. The `component-host` nextest group keeps the measured
  four-concurrent-test Wasm host budget: do not add a process-wide guest mutex
  or change production Wasmtime limits for a heavy run; use an explicit
  environment override for a proven-heavy scope.
- **CI short run.** `KRASNOKAMEN_NEXTEST_PROFILE=ci` selects the `ci` profile in
  `.config/nextest.toml`, whose `default-filter` is a **fixed list** of tests
  that took over 3 s in one L2 run (2026-10-18); it is never recomputed. The
  local gate keeps the full suite.
- **Memory receipts.** Successful `test` receipts record `memory_peak_bytes`,
  `memory_limit_bytes`, `memory_cgroup`, `memory_events`,
  `memory_swap_peak_bytes`, the thread environment, `test_runner` and
  `nextest_profile`; unavailable counters say so, never zero. Scope peaks include
  Cargo, compilation and page cache: for a concurrency decision build with
  `cargo test -p PACKAGE --test BINARY --no-run`, run only that executable in its
  own scope and read anonymous versus file pages in `memory.stat`.

Test-writing helpers with a fixed contract:

- `crate::tick_tests::warp_to_block_tick` jumps an isolated scheduled-block
  simulation to the block's real queued deadline; nothing else advances. Do not
  use it when movement, random ticks, other deadlines or per-tick effects matter;
  keep an ordinary-tick test, or compare a small continuous case with the warped
  one, including a control that skips a consequential tick. `/tick sprint`
  removes waits but still executes every tick.
- Live pause tests await `pause_when_empty_tests::wait_for_pause` (the tick
  owner's pause acknowledgement); a quiet broadcast channel is not proof of
  pause. Continued-simulation tests await a concrete tick count.
- Raider casting fixtures advance real ticks until the target owner records
  acquisition (bounded by `raiders::SEARCH_INTERVAL`) and journal every
  intervening tick, not only the casting tick.
- `inventory::conservation_tests` (proptest over one- or two-player client
  operation sequences through `run_tick`; item totals never change) runs 64
  cases; `PROPTEST_CASES=3000 cargo test -p mc-core --lib inventory::conservation_tests -- --nocapture`
  is the long form with the per-operation coverage table. Shrunk failures are not
  persisted: turn one into a deterministic regression.

## Bot scenario matrix

`protocol-matrix`, `java-matrix` and `native-matrix` share the
`matrix_api.Adapter(setup, act, oracle)` contract and the `matrix_execution`
context boundaries; the profiles own processes and receipts (`ProtocolServer`
starts through `runtime.start_server`). Missing verbs and observations raise
`MissingCapability` and never count as coverage. Adapter guidance:
[matrix domains](../tools/harness/matrix_domains/README.md),
[containers](../tools/harness/matrix_domains/containers.md),
[platform coverage](../tools/harness/matrix_domains/platform.md).

The frozen combinatorial specs and generator live in
`tools/harness/matrix_specs/` (`gen_matrix.py`, `subjects*.py`, `domains/`,
`matrix.yaml`, `MATRIX_FREEZE.json`); the frozen matrix describes historical
26.1.2 cells and its recorded source heads are provenance, not coverage. A
changed frozen id set needs explicit `--refreeze`. Generation reads this checkout
and the siblings under `KRASNOKAMEN_WORK_ROOT`. Mutable ledger JSONL, copied
plugins and run directories stay outside Git, by default in
`<work>/scenarios/` (`KRASNOKAMEN_MATRIX_DATA` changes the root,
`SCENARIO_LEDGER` the ledger). Run only against isolated worlds, never the live
test server. Per-row oracle, port ranges and
[plugin lanes](../tools/harness/matrix_domains/README.md#readiness-json-state-and-plugin-lanes)
are in the adapter contract.

```sh
python3 -m tools.harness run protocol-matrix \
  --server-bin "$KRASNOKAMEN_SERVER_BINARY" \
  --vanilla-data /absolute/path/to/data/vanilla \
  --subject block_place.full_block
```

The 26.3 cache's version metadata selects the wire protocol. Select a frozen
protocol row with `--matrix tools/harness/matrix_specs/matrix.yaml --matrix-id ID`
instead of `--subject`. Java takes `--matrix`, `--id`, `--ledger`, `--data-dir`;
native takes `--client native --windowed --matrix`, `--only-id`, `--ledger`,
`--run-root`. Both graphical lanes need `--plugins-dir`. The native lane runs
inside the private hardware-free `tools/harness/screen_audio_graph.sh` wrapper,
which points `XDG_DATA_HOME` at its own graph where the client finds no content:
run `sh -c 'XDG_DATA_HOME=<client data XDG> exec python3 -m tools.harness run native-matrix ...'`
inside it, with that XDG's `krasnokamen-client/` symlinking the client content.

### Matrix runner status

True-vanilla Java needs the Loader's premain event transformer and a direct,
non-symlink protocol-775 `KRASNOKAMEN_LEGACY_CONTENT_CACHE`; Java matrix Play
entry exists but is **not gameplay acceptance**. Experimental `matrix-random`
includes conflicting same-chest-slot quick moves with ≥2 bots; its opt-in
consumed-input/tick log must be complete, and a fixed action seed does not make
network scheduling deterministic. Fixture setup removes dropped stock and waits
for observer-confirmed emptiness; conservation covers every item type in x/z
24–44, y 79–90, and each player's received inventory supplies private evidence.
Restart tests carried-stack custody first, then crashes after the transfer/WAL
fence, comparing per-player and chest slots, item fields/component SNBT and
observed block states. Neither replay determinism nor crash atomicity follows
without real-server receipts.

`tools/dev/scenario-nightly --install --server-bin /absolute/pin/krasnokamen --vanilla-data /absolute/vanilla-data`
installs the owner-host 03:00 user timer (`systemctl --user disable --now
solaris-scenario.timer` removes it). It needs a verified pin and imported data,
runs at low priority through `mem-run` within a campaign time budget, and writes
to `<work>/receipts/scenarios` by default. Campaign `result.json` keeps manifest,
requested/attempted seeds and reductions; each seed keeps command, step/bot
counts, server commit/hash, harness fingerprint and result. Retention may prune
pass artifacts and older failure artifacts but keeps the compact seed records.
The budget is checked between seeds (not a quota), and each seed or reduction
needs ≥100 GiB free or the campaign stops with `disk_floor`; host-wide disk rules
are in [heavy commands](development/heavy-commands-and-memory.md#disk).

Optional CRAP measurement (`cargo-llvm-cov` + `cargo-crap`): keep the test
command, revision, raw LCOV, analyzer output and exact scope; a partial suite is
not workspace coverage, line coverage is not branch coverage, and missing
function coverage is unknown. The owner's `CRAP < 10` rule for new or changed
functions applies only after 1.0 ships.

## Code-health ratchets

`cargo run -p xtask -- code-health` (harness `run code-health`) measures
`mc-core` production code with `syn`, never text: comments, attributes,
`#[cfg(test)]`/`#[test]` items and `*_tests.rs` files are outside every number.
`--root <dir>` measures another checkout read-only. These families fail when
they rise above their ceiling in `crates/xtask/src/code_health/ratchet.rs`:

- top-level modules in the largest dependency cycle (Tarjan over
  `crate::`/`super::` paths and root re-exports): no module may join it
  ([ADR 0021](decisions/0021-build-time-and-crate-split.md#cycle-ratchet-implemented-2026-11-07),
  [new features are modules](development/adding-a-feature.md#new-features-are-modules));
- string literals starting with `minecraft:`;
- `==`/`!=` with a `.path()` call on either side;
- `panic!`;
- `.push(..)` on `merge_block_writes`;
- production items plus non-item statements per hot file (`tick/entity_phase.rs`,
  `settlements/construction.rs`, `tick.rs`, `entities.rs` under
  `crates/mc-core/src`): item statements count once, inline modules belong to
  their file, external module declarations to the parent;
- `.project` and `.pending_order` field expressions outside
  `settlements/construction*.rs` and its children: a syntactic proxy for
  `SettlementRecord`, so unrelated same-named fields count too.

Independently, `code_health/domain_boundary.rs` caps named production
references from outside settlements and villager AI into the approved D65 move
set (villager targets at submodule granularity; imports, qualified aliases,
module-relative paths and re-exports count; method resolution and opaque macro
paths are not covered). Passing one metric never compensates for failing the
other. Code-health also runs owner-boundary tripwires (lower-crate ownership,
block-placement semantics, boundary-leak scans, harness catalogue ownership, API
manifests); each failure names the rule that moved.

Macro bodies that parse as comma-separated expressions (`format!`, `matches!`,
`assert!`) are visited like code; other bodies are read token by token only for
vanilla ids, panics and merge-block-write pushes. Hot-file counts and
construction field expressions do not expand opaque macros.

Current values and the reason for every change are the comments on the
constants; the report prints them. A value below its ceiling prints
`ceiling can drop to N`: lower the constant in the same change. Raising one is an
explicit edit with a reason comment; there is no flag or override. Cycle
production lines are a printed trend, never a failure. The report also lists
per-module breakdowns, the most connected cycle modules with the cheapest cuts,
and known blind spots: dependencies through a type's methods without a module
path, ids without the `minecraft:` prefix, `.path()` right operands or
construction fields inside opaque macros, destructured construction fields.
Known false positives: vanilla ids in data/registry tables and protocol/NBT/Anvil
schemas, `path()` on non-identity types, panics on truly impossible states, the
push inside `TickState::write_block`.

## Resource bounds

`run` and `client` scope the whole invocation and every child before work;
limits are totals, not per process:

| Property | Default | Override |
|---|---|---|
| CPUQuota, CPUWeight | physical cores ×100 %, weight 50 | `KRASNOKAMEN_HARNESS_CPU_QUOTA` (`off` disables) |
| MemoryHigh | off | `KRASNOKAMEN_HARNESS_MEMORY_HIGH` |
| MemoryMax | 12G for `test`/`correctness` and `MEMORY_HEAVY_PROFILES` (`java-battle-functional`, `java`, `loader-live`, `scene-shot`, `battle-crash-recovery`; server+Java+Xvfb exceeded 4G); 4G otherwise; a named scenario inherits its profile | `KRASNOKAMEN_HARNESS_MEMORY_MAX` |
| MemorySwapMax | 1G | `KRASNOKAMEN_HARNESS_MEMORY_SWAP_MAX` |

Admission refuses when MemAvailable is below MemoryMax + 1 GiB; a refusal is not
acceptance. On the owner host, Main's
[`mem-run.sh`](development/heavy-commands-and-memory.md) wraps the run and sets
`KRASNOKAMEN_HARNESS_CPU_SCOPE=1` so no second scope escapes it.

## Receipts and failure diagnosis

Each `run` writes an isolated `.analysis/validation/<UTC stamp>-<profile>-<id>/`
with `result.json`, per-command logs and scenario evidence. `result.json`
records `profile`, `client`, `status`, `scope`, `commands`, `exits` (`null` for
an unlaunchable command, never an invented code), repo-relative `logs`,
`started_at`/`ended_at`, `duration_seconds`, `details` and `error`. An unknown
profile still writes a `failed` (`unknown-profile`) receipt listing the known
ones. Graphical profiles share one process/readiness implementation driven by
process, log and client-state events and clean up their server, client and Xvfb.

`details.failure` names the observed lifecycle `phase` (`preflight`,
`server_start`, `client_start`, `join`, `scenario`, `assertion`, `teardown`,
`unknown`), the child `exit_code` or `signal`, log paths and the server/client
binary paths and SHA-256 when available; null means not observed, and a cleanup
exit is not the failure before it. Never infer OOM or owner cancellation from a
signal alone: match child, scope and cgroup evidence under
[OOM diagnosis](development/heavy-commands-and-memory.md#oom-and-killed-commands).

Preflight takes prerequisites from the profile declaration in
`tools/harness/profiles.py`, not its scope label. Native profiles read the
client's MCP schemas with `krasnokamen-client mcp-schema` (`tools` and
`headless_tools`, the same catalogue as MCP `tools/list`); a release that
answers `unknown_command: mcp-schema` still runs with `details.mcp_schema` "not
validated", and any other failure fails preflight. Java MCP profiles compile the
selected Loader's `ClientMcpTools` definitions and check scene-shot input calls
and literal calls in the backend and scenario sources against them; computed
calls are not statically proven.

- Start from `result.json`: take the nonzero `exits` entry, then its `commands`
  argv and `logs` file. `details.error` names the failing backend;
  `details.mode` separates preparation from execution. A `prepared` receipt
  where you expected execution means a wrong or defaulted mode flag.
- `loader-live`, `core-client` and `inventory` also fail when the scenario report
  is not `passed`.
- CI `test` and `loader` jobs upload failed receipts as `test-failure-receipts`
  and `loader-failure-receipts` (two days); read the command error there, not
  the console's `see .../test.log` line.
- `KeyboardInterrupt` gives `interrupted`, exit 130, with every command, exit
  and log kept. If a port stays held, stop the stray Gradle/Minecraft/Xvfb
  process by PID. Never delete `.analysis/validation/` evidence.
- A second real client failing almost always means a missing
  `KRASNOKAMEN_REAL_CLIENT_SECOND_AGENT_SECRET` or bridge URL/port; the error
  names the variable.
- `civilization-seed-qa` awaits each console sprint's exact `tick sprint
  finished` event within the scenario deadline (not the 120 s MCP request cap);
  slow simulation shows in sprint TPS and `MC_CORE_PHASE_LOG`. Its fixture uses
  simulation distance 6 with view distance 4 so the mine stays active. House
  milestones use the received entrance door, open it ordinarily, await both open
  halves, then require the exact bed raycast and rendered capture; a found path,
  bed row or wall screenshot does not replace that, and saved-world route proofs
  are not fresh-seed acceptance.
- Graphical failures: [real-client gates](development/real-client-gates.md),
  [live-gate triage](development/live-gate-triage.md).

## Java and Loader client launch (`client`)

The default development client lives in the sibling
`../solaris-loader/fabric-agent/`; despite the name it is a **NeoForge** mod
(`dev.krasnokamen.agent.neoforge.KrasnokamenClientAgentMod`, launch task
`:fabric-agent:runClientMcp`). The Loader adapters (`loader-fabric`,
`loader-neoforge`, `loader-forge`) register the same Configuration-state
manifest/ack payloads through their native networking APIs and launch as
`:loader-<platform>:runClientMcp`. The Minecraft version is the Loader's
`minecraftVersion` in `gradle.properties`.

`client` (launch and `--check`) reads, per run:

- `KRASNOKAMEN_CLIENT_MCP_TOKEN`: a random local bearer secret you generate, not
  an account credential; without it `client` exits 2 before Gradle starts.
- `KRASNOKAMEN_CLIENT_MCP_PORT`: free IPv4 loopback port, default `39095`.
- `KRASNOKAMEN_CLIENT_MCP_GAME_DIR`: isolated game dir; default
  `../solaris-loader/fabric-agent/run-mcp`, or
  `.analysis/minecraft-loader-mcp/<platform>` with `--platform`.
- `KRASNOKAMEN_CLIENT_MCP_USERNAME`: 1–16 ASCII letters/digits/underscores;
  default `KrasnoMcp` (platformless) or `KrasnoLoader`.

```sh
KRASNOKAMEN_CLIENT_MCP_TOKEN=local-check-token python3 -m tools.harness client --check
export KRASNOKAMEN_CLIENT_MCP_TOKEN="$(openssl rand -hex 32)"
export KRASNOKAMEN_CLIENT_MCP_GAME_DIR=.analysis/minecraft-mcp-primary
python3 -m tools.harness client
python3 -m tools.harness client --platform neoforge   # another port, dir and user
python3 -m tools.harness mcp --server-address 127.0.0.1:25565 --exercise-input --disconnect
```

Launch refuses an MCP port already in use. HTTP 401 means a different bearer
token answered: check the port and process, do not retry. `--check` validates
Java 25, configuration, MCP transport tests and the Gradle adapter without
launching Minecraft. Multiplayer gates use a second port, token, game directory
and username. The endpoint is `http://127.0.0.1:<port>/mcp`. The `mcp` smoke
checks the tool catalogue, calls `minecraft_observe`, and optionally connects,
waits for Play, reads and scans blocks, lists entities, reads the recipe book,
exercises one input, runs an in-client scenario (`--scenario-id`) and
disconnects; its waits block on packet/lifecycle notifications, and interaction
checks fence reach, raycast and authoritative world state first.

## Per-run environment overrides

The harness reads no config file for these; export them before `run`:

| Variable | Effect (default) |
|---|---|
| `RUST_TEST_THREADS`, `NEXTEST_TEST_THREADS` | `run test` default concurrency (CPU count; `RUST_TEST_THREADS` wins) |
| `KRASNOKAMEN_NEXTEST_PROFILE` | nextest profile (`default`; CI sets `ci`) |
| `KRASNOKAMEN_REAL_CLIENT_MANIFEST` | regression pack manifest (`docs/real-client-regression/manifests/m94-regression-pack.json`; `playable` and `replay` preset theirs) |
| `KRASNOKAMEN_REAL_CLIENT_RUN_ROOT` | artifact root (preset under the run's `regression/`) |
| `KRASNOKAMEN_REAL_CLIENT_SERVER_CONFIG` | server config (`example.toml`; `playable` presets `playable.toml`) |
| `KRASNOKAMEN_REAL_CLIENT_FRESH_WORLD` | `1` copies the config into the run dir with a fresh world (`playable`/`replay` preset `1`) |
| `KRASNOKAMEN_REAL_CLIENT_SERVER_SEED` | optional signed 64-bit decimal overriding `data.seed` |
| `KRASNOKAMEN_REAL_CLIENT_SERVER_ADDR` | server address for the in-client driver (`127.0.0.1:25565`) |
| `KRASNOKAMEN_REAL_CLIENT_TIMEOUT_SECONDS` | Gradle runClient adapter timeout (180; `playable` 1500; `--timeout-seconds` overrides for `playable`/`replay`) |
| `KRASNOKAMEN_REAL_CLIENT_SERVER_READY_TIMEOUT_SECONDS` | server-readiness wait (120) |
| `KRASNOKAMEN_REAL_CLIENT_AGENT_BRIDGE_URL` / `_AGENT_SECRET` / `_AGENT_PORT` / `_AGENT_DRIVER` / `_AGENT_SCENARIO` | loopback JSON bridge, its per-run secret (required in agent-driver mode), port, internal driver path, scenario id; a second client also needs `KRASNOKAMEN_REAL_CLIENT_SECOND_AGENT_SECRET` plus bridge URL or port |
| `M79_ORACLE_REPORT_DIR` / `M79_ORACLE_CHECK_ROOT` | oracle report/check roots (preset under the run) |
| `KRASNOKAMEN_ENTITY_BENCH_*` | scale workload knobs (preset under the run's `bench/`) |
| `KRASNOKAMEN_VALIDATION_RUN_DIR` | canonical run dir injected into every backend |

Native-client variables (`KRASNOKAMEN_CLIENT_*`) are in the
[real-client prerequisites](development/real-client-gates.md#prerequisites).
`run` sets `KRASNOKAMEN_HARNESS_LOG_STDOUT=1` for its subprocesses so readiness
events and captured logs stay available (`RUST_LOG` controls `logs/debug.log`);
operator launches never set it, `KRASNOKAMEN_HARNESS_CPU_SCOPE` or the memory
variables.

## Decoder fuzzing (`fuzz`)

The independent `fuzz/` workspace is outside L2 and code-health; targets,
budgets, corpus retention and reproduction live in
[decoder fuzzing](../fuzz/README.md). CI's `ci.yml` `fuzz` job is canonical
(six legs, 60 s per target on push/PR); `fuzz-nightly.yml` runs the same legs
for 900 s at `17 3 * * *` or the dispatched `seconds`. Keep both workflows'
flags in step; `release-build` depends on fuzz. A local dev-profile smoke is not
release-window coverage. Minimize findings, add owning-crate regressions and
ledger rows; never bundle parser fixes into fuzz-tooling changes.

## Other tools

- [`tools/dev/README.md`](../tools/dev/README.md): owner-host runner, watchdog,
  `scene-slot.sh`, pins, worktrees, fresh-machine setup (rules:
  [heavy commands](development/heavy-commands-and-memory.md));
  [`test-server/README.md`](../tools/dev/test-server/README.md) belongs to
  [test server operations](development/test-server-ops.md).
- [`tools/worldgen_diff/README.md`](../tools/worldgen_diff/README.md): saved-block
  worldgen comparison against vanilla ([results](evidence/worldgen-diff/README.md)).
- [`examples/loader-live-gate/README.md`](../examples/loader-live-gate/README.md):
  the two-owner Loader plugin fixture.
- The client MCP (in the Loader dev client and the native client) is an optional
  helper; owner-workstation extras are never required. Do not route provider
  traffic through proxies or enable MCPs the owner did not ask for. CodeGraph's
  MCP server is disabled on the owner host; `codegraph init` builds its ignored
  `.codegraph/` index when the CLI is wanted.
- Agent runtime: account routing and model choice are separate. Use the running
  version's supported account policy for owner-designated work accounts; never
  copy credentials or rotating OAuth refresh tokens between machines, log
  accounts out or spend saved resets to force routing. Record the running agent
  build, its process scope and the time of any memory observation; RSS minus JS
  heap is not live native allocation. Diagnostic bundles can hold credentials
  and transcripts: keep them private and share only inspected, redacted,
  explicitly authorized evidence.

## Negative-code gate

The reviewer the [owner rules](RULES.md#commits-reviews-and-publication) require (promotion batches of five or more rows;
persistence, protocol, custody or battle-core changes before commit; releases)
also checks: scope fit, duplicate mechanisms (not repeated call-site code), fake
or wrong abstractions (caller-specific flags or branches in shared code),
wider-than-needed configuration and stale docs. Docs-only and test-only slices
get no reviewer, and never several reviewer roles unless the owner asks. A
reviewed change's report says what this review found.
