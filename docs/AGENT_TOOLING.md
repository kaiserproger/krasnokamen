# Agent Tooling

`AGENTS.md` contains the rules; this file is the local wiring around the
validation harness. `python3 -m tools.harness list` is authoritative for the
profile catalog and `python3 -m tools.harness explain` for the scenario catalog;
prose here never overrides them. Never invoke `tools/harness/backends/`
directly: those engines are private and only a profile run leaves a canonical
receipt.

Related pages: [real-client gates](development/real-client-gates.md) (native
client prerequisites, graphical gates, the render oracle),
[heavy commands and memory](development/heavy-commands-and-memory.md) (the
`tools/dev/mem-run.sh` runner), [catalog load](development/catalog-load.md),
[performance windows](development/performance-windows-and-profiling.md).

## Prerequisites

- Repository root as cwd; `python3`, `cargo` (toolchain pinned by
  `rust-toolchain.toml`) and, for `run test`/`correctness`, `cargo-nextest`
  (CI installs 0.9.143).
- Linux with a systemd user scope for the harness's resource bounds (see
  [Resource bounds](#resource-bounds)); without one a run prints a warning and
  continues unbounded.
- Java profiles (`java`, `--client java`): Java 25
  (`java -XshowSettings:properties -version` must report specification 25), an
  executable `../solaris-loader/gradlew` (`SOLARIS_LOADER_ROOT` selects another
  Loader checkout), a Minecraft client jar via `SOLARIS_CLIENT_JAR` (CI
  downloads the version in the Loader's `gradle.properties` and verifies
  Mojang's SHA-1), and Xvfb for every graphical profile. Wasm package coverage
  comes from `standard-pack` over `../solaris-default-plugins`
  (`SOLARIS_DEFAULT_PLUGINS_ROOT`), not from an in-core plugin copy.
- Native client profiles (`--client native`): a built `solaris-client` and a
  prepared data root; see the prerequisites block in
  [real-client gates](development/real-client-gates.md#prerequisites).
- Real-client and scale profiles need a **release** `solaris-v2` as
  `SOLARIS_SERVER_BINARY` (`cargo build --release --locked -p mc-edge --bin
  solaris-v2`; on the owner host a path from `tools/dev/pin-release`).
- Free loopback ports (default MCP `39095`, scenario servers such as
  `25565`/`25567`, per-run bridge ports); a run that cannot bind fails closed.
  Readiness on Linux uses inotify/pidfd event waits, Xvfb process management and
  MCP state events, never sleeps.
- Neither Mojang bytes nor `data/vanilla/*` nor `.analysis/*` enter Git.

## Harness entrypoint

Run from the repository root:

```sh
python3 -m tools.harness list
python3 -m tools.harness explain [scenario-id]
python3 -m tools.harness run PROFILE [--release] [--timeout-seconds N] [--client java|native] [--platform fabric|neoforge|forge] [--plugins-dir PATH] [--windowed] [--server-bin PATH] [backend args]
python3 -m tools.harness run scenario <id>
python3 -m tools.harness client [--check] [--platform fabric|neoforge|forge]
python3 -m tools.harness mcp [--server-address host:port] [--exercise-input] [--disconnect] [--scenario-id ID]
python3 -m tools.harness plan [--base REV] [--paths PATH...] [--json]
```

Typed-option constraints (the run fails closed with exit 2 on violation; the
source of truth is `tools/harness/__main__.py`):

- `--server-bin` is only supported by `loader-live` (immutable candidate
  executable). Other launch paths honor the read-only `SOLARIS_SERVER_BINARY`
  environment candidate instead of building.
- `--plugins-dir` requires `--client native`, `java-v2-p5-towns`, `loader-live`
  or `graves`. The directory must already exist; it is used as-is.
- `--windowed` requires `--client native` (`scene-shot` and `settlement-map-qa`
  also accept `--client java`).
- `--client native` is enabled only for the profiles in the allow-list in
  `tools/harness/__main__.py`; any other profile fails before launching a client.
  Most `list` descriptions say where native is supported.
- `--release` only changes `build`; every other profile ignores it.
- `--timeout-seconds N` overrides the scenario default where the backend honors
  it (`core-client`/`inventory` 600 s, `loader-live` 600 s, `playable` 1500 s,
  `replay` 180 s).

Receipt statuses are `passed` | `failed` | `prepared` | `interrupted`. Only real
completion is `passed`. `--check`/`--prepare` (and other preparation-only modes)
report `prepared` with exit 0, never a gameplay pass. Unknown profiles,
unlaunchable or nonzero commands, and missing scenario evidence fail closed with
a nonzero exit. Every timeout is failure, never success.

Real-client and functional-native runs check prerequisites before their
profile launches: set `SOLARIS_SERVER_BINARY` to a pinned executable and, for
native clients, `SOLARIS_CLIENT_BINARY` and the client data paths described in
the real-client guide. Pooled `targets/slot-*` binary paths are refused. Scene
JSON step contracts are checked before startup. Failed receipts include
`details.failure` with available binary hashes, logs and child exit or signal;
`phase: unknown` means the failing backend has not identified its phase.

The shared Java client launcher sets both `HOME` and JVM `user.home` to `.home`
inside that client's game directory. Forge's home-derived Minecraft assets
therefore use that private directory; provision assets there when needed.
Gradle's separately configured cache is not isolated by this setting.

## Profiles and scenarios

Do not copy the catalog into prose; ask the harness:

- `python3 -m tools.harness list` prints every profile with its family
  (`rust`, `harness`, `loader`, `plugins`, `release`, `real-client`,
  `functional-native`, `oracle`, `scale`, `fixture`, `terrain`, `debug`) and a
  one-line scope.
- `python3 -m tools.harness explain` lists scenario ids; `explain <id>` prints
  one entry's profile, exact selector, required tests, declared prerequisites,
  assertion sources, and what it proves and does not prove. `explain` starts no
  server or client, downloads nothing, needs no credentials and writes no
  artifact.
- `run scenario <id>` dispatches to that existing profile plus its exact
  selector through the one shared run path (same artifact directory, bounded
  scope, receipt writer) with no free-form trailing arguments. Graphical entries
  never fall back to TCP evidence.

A named run fails when the id is unknown, extra arguments are passed, a required
test is absent from the log, the engine reports no counts, nothing was selected
or executed, or a selected test failed. Its receipt carries
`details.named_scenario`: the id, evidence kind, resolved profile and selector,
required test paths, bound `scenario_id`/manifest/seed/fixture, prerequisites,
assertion sources, proves/limits, participating repository revisions,
per-command cwds, engine counts (`selected`/`executed`/`passed`/`failed`/
`ignored`/`filtered_out`) and any missing tests.

### Evidence boundaries

Each of these is a limit on what a green receipt means; never state more.

- `correctness` is the L2 close gate (`regressions`, fmt, strict workspace
  Clippy, `code-health`, then the nextest suite). `code-health` is a fail-only
  structural tripwire, not gameplay evidence.
- `core-client` is the server/own-chat continuity subset only (Java default,
  native subset). Its receipt explicitly excludes Wasm, economy and
  Loader-required rejection evidence.
- `logical-native` and `native-scale` are functional-native: a cheap headless
  movement/look/rejected-use subset and non-exclusive CPU/RSS canaries. Neither
  is `playable`, `regression` or `replay`; `logical-native --windowed` is
  rejected.
- `regression` rejects protocol bots, wire probes, `mc-test-harness` clients and
  mocks as participants. `playable` is the complete twenty-minute no-debug
  survival loop (`docs/playable/real-client-playable-loop.json`, fresh per-run
  world): never shorten it or substitute a debug scenario. `replay` is the core
  seed-81 action replay (`docs/real-client-regression/manifests/core-replay-seed-81.json`),
  not a general regression short-cut. `regression`, `playable` and `replay`
  append `--run` unless a mode token is already present (`--check`, `--prepare`,
  `--run`, `--validate-run`): `--check` validates the Gradle runClient adapter,
  `--prepare` creates a run directory and observation templates,
  `--validate-run <dir>` checks an existing run shape.
- The regression runner is fail-closed on scenario provenance: for `--check` and
  `--run`, `SOLARIS_REAL_CLIENT_AGENT_SCENARIO` must name exactly one scenario in
  `SOLARIS_REAL_CLIENT_MANIFEST`; an implemented but undeclared debug scenario is
  not valid evidence for a no-debug playable manifest.
- `oracle` without `--run` is readiness inspection only (`prepared`,
  blocked/degraded when artifacts are absent); `oracle --run` executes
  comparisons against the local artifacts each manifest names. `quirk-parity`
  compares a real vanilla oracle with Solaris; see
  [QUIRK_PARITY.md](QUIRK_PARITY.md).
- `catalog-load` and `constantinople`: [catalog load](development/catalog-load.md)
  and [Constantinople](development/constantinople-fixture.md). Smoke runs and
  `ASSISTED_CHAIN_ONLY` are never capacity results.
- `seed-review` and `seed-contact-sheet` capture terrain evidence and never grant
  owner `ACCEPT`; that decision lives outside the harness.
- Native `--windowed` receipts prove what the retained frames and predicates
  show; headless native receipts (`details.headless=true`) prove received state
  only. The details are in
  [real-client gates](development/real-client-gates.md#evidence-classes).
- Retired `entity-scale`/`living-world-scale` results keep their original
  release-build scope as history; use `catalog-load` for public-protocol
  workloads. Debug numbers are not optimized performance evidence.
- Artifact-building utilities (extract/dump scripts, fixture generators,
  `prepare-real-client-scenario.sh`) are not validation entrypoints.

## Validation plan (`plan`)

`python3 -m tools.harness plan [--base REV] [--paths PATH...] [--json]` prints the
validation a change requires, before you start and again before you report.
It reads the changed files (default: `git diff --name-only` against `--base`,
default `HEAD`, plus untracked files, in the cwd tree; `--paths` replaces the
diff, and `../solaris-client/...` names a sibling repository) and prints one
deduplicated list. Each entry has the exact gate, `why` (the rule's doc path
and section), `runs` (1, or 3 for a variable profile), whether a negative control
is required, and whether an independent reviewer is required and of which kind
(persistence, protocol, custody, ledger batch of five or more rows, innovation).
Rules of several paths are unioned; the strictest run count wins.

The rules are the `RULES` table in `tools/harness/plan.py`, one row per rule with its
doc source; it restates AGENTS.md (levels L0/L1/L2, reviewers, gate hygiene),
`docs/architecture/persistence.md`, `docs/development/protocol-changes.md`,
[real-client gates](development/real-client-gates.md), the
[bug ledger](bugs/README.md) and the sibling repositories' own docs. It adds no
policy: change a row only when its cited document changes. A gate that maps to a
real-client profile comes from `AREAS`, `BACKEND_PROFILES` and the scenario
catalog; otherwise the entry reads `no mapped gate: choose from ...`. A profile is
*variable* when it appears in `VARIABLE_PROFILES` (timing-, seed- or
concurrency-dependent behaviour, [gate hygiene](QA_AGENT_PROTOCOL.md#gate-hygiene)).

The path tables are minimal, not exhaustive. A changed file that no rule names, or
that falls in two or more risk domains (persistence, protocol, custody, gate),
is printed as `manual classification required: <files>`; so is any change whose
domain its path does not show, which you must classify yourself. A new direct
dependency, a toolchain change or a new required prerequisite prints `decision
record required` and a reviewer (see
[adding a feature](development/adding-a-feature.md), "New dependencies and components").
Run it from inside the tree to be judged; `plan` is read-only.

## Test gate (nextest)

`run test` (including the `test` stage of `correctness`) runs:

```sh
cargo nextest run --profile default --ignore-default-filter --no-fail-fast --no-tests=fail --workspace --all-targets
```

No retries and no default-filter exclusions locally: the full suite runs even
after failures, and a missing binary, failed test or empty selection fails
closed. Trailing package/target/name selectors use the same runner, for example
`python3 -m tools.harness run test -p mc-core --lib inventory`. Criterion
`harness = false` bench targets stay in the all-targets gate in test mode;
explicit `cargo bench` remains the performance-measurement path. Per-test
durations are retained in `test.log`.

Concurrency: `RUST_TEST_THREADS` (first priority), else `NEXTEST_TEST_THREADS`
(default `num-cpus`), selects the gate default; nextest's `--test-threads=N`
overrides it (place it before any `--`). Unsupported Cargo/libtest and
non-executing flags such as `--no-run` are rejected, never rerouted to another
runner. The `component-host` nextest group keeps the measured four-concurrent-test
host budget; other packages fill the remaining CPU slots. Do not add a
process-wide guest mutex/semaphore or change production Wasmtime limits for a
heavy run; use an explicit environment override for a proven-heavy scope instead.

CI short run: `SOLARIS_NEXTEST_PROFILE=ci` selects the `ci` profile in
`.config/nextest.toml`, whose `default-filter` is a **fixed list of test names**
that were slow (over 3 s) in one L2 run on 2026-10-18. It is not recomputed:
a test added or slowed since is not excluded. The local gate keeps the full
suite. Successful `test`
receipts record `memory_peak_bytes`, `memory_limit_bytes`, `memory_cgroup`,
`memory_events`, `memory_swap_peak_bytes`, `rust_test_threads_env`,
`test_runner`, `nextest_test_threads_env` and `nextest_profile`; unavailable
counters are reported as unavailable, never zero. Scope peaks include Cargo,
compilation and cached file pages: for a concurrency decision, build first with
`cargo test -p PACKAGE --test BINARY --no-run`, run only that executable in its
own mem-run scope and read `memory.stat` anonymous versus file pages.

Inventory conservation property: `inventory::conservation_tests` in `mc-core`
(proptest, random client operation sequences from one or two players through
`run_tick`; item totals must never change). It runs 64 cases by default (about
80 s in a debug build, most of it loading the vanilla catalog);
`PROPTEST_CASES=3000 cargo test -p mc-core --lib inventory::conservation_tests
-- --nocapture` runs the long form (about 13 min) and prints the per-operation
coverage table. A failure prints the shrunk operation sequence; turn it into a
deterministic regression test (shrunk cases are not persisted).

Optional CRAP measurement: `cargo-llvm-cov` and `cargo-crap` measure line
coverage and cyclomatic complexity per function on one source tree. Retain the
test command, revision, raw LCOV, analyzer output and exact test scope; a partial
suite is not workspace coverage, and line coverage is not branch or path
coverage. Missing or ambiguous function coverage is unknown, not zero or 100%.
The owner's `CRAP < 10` rule for new or changed functions applies only after 1.0
ships; it is not a pre-release gate.

## Code-health ratchets

`cargo run -p xtask -- code-health` (harness `run code-health`) measures
`mc-core` production code with `syn`, never text. Comments, doc comments,
attributes, `#[cfg(test)]` and `#[test]` items and `*_tests.rs` files are outside
every number. `--root <dir>` measures another checkout read-only. Five ratchets
fail when they rise above their ceiling in
`crates/xtask/src/code_health/ratchet.rs`:

- modules in the largest cycle of top-level modules (Tarjan over
  `crate::`/`super::` paths and root re-exports): no module may join it
  ([ADR 0021](decisions/0021-build-time-and-crate-split.md#cycle-ratchet-implemented-2026-11-07));
- string literals whose value starts with `minecraft:`;
- `==` or `!=` with a `.path()` call on either side;
- `panic!`;
- `.push(..)` on `merge_block_writes`.

Macro bodies count too: a body that parses as comma-separated expressions
(`format!`, `matches!`, `assert!`) is visited like code; any other body
(`macro_rules!`, `vec![x; n]`, tracing fields) is read token by token.

A value below its ceiling prints `ceiling can drop to N`: lower the constant in
the same change. Raising a ceiling is an explicit edit of the constant with a
reason comment; there is no flag or override. The cycle's production lines are
a printed trend, never a failure: features inside cycle modules add lines.

The report prints a per-module breakdown for each count and these known false
positives: vanilla ids in data and registry tables and in protocol, NBT and
Anvil schemas; `path()` on a non-identity type (matched by method name); a
panic on a truly impossible internal state; the push inside
`TickState::write_block` itself. Not seen: a module dependency through a type's
methods without a module path, ids without the `minecraft:` prefix, and a
`.path()` right operand inside a macro body that is not expressions. For the
cycle it prints the most connected modules and decoupling hints: the modules
that leave the cycle for the fewest cut edges.

## Resource bounds

`run` and `client` put the whole invocation into one bounded systemd user scope
before doing any work; every child (Cargo, test binaries, harness
server/client/Xvfb, private backends) inherits that one cgroup, so each bound is
a total, not per-process.

| Property | Default | Env |
|---|---|---|
| `CPUQuota` (+`CPUWeight=50`) | every physical core | `SOLARIS_HARNESS_CPU_QUOTA` |
| `MemoryHigh` | `off` (no throttle) | `SOLARIS_HARNESS_MEMORY_HIGH` |
| `MemoryMax` | `12G` for `test`/`correctness`; `4G` for other profiles and clients | `SOLARIS_HARNESS_MEMORY_MAX` |
| `MemorySwapMax` | `1G` | `SOLARIS_HARNESS_MEMORY_SWAP_MAX` |

```sh
python3 -m tools.harness run correctness              # the defaults above
SOLARIS_HARNESS_CPU_QUOTA=150% python3 -m tools.harness run test
python3 -m tools.harness run test -p mc-core --lib inventory  # focused Rust loop
SOLARIS_HARNESS_MEMORY_MAX=12G python3 -m tools.harness client   # heavier client runs
SOLARIS_HARNESS_CPU_QUOTA=off  python3 -m tools.harness run test  # unbounded, opt-out
```

`run`/`client` refuse admission when `MemAvailable` is below the selected
`MemoryMax` + 1 GiB, naming the knob to change; that refusal is not acceptance
evidence. On the shared agent host heavy commands additionally run under
`tools/dev/mem-run.sh` (it exports `SOLARIS_HARNESS_CPU_SCOPE=1` so the harness
does not open a second scope); the runner, its lanes and the Cargo
rules are in [heavy commands](development/heavy-commands-and-memory.md). A run
without a user systemd scope prints a warning and continues unbounded; the
receipt records the profile's own result either way.

## Receipts and failure diagnosis

Each `run` creates an isolated `.analysis/validation/<UTC stamp>-<profile>-<id>/`
directory with `result.json`, per-command logs and scenario evidence.
`result.json` records `profile`, `client`, `status`, `scope`, `commands`,
`exits` (an unlaunchable command records `null`, never a fabricated exit code),
`logs` (repo-relative), `started_at`/`ended_at`, `duration_seconds`, `details`
and `error`. An unknown profile still writes a `failed` (`unknown-profile`)
receipt listing the known profiles. Graphical profiles share one process/readiness
implementation and clean up their owned server, client and Xvfb processes;
readiness is driven by process, log and client-state events.

For a failed run, `details.failure` records the observed lifecycle `phase`
(`preflight`, `server_start`, `client_start`, `join`, `scenario`, `assertion`,
`teardown`, or `unknown`), child `exit_code` or terminating `signal` when
observed, log paths, and server/client binary paths and SHA-256 hashes when
available. A null field means the information was not observed; a cleanup
exit must not be confused with the failure that preceded it. Coverage is
currently limited to instrumented shared helpers and probe sections: an
unclassified failure remains `unknown`.

Preflight selects prerequisites from the profile's declaration in
`tools/harness/profiles.py`, not from its scope label. Native profiles that
launch a client read its schemas with `solaris-client mcp-schema`; the command
prints JSON with `tools` (windowed) and `headless_tools` arrays from the same
catalog as MCP `tools/list`, without starting a game. A release that predates
the command (it answers `unknown_command: mcp-schema`) still runs, with
`details.mcp_schema` recorded as not validated; any other export failure fails
preflight. For Java MCP profiles, preflight
compiles the selected Loader checkout's `ClientMcpTools` definitions using the
checkout's cached Gson dependency. Scene-shot input-derived MCP calls and
literal calls in the selected backend and named scenario sources are checked
against those definitions. Computed calls and dynamically supplied values are
not proven by static validation.

- Start from `result.json`: take the nonzero entry in `exits`, read the matching
  `commands` argv and `logs` file. `details.error` names the failing backend;
  `details.mode` tells preparation (`prepared`) from execution.
- Hosted `test` and `loader` CI jobs upload their failed receipts as
  `test-failure-receipts` and `loader-failure-receipts` (two-day retention). Read
  the actual command error there; the console's `see .../test.log` line alone is
  not the underlying failure.
- Failing backends fail closed: nonzero commands raise with the log path, and
  `loader-live`/`core-client`/`inventory` additionally raise when the scenario
  report is not `passed`. A `prepared` receipt after expected execution means a
  mode flag was passed (or defaulted) wrong, not a green run.
- `KeyboardInterrupt` yields `interrupted`, exit 130, keeping every command, exit
  and log. Owned processes are stopped by the shared runtime; if a port stays
  held, stop the stray Gradle/Minecraft/Xvfb process by PID. Never delete
  `.analysis/validation/` evidence or historical logs.
- Second real-client (multiplayer) failures almost always mean a missing
  `SOLARIS_REAL_CLIENT_SECOND_AGENT_SECRET` or bridge URL/port pair; the backend
  errors name the exact variable.
- `civilization-seed-qa` awaits each console sprint's exact `tick sprint
  finished` event under the remaining scenario deadline, not the 120-second MCP
  request cap. Slow simulation is recorded in sprint TPS and `MC_CORE_PHASE_LOG`
  phase timings; neither elapsed time nor a missing completion is success.
- Full workflows for graphical failures: [real-client gates](development/real-client-gates.md)
  and [live-gate triage](development/live-gate-triage.md).

## Java and Loader client launch (`client`)

The default (platformless) development client lives under the sibling
`../solaris-loader/fabric-agent/`; despite that name it is a **NeoForge** mod
(entrypoint `dev.solaris.agent.neoforge.SolarisClientAgentMod`, fixed launch task
`:fabric-agent:runClientMcp`). The three Loader adapters (`loader-fabric`,
`loader-neoforge`, `loader-forge`) register the same Configuration-state
manifest/ack payloads through their native networking APIs and launch as
`:loader-<platform>:runClientMcp`. The Minecraft version of this client is the
Loader's `minecraftVersion` in its `gradle.properties`.

`client` (launch and `--check` alike) needs these per-run values. The token is a
random local bearer secret you generate, not an account credential:

- `SOLARIS_CLIENT_MCP_TOKEN`: random bearer token, no default; without it
  `client` exits 2 before Gradle starts.
- `SOLARIS_CLIENT_MCP_PORT`: free IPv4 loopback port, default `39095`.
- `SOLARIS_CLIENT_MCP_GAME_DIR`: isolated game dir; default
  `../solaris-loader/fabric-agent/run-mcp`, or
  `.analysis/minecraft-loader-mcp/<platform>` when `--platform` is given.
- `SOLARIS_CLIENT_MCP_USERNAME`: `1..16` ASCII letters/digits/underscores;
  default `SolarisMcp` (platformless) or `SolarisLoader` (Loader platforms).

```sh
SOLARIS_CLIENT_MCP_TOKEN=local-check-token python3 -m tools.harness client --check
export SOLARIS_CLIENT_MCP_TOKEN="$(openssl rand -hex 32)"
export SOLARIS_CLIENT_MCP_PORT=39095
export SOLARIS_CLIENT_MCP_GAME_DIR=.analysis/minecraft-mcp-primary
export SOLARIS_CLIENT_MCP_USERNAME=SolarisMcpA
python3 -m tools.harness client
python3 -m tools.harness client --platform neoforge   # Loader client on another port/dir/user
```

Normal launch refuses to start when the MCP port is already in use. An HTTP 401
means the caller reached an endpoint with a different bearer token: verify the
port and process instead of retrying it as a transient failure. `--check`
validates Java 25, configuration, MCP transport tests and the Gradle adapter
without launching Minecraft. Use a second port, token, game directory and
username for multiplayer gates. The endpoint is `http://127.0.0.1:<port>/mcp`.

MCP smoke against an already-running client (`SOLARIS_CLIENT_MCP_TOKEN` still
required):

```sh
python3 -m tools.harness mcp
python3 -m tools.harness mcp --server-address 127.0.0.1:25565 --exercise-input --disconnect
```

The smoke checks the tool catalog, calls `minecraft_observe`, and optionally
connects, waits for Play, reads the block below the player, scans blocks, lists
entities, reads the recipe book, exercises one input, runs a deterministic
in-client scenario (`--scenario-id`) and disconnects. Inventory, entity, login
and client lifecycle waits block on packet/lifecycle state notifications; the
separate tick notification drives only tick progression itself. Canonical
interaction checks fence reach, raycast and authoritative world state first.

## Per-run environment overrides

The harness never reads a config file for these; export them before `run`:

| Variable | Effect (default) |
|---|---|
| `RUST_TEST_THREADS`, `NEXTEST_TEST_THREADS` | `run test` default concurrency (CPU count; `RUST_TEST_THREADS` wins). Nextest `--test-threads=N` overrides. |
| `SOLARIS_NEXTEST_PROFILE` | Nextest profile (`default` local full suite; CI sets `ci` short run) |
| `SOLARIS_REAL_CLIENT_MANIFEST` | Regression pack manifest (`docs/real-client-regression/manifests/m94-regression-pack.json`; `playable` presets its loop manifest, `replay` its seed-81 manifest) |
| `SOLARIS_REAL_CLIENT_RUN_ROOT` | Local artifact root (preset under the run artifact `regression/` dir) |
| `SOLARIS_REAL_CLIENT_SERVER_CONFIG` | Server config (`example.toml`; `playable` presets `playable.toml`) |
| `SOLARIS_REAL_CLIENT_FRESH_WORLD` | `1` copies the config into the run dir with a fresh per-run world (`playable`/`replay` preset `1`) |
| `SOLARIS_REAL_CLIENT_SERVER_SEED` | Optional signed 64-bit decimal overriding `data.seed` |
| `SOLARIS_REAL_CLIENT_SERVER_ADDR` | Server address for the in-client driver (`127.0.0.1:25565`) |
| `SOLARIS_REAL_CLIENT_TIMEOUT_SECONDS` | Gradle runClient adapter timeout (180; `playable` 1500, `replay` 180). `--timeout-seconds` overrides it for `playable`/`replay`. |
| `SOLARIS_REAL_CLIENT_SERVER_READY_TIMEOUT_SECONDS` | Server-readiness wait (120) |
| `SOLARIS_REAL_CLIENT_AGENT_BRIDGE_URL` / `AGENT_SECRET` / `AGENT_PORT` / `AGENT_DRIVER` / `AGENT_SCENARIO` | Loopback JSON bridge URL, per-run bridge secret (required for agent-driver mode), bridge port, internal driver path, scenario id. Second-client mode additionally needs `SOLARIS_REAL_CLIENT_SECOND_AGENT_SECRET` plus bridge URL or port. |
| `M79_ORACLE_REPORT_DIR` / `M79_ORACLE_CHECK_ROOT` | Oracle report/check roots (harness presets them under the run artifact) |
| `SOLARIS_ENTITY_BENCH_*` | Scale workload knobs (preset under artifact `bench/`) |
| `SOLARIS_VALIDATION_RUN_DIR` | Canonical run dir injected into every backend env; used by seed-review evidence |

Native-client variables (`SOLARIS_CLIENT_*`) are in the
[real-client gates prerequisites](development/real-client-gates.md#prerequisites).
Normal operator launches never set `SOLARIS_HARNESS_LOG_STDOUT`,
`SOLARIS_HARNESS_CPU_SCOPE` or the memory-cap variables beyond the recipes above;
the canonical `run` entrypoint sets `SOLARIS_HARNESS_LOG_STDOUT=1` for its
subprocesses so readiness events and per-run captured logs stay available
(`RUST_LOG` controls `logs/debug.log`, not the captured threshold).

## Code and client tools

The Minecraft client MCP (embedded in the Loader dev client and the native
client) is an optional helper; normal work uses the repository rules, the
harness and the agent's own tools. Owner-workstation extras are never required.
Do not route provider traffic through proxies or enable MCPs the owner did not
ask for. The CodeGraph MCP server is disabled on the owner host; the CLI remains
installed for occasional questions. Build its ignored `.codegraph/` index with
`codegraph init` when needed.

## Other tools in the repository

- [`tools/dev/README.md`](../tools/dev/README.md): the memory-capped runner
  `mem-run.sh`, its watchdog, `scene-slot.sh` and the fresh-machine setup (rules:
  [heavy commands](development/heavy-commands-and-memory.md)); its
  [`test-server/README.md`](../tools/dev/test-server/README.md) belongs to
  [test server operations](development/test-server-ops.md).
- [`tools/worldgen_diff/README.md`](../tools/worldgen_diff/README.md): saved-block
  worldgen comparison against stock vanilla (results:
  [worldgen evidence](evidence/worldgen-diff/README.md); working method:
  [performance windows](development/performance-windows-and-profiling.md)).
- [`fuzz/README.md`](../fuzz/README.md): `cargo-fuzz` targets for client-controlled
  decoders (protocol frames, packets, Loader wire, NBT, edge play). Outside the
  root workspace, so L2 and CI never build it; every finding becomes a regression
  test in the owning crate.
- [`examples/loader-live-gate/README.md`](../examples/loader-live-gate/README.md):
  the retained two-owner Loader plugin fixture.

## Negative-code gate

The reviewer that AGENTS.md "Working rules" requires (a promotion batch of five
or more rows; a persistence, protocol or custody change before commit; a
release) includes negative-code checks: scope fit, duplicate mechanisms (not
repeated call-site code), fake abstractions and wrong ones (caller-specific
flags or branches in shared code), wider-than-needed config and stale docs.
Slices without a review point (docs-only, test-only) get no reviewer. Never
run multiple reviewer roles unless the owner explicitly asks. A reviewed
change's report says what the negative-code review found.
