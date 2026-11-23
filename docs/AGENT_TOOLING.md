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

Run from repo root with Python 3, the pinned Rust toolchain and cargo-nextest
(CI 0.9.143). [Contributor setup](../CONTRIBUTING.md#prerequisites) covers builds.
Linux systemd user scopes provide [resource bounds](#resource-bounds);
without one the harness warns and continues unbounded.

- Java: specification **25**, executable sibling Loader `gradlew`, client jar
  in `KRASNOKAMEN_CLIENT_JAR` (CI uses Loader `gradle.properties` and Mojang SHA-1),
  Xvfb for graphical profiles. `KRASNOKAMEN_LOADER_ROOT` overrides the sibling;
  `standard-pack` uses `KRASNOKAMEN_DEFAULT_PLUGINS_ROOT`, never an in-core copy.
- Native: built client and prepared data root; exact
  [client prerequisites](development/real-client-gates.md#prerequisites).
- Real-client/scale: release server in `KRASNOKAMEN_SERVER_BINARY`, owner-host
  `pin-release`, never pooled targets. Elsewhere build with
  `cargo build --release --locked -p mc-edge --bin krasnokamen` and use the
  [environment block](development/real-client-gates.md#3-environment-block).
  Free loopback ports (MCP default 39095, scenario servers commonly 25565/25567
  and per-run bridge ports). Bind failures fail closed; readiness uses
  inotify/pidfd/log/MCP events, not sleeps.
- Mojang bytes, `data/vanilla/*` and `.analysis/*` never enter Git.

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

Typed options are owned by `tools/harness/__main__.py`; violations fail exit 2:

| Option | Boundary |
|---|---|
| `--server-bin` | Immutable executable for `loader-live`, `protocol-matrix`, `java-matrix`, `native-matrix` or `matrix-random`; other paths use `KRASNOKAMEN_SERVER_BINARY`. |
| `--plugins-dir` | Existing directory for native, `java-v2-p5-towns`, `scene-shot`, `loader-live`, `graves`, matrix; matrix stages its own set. |
| `--windowed` | Native only, except scene-shot/settlement-map-qa also accept Java. |
| `--client native` | Only CLI allow-listed profiles; `list` describes support. `screens-voice-qa --client native --windowed` dispatches native/Fabric legs. |
| `--release` | Changes `build` only; other profiles ignore it. |
| `--timeout-seconds` | Per-stage command deadline, otherwise none; overrides backend scenario default where honored (core-client/inventory/loader-live 600 s, playable 1500, replay 180). |

**Loader event-only readiness:** `loader-live` sets
`client_ready_timeout_seconds=None`. Cold NeoForge compiles ~7300 sources and
Gradle's launch task remains alive with the game: neither a build timeout nor
process completion proves successful launch. Wait for the owned MCP listener,
fail on process exit; cancellation tears down the client group. Receipt allowance
is `null`; scenario deadlines are unchanged, other Java callers retain 180 s.

Statuses: `passed`, `failed`, `timeout`, `prepared`, `interrupted`.
Only completed execution passes; preparation/check modes return `prepared`
even with exit 0. Unknown/unlaunchable/nonzero commands, absent scenario evidence
and all timeouts fail closed. Timeout or SIGINT/SIGTERM/SIGHUP tears down stage
descendants including separate groups and records the timeout/signal.

Preflight rejects pooled `targets/slot-*` pins and invalid scene steps before
launch; native also needs `KRASNOKAMEN_CLIENT_BINARY` and data paths.
Failure evidence and schemas: [receipts](#receipts-and-failure-diagnosis).

Shared Java launch isolates `HOME` and JVM `user.home` to `<game>/.home`
(provision Forge assets there; Gradle cache is separate). It retries only explicit
render-thread observation timeouts inside the bootstrap deadline, still requires
the actual title screen and propagates other MCP errors. The 512 MiB game heap
belongs only to runClientMcp JavaExec/prepared/plain Java, never an injected
`JDK_JAVA_OPTIONS` affecting compiler JVMs; caller-supplied options remain intact.
Vanilla-world roundtrip is normal-lane correctness, not `-x` measurement.
Named private audio captures monitor only their named sink and retain `<sink>.f32le`.

## Matrix runner status

`protocol-matrix`, `native-matrix`, and `java-matrix` share the matrix adapters.
Adapter guidance: [matrix domains](../tools/harness/matrix_domains/README.md),
[containers](../tools/harness/matrix_domains/containers.md), and
[platform coverage](../tools/harness/matrix_domains/platform.md).

True-vanilla Java needs Loader's premain event transformer and a direct,
non-symlink protocol-775 `KRASNOKAMEN_LEGACY_CONTENT_CACHE`. Historical Play
entry at `<work>/wt/FuzzE2E/.analysis/validation/20261107T173128-java-matrix-hymtay_4/`
later blocked on missing eye-height observation: **not gameplay acceptance**.

Experimental `matrix-random` includes conflicting same-chest-slot quick moves
with ≥2 bots. Its opt-in consumed-input/tick log must be complete; a fixed action
seed does not make network scheduling deterministic. Fixture setup removes dropped
stock and waits for observer-confirmed emptiness. Conservation covers all item
types in x/z 24–44, y 79–90, not unrelated natural drops.
Each player's own received inventory supplies private inventory evidence.
Restart first tests carried-stack custody, then crashes after transfer/WAL fence;
the second comparison retains per-player/chest slots, item fields/component SNBT
and observed block states. Disconnect destination is not predicted.
Neither replay determinism nor crash atomicity follows without real-server receipts.

`tools/dev/scenario-nightly --install --server-bin /absolute/pin/krasnokamen
--vanilla-data /absolute/vanilla-data` installs the owner-host 03:00 local user
timer; disable with `systemctl --user disable --now solaris-scenario.timer`.
It requires a verified pin/imported data, runs low-priority through mem-run with
a campaign time budget, and defaults artifacts to `<work>/receipts/scenarios`.
Campaign `result.json` retains manifest, requested/attempted seeds and reductions;
each seed retains command, step/bot counts, server commit/hash, harness fingerprint
and result. Full pass receipts may be pruned; older failure artifacts may be
removed under its configured campaign retention, but compact seed records remain.
Disk budget is checked **between** seeds (not a quota); a seed/reduction may exceed
it. Owner-host admission checks ≥100 GiB before each seed/reduction world run,
else stops `disk_floor`; this is not permission to ignore the host's 250 GiB policy.

## Profiles and scenarios

`battle-crash` drives the battle runner with a live kit, a separately
acknowledged `/give`, SIGKILL and recovery at the four WAL/checkpoint boundaries.
It requires `KRASNOKAMEN_SERVER_BINARY`, `KRASNOKAMEN_CLIENT_BINARY`,
`KRASNOKAMEN_BATTLE_CLIENT_ROOT` and `KRASNOKAMEN_BATTLE_WORLD`; run
`python3 -m tools.harness run battle-crash`. Each cut retains the crash copy,
published WAL and backup. The ordinary production option `--start-frozen` is
a diagnostic recovery barrier: it freezes autonomous simulation before the
first tick, not all mutation. Joins, commands, plugins, chunk generation,
generated-entity admission, WAL and checkpoints continue; `tick unfreeze`
resumes simulation. This harness uses an explicitly empty plugin directory,
and takes its initial recovery census as the first console input, before any
client joins or client commands. It is not a universal immutable snapshot.
The internal `world crash-census --copy` command may repair redb metadata in
the world it opens: give it only the harness-created disposable copy, never a
retained source or a live world. Its entity and player holder census comes
directly from the ordered checkpoint-plus-WAL replay, before `restore_entity`/live
hydration and checkpoint encoding; it shares the replay decoder and WAL fold
with recovery, so defects in those stages can compare equal.
The before-crash live census is only a witness.
Both durable and console holder filters include case-sensitive projectile `item`
and `FireworksItem` stacks, as well as the ordinary `Item` holder.
G3 covers entity and player holders only, not block-entity containers (chests,
furnaces, hoppers or shulkers).
The holder projection mirrors mc-core capture encoders; drift in compared fields
fails closed as `changed`, but a key excluded by both filters remains invisible.
The battle kit gate retains its export-based censuses: its separate client-checked
issued kit and cross-phase custody reconciliation reject restore-only loss even
when both exports lose the same piece. Export agreement alone is not that proof.
Reconnected players are compared while simulation remains frozen. A missing
witnessed arrow without an identity-scoped removal proof remains unverified and makes the cut
incomplete.

The catalog is not copied here:

- `list`: all profiles, family and scope.
- `explain [id]`: exact profile/selector, required tests, prerequisites,
  assertion sources, proves/limits; read-only, no launch/download/credentials/artifacts.
- `run scenario <id>`: shared execution/receipt path with catalog selector,
  no free-form trailing args and no graphical→TCP fallback.

Unknown IDs, missing required tests/counts, empty selection/execution or selected
failures fail closed. `details.named_scenario` binds ID/evidence/profile/selector,
manifest/seed/fixture, prerequisites, sources, proves/limits, repository revisions,
command cwds, selected/executed/passed/failed/ignored/filtered counts and missing refs.
Discovery is not execution: Cargo refs are exact non-ignored function names
([registration](bugs/README.md)), not modules. Ignored local-input cases require
declared input and explicit execution; a normal suite cannot claim them.

### Evidence boundaries

Each of these is a limit on what a green receipt means; never state more.

- `correctness` is the L2 close gate (fmt, strict workspace Clippy,
  `code-health`, the full nextest suite, then registered `regressions`).
  `code-health` is a fail-only structural tripwire, not gameplay evidence.
- `core-client` is the server/own-chat continuity subset only (Java default,
  native subset). Its receipt explicitly excludes Wasm, economy and
  Loader-required rejection evidence.
- `entities` summons its combat fixture with a fresh UUID and waits for that
  exact received entity; naturally spawned pigs do not identify the fixture.
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
  `--run`, `KRASNOKAMEN_REAL_CLIENT_AGENT_SCENARIO` must name exactly one scenario in
  `KRASNOKAMEN_REAL_CLIENT_MANIFEST`; an implemented but undeclared debug scenario is
  not valid evidence for a no-debug playable manifest.
- `oracle` without `--run` is readiness inspection only (`prepared`,
  blocked/degraded when artifacts are absent); `oracle --run` executes
  comparisons against the local artifacts each manifest names. `quirk-parity`
  compares a real vanilla oracle with Krasnokamen; see
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

`python3 -m tools.harness plan [--base REV] [--paths PATH...] [--json]` is read-only
and must run inside the judged tree before you start and again before you report.
Default input: changed/untracked paths against HEAD (or `--base`); `--paths`
replaces it and accepts sibling paths.
It unions gates with exact command, cited policy `why`, strictest run count
(variable profiles ≥3), negative-control and reviewer requirements.

`tools/harness/plan.py` `RULES` restates policy, never creates it: change only with
its cited AGENTS/persistence/protocol/real-client/ledger/sibling document.
`AREAS`, `BACKEND_PROFILES` and scenario catalog supply mapped gates; unmapped ones
say `no mapped gate`. `VARIABLE_PROFILES` follows
[gate hygiene](QA_AGENT_PROTOCOL.md#gate-hygiene).
Unknown paths and multiple/hidden risk domains require manual classification.
New dependencies/toolchains/prerequisites require a decision record under
[adding a feature](development/adding-a-feature.md), not a new review checkpoint.

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

L2 snapshots the full run's fresh nextest JUnit report as `test-junit.xml` in
its artifact directory, including on test failure. Registered Cargo regressions
consume that report instead of listing/building/running the same tests again:
every exact package/function ref must have executed and passed, with missing,
ignored/skipped or failed refs failing the gate with their ledger row IDs.
Stale reports are discarded before execution. L2 does not opt into ignored tests.
Python, client and harness regression refs retain their existing execution.
Standalone `run regressions` still discovers and executes its exact Cargo refs.

For a long, isolated scheduled-block simulation, use
`crate::tick_tests::warp_to_block_tick` to jump to the target block's real
queued deadline, then run the native tick and assert the outcome. This does
not advance other simulation systems: do not use it if movement, random ticks,
other deadlines, or elapsed per-tick effects matter. For each warped behaviour,
retain an ordinary-tick test or compare a small continuous-tick case with the
warped case, including a negative control that skips a consequential tick.
`/tick sprint`, including the civilization-seed QA sprint, removes waits but
still executes every tick.

Live pause tests await `pause_when_empty_tests::wait_for_pause`, which observes
the tick owner's test-only pause acknowledgement and drains only broadcasts
at or before that acknowledged tick. A quiet broadcast channel is not proof
of pause: a busy tick can exceed the old idle-gap window. Tests for continued
simulation await a concrete number of ticks instead.

Raider casting fixtures advance real ticks until the target owner records
acquisition, bounded by `raiders::SEARCH_INTERVAL` for the fixed fixture.
Checkpoint/WAL tests journal every intervening tick, not just the casting tick,
and retain all spell, projectile and recovery assertions.

Concurrency: `RUST_TEST_THREADS` (first priority), else `NEXTEST_TEST_THREADS`
(default `num-cpus`), selects the gate default; nextest's `--test-threads=N`
overrides it (place it before any `--`). Unsupported Cargo/libtest and
non-executing flags such as `--no-run` are rejected, never rerouted to another
runner. The `component-host` nextest group keeps the measured four-concurrent-test
host budget; other packages fill the remaining CPU slots. Do not add a
process-wide guest mutex/semaphore or change production Wasmtime limits for a
heavy run; use an explicit environment override for a proven-heavy scope instead.

CI short run: `KRASNOKAMEN_NEXTEST_PROFILE=ci` selects the `ci` profile in
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

### Bot scenario matrix

`protocol-matrix`, `java-matrix` and `native-matrix` use the same
`matrix_api.Adapter(setup, act, oracle)` contract and `matrix_execution` context
boundaries. The profiles, not a standalone runner, own processes and receipts;
`ProtocolServer` launches through `runtime.start_server`. Missing verbs and
observations raise `MissingCapability` and never count as passing coverage.

The frozen combinatorial specs and generator live in
`tools/harness/matrix_specs/` (`gen_matrix.py`, `subjects*.py`, `domains/`,
`matrix.yaml`, `MATRIX_FREEZE.json`). The frozen matrix describes the historical
26.1.2 cells; its recorded source heads are provenance, not current coverage.
Keep mutable ledger JSONL, copied live plugins and run directories outside Git,
by default in `/run/media/kaiserroman/games/solaris-work/scenarios/`.
`KRASNOKAMEN_MATRIX_DATA` changes that root; `SCENARIO_LEDGER` changes the ledger.
Run only against isolated worlds; never send matrix bots to the live test-server
world. The [adapter contract](../tools/harness/matrix_domains/README.md) covers
the per-row oracle, port ranges and [plugin profiles](../tools/harness/matrix_domains/README.md#readiness-json-state-and-plugin-lanes).
Generation reads this checkout and sibling repositories selected by
`KRASNOKAMEN_WORK_ROOT`. A changed frozen id set requires explicit `--refreeze`.

For a protocol adapter probe, use the pinned release binary and the current
26.3 content cache; its version metadata selects the wire protocol:

```sh
python3 -m tools.harness run protocol-matrix \
  --server-bin "$KRASNOKAMEN_SERVER_BINARY" \
  --vanilla-data /absolute/path/to/data/vanilla \
  --subject block_place.full_block
```

Select a frozen protocol row with `--matrix tools/harness/matrix_specs/matrix.yaml
--matrix-id ID` instead of `--subject`. Java uses `--matrix`, `--id`,
`--ledger`, `--data-dir`; native uses `--client native --windowed --matrix`,
`--only-id`, `--ledger`, `--run-root`. Both graphical lanes require
`--plugins-dir`; the native lane also requires the private hardware-free
`tools/harness/screen_audio_graph.sh` wrapper. Every harness-launched Java
client gets `ALSOFT_DRIVERS=null` in its own environment whatever the caller
exported, plus a private null ALSA config, `PIPEWIRE_REMOTE=/nonexistent`,
`PULSE_SERVER=unix:/nonexistent` and a private `XDG_RUNTIME_DIR`; launch is
refused (`MissingCapability java_audio:*`) if the child environment would carry
anything else, and the check is recorded in `<game dir>/java-audio-isolation.json`.
The Java lane needs no wrapper. On the owner host wrap every run in Main's absolute
`tools/dev/mem-run.sh -m 6G` invocation.

The wrapper points `XDG_DATA_HOME` at its private graph, where the native client
finds no content and exits before MCP is ready. Run the harness through
`sh -c 'XDG_DATA_HOME=<client data XDG> exec python3 -m tools.harness run native-matrix ...'`
inside the wrapper, with that XDG's `krasnokamen-client/` symlinking the client content
(see [real-client gates](development/real-client-gates.md)).

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
every number. `--root <dir>` measures another checkout read-only. Seven ratchet families
fail when they rise above their ceiling in
`crates/xtask/src/code_health/ratchet.rs`:

- modules in the largest cycle of top-level modules (Tarjan over
  `crate::`/`super::` paths and root re-exports): no module may join it
  ([ADR 0021](decisions/0021-build-time-and-crate-split.md#cycle-ratchet-implemented-2026-11-07));
- string literals whose value starts with `minecraft:`;
- `==` or `!=` with a `.path()` call on either side;
- `panic!`;
- `.push(..)` on `merge_block_writes`;
- production items (including associated/foreign items) plus non-item statements
  per hot file: `tick/entity_phase.rs`, `settlements/construction.rs`, `tick.rs`,
  and `entities.rs`, relative to `crates/mc-core/src`. Item statements count once,
  as declarations; inline modules belong to their physical file, external module
  declarations to their parent file. Comments and whitespace cannot change this
  count. The 2026-11-17 `b58021e61` baseline is 734, 1076, 834 and 924 respectively;
  the D9 motion-gate extraction (2026-11-17) took `entities.rs` to 864 (925 on main before the cut);
  the owner-module extraction (`tick.rs` and `tick/entity_phase.rs` into per-owner
  submodules, ADR 0021 Step 3) took them to 261 and 260, measured on its merge
  with main `30418aee2` (2026-11-19);
- `.project` and `.pending_order` field expressions outside
  `settlements/construction*.rs` and their module children (including `#[path]`
  children), or the inline `settlements::construction` module and its children.
  This is a syntactic proxy for `SettlementRecord`, not receiver type resolution:
  unrelated same-named fields count too. Baseline: 31 on `b58021e61`, 2026-11-17.

D65 also has an independent inbound-domain-path ceiling in
`code_health/domain_boundary.rs`: named production references from outside
settlements and villager AI into the approved move set, with villager targets
classified at submodule granularity. The module walk, test exclusions, `use`
tree expansion and paths share the `syn` collector above. Qualified alias uses,
module-relative paths and explicit re-exports resolve after the module walk;
imports are also references. Method resolution and opaque macro-generated paths
are not a completeness claim.
The original baseline is 116; S3 lowered it to 113 before D63. On approved
batch B, D63's seven starter-chest host references make the rebased S1 baseline
123; S3 lowers it to **119**, including the offline grant projection cut.
The six remaining QW journal/dimension hooks move in D65 S2/S6 and lower the
ceiling then. This incoming-baseline adjustment was approved by Main.
The report identifies exact field/type allowances through C1 (journal images
and replay tail), C2 (`TickState.settlements`) and C3 (villager settlement and
soldier identities). These references remain counted, not blanket exemptions
for their files. The top-level SCC ceiling stays **186**, independently:
passing either metric cannot compensate for failing the other. An AST fixture
negative control adds one domain import while its largest SCC stays at one.

Macro bodies count too: a body that parses as comma-separated expressions
(`format!`, `matches!`, `assert!`) is visited like code; any other body
(`macro_rules!`, `vec![x; n]`, tracing fields) is read token by token only for
vanilla ids, panics and merge-block-write pushes. Hot-file node counts and
construction field expressions do not expand opaque macros.

A value below its ceiling prints `ceiling can drop to N`: lower the constant in
the same change. Raising a ceiling is an explicit edit of the constant with a
reason comment; there is no flag or override. The cycle's production lines are
a printed trend, never a failure: features inside cycle modules add lines.
The hot-file and construction-access ceilings start at their measured values,
without headroom. Extractions lower the affected constants in the same change;
removing a watched file measures zero. Main remeasures these baselines on the
integrated tree after the D7 Spain/A9 fixes merge, before the Wednesday review.

The report prints a per-module breakdown for each count and these known false
positives: vanilla ids in data and registry tables and in protocol, NBT and
Anvil schemas; `path()` on a non-identity type (matched by method name); a
panic on a truly impossible internal state; the push inside
`TickState::write_block` itself; unrelated `.project`/`.pending_order` fields.
Not seen: a module dependency through a type's methods without a module path,
ids without the `minecraft:` prefix, a `.path()` right operand or construction
field access inside an opaque macro, and construction field destructuring or
fields generated by macro expansion. For the cycle it prints the most connected
modules and decoupling hints: the modules that leave the cycle for the fewest
cut edges.

## Resource bounds

`run`/`client` scope the entire invocation and every child before work; limits
are totals, not per-process:

| Property | Default | Override |
|---|---|---|
| CPUQuota, CPUWeight | physical cores ×100%, weight 50 | `KRASNOKAMEN_HARNESS_CPU_QUOTA` (`off` disables) |
| MemoryHigh | off | `KRASNOKAMEN_HARNESS_MEMORY_HIGH` |
| MemoryMax | 12G for test/correctness, java-battle-functional/java/loader-live/scene-shot/battle-crash-recovery; 4G otherwise; named scenario inherits profile | `KRASNOKAMEN_HARNESS_MEMORY_MAX` |
| MemorySwapMax | 1G | `KRASNOKAMEN_HARNESS_MEMORY_SWAP_MAX` |

`MEMORY_HEAVY_PROFILES` in `__main__.py` owns the list: server+Java+Xvfb exceeded
4G on 2/2 `receipts/JavaGateBreaker` runs. Admission refuses if MemAvailable is
below MemoryMax+1 GiB; refusal is not acceptance. Without a user systemd scope,
warn and continue unbounded, still recording the profile result.
Owner-host invocations additionally use Main's [mem-run](development/heavy-commands-and-memory.md);
its `KRASNOKAMEN_HARNESS_CPU_SCOPE=1` prevents a second escaping cgroup.

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

Do not infer OOM or owner cancellation from a terminating signal alone. Preserve
the observed phase/status and match child, scope and cgroup evidence under
[OOM diagnosis](development/heavy-commands-and-memory.md#oom-and-killed-commands);
unknown cause is not an interrupted-by-owner claim.

Preflight selects prerequisites from the profile's declaration in
`tools/harness/profiles.py`, not from its scope label. Native profiles that
launch a client read its schemas with `krasnokamen-client mcp-schema`; the command
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
  `KRASNOKAMEN_REAL_CLIENT_SECOND_AGENT_SECRET` or bridge URL/port pair; the backend
  errors name the exact variable.
- `civilization-seed-qa` awaits each console sprint's exact `tick sprint
  finished` event under the remaining scenario deadline, not the 120-second MCP
  request cap. Slow simulation is recorded in sprint TPS and `MC_CORE_PHASE_LOG`
  phase timings; neither elapsed time nor a missing completion is success.
  Its isolated fixture uses simulation distance 6 while keeping view distance 4:
  the village's mine must remain active beyond the presentation window. Other
  autonomy profiles and the global playable configuration stay unchanged.
  House milestone capture uses the received entrance door, approaches and uses
  it ordinarily when closed, and awaits both open halves before the existing
  exact-bed capture. It returns to the original observer afterward. A found
  path, bed row or exterior-wall screenshot does not replace the exact bed
  raycast and rendered capture; saved-world route proofs are not fresh-seed
  civilization acceptance.
- Full workflows for graphical failures: [real-client gates](development/real-client-gates.md)
  and [live-gate triage](development/live-gate-triage.md).

## Java and Loader client launch (`client`)

The default (platformless) development client lives under the sibling
`../solaris-loader/fabric-agent/`; despite that name it is a **NeoForge** mod
(entrypoint `dev.krasnokamen.agent.neoforge.KrasnokamenClientAgentMod`, fixed launch task
`:fabric-agent:runClientMcp`). The three Loader adapters (`loader-fabric`,
`loader-neoforge`, `loader-forge`) register the same Configuration-state
manifest/ack payloads through their native networking APIs and launch as
`:loader-<platform>:runClientMcp`. The Minecraft version of this client is the
Loader's `minecraftVersion` in its `gradle.properties`.

`client` (launch and `--check` alike) needs these per-run values. The token is a
random local bearer secret you generate, not an account credential:

- `KRASNOKAMEN_CLIENT_MCP_TOKEN`: random bearer token, no default; without it
  `client` exits 2 before Gradle starts.
- `KRASNOKAMEN_CLIENT_MCP_PORT`: free IPv4 loopback port, default `39095`.
- `KRASNOKAMEN_CLIENT_MCP_GAME_DIR`: isolated game dir; default
  `../solaris-loader/fabric-agent/run-mcp`, or
  `.analysis/minecraft-loader-mcp/<platform>` when `--platform` is given.
- `KRASNOKAMEN_CLIENT_MCP_USERNAME`: `1..16` ASCII letters/digits/underscores;
  default `KrasnoMcp` (platformless) or `KrasnoLoader` (Loader platforms).

```sh
KRASNOKAMEN_CLIENT_MCP_TOKEN=local-check-token python3 -m tools.harness client --check
export KRASNOKAMEN_CLIENT_MCP_TOKEN="$(openssl rand -hex 32)"
export KRASNOKAMEN_CLIENT_MCP_PORT=39095
export KRASNOKAMEN_CLIENT_MCP_GAME_DIR=.analysis/minecraft-mcp-primary
export KRASNOKAMEN_CLIENT_MCP_USERNAME=KrasnoMcpA
python3 -m tools.harness client
python3 -m tools.harness client --platform neoforge   # Loader client on another port/dir/user
```

Normal launch refuses to start when the MCP port is already in use. An HTTP 401
means the caller reached an endpoint with a different bearer token: verify the
port and process instead of retrying it as a transient failure. `--check`
validates Java 25, configuration, MCP transport tests and the Gradle adapter
without launching Minecraft. Use a second port, token, game directory and
username for multiplayer gates. The endpoint is `http://127.0.0.1:<port>/mcp`.

MCP smoke against an already-running client (`KRASNOKAMEN_CLIENT_MCP_TOKEN` still
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
| `KRASNOKAMEN_NEXTEST_PROFILE` | Nextest profile (`default` local full suite; CI sets `ci` short run) |
| `KRASNOKAMEN_REAL_CLIENT_MANIFEST` | Regression pack manifest (`docs/real-client-regression/manifests/m94-regression-pack.json`; `playable` presets its loop manifest, `replay` its seed-81 manifest) |
| `KRASNOKAMEN_REAL_CLIENT_RUN_ROOT` | Local artifact root (preset under the run artifact `regression/` dir) |
| `KRASNOKAMEN_REAL_CLIENT_SERVER_CONFIG` | Server config (`example.toml`; `playable` presets `playable.toml`) |
| `KRASNOKAMEN_REAL_CLIENT_FRESH_WORLD` | `1` copies the config into the run dir with a fresh per-run world (`playable`/`replay` preset `1`) |
| `KRASNOKAMEN_REAL_CLIENT_SERVER_SEED` | Optional signed 64-bit decimal overriding `data.seed` |
| `KRASNOKAMEN_REAL_CLIENT_SERVER_ADDR` | Server address for the in-client driver (`127.0.0.1:25565`) |
| `KRASNOKAMEN_REAL_CLIENT_TIMEOUT_SECONDS` | Gradle runClient adapter timeout (180; `playable` 1500, `replay` 180). `--timeout-seconds` overrides it for `playable`/`replay`. |
| `KRASNOKAMEN_REAL_CLIENT_SERVER_READY_TIMEOUT_SECONDS` | Server-readiness wait (120) |
| `KRASNOKAMEN_REAL_CLIENT_AGENT_BRIDGE_URL` / `AGENT_SECRET` / `AGENT_PORT` / `AGENT_DRIVER` / `AGENT_SCENARIO` | Loopback JSON bridge URL, per-run bridge secret (required for agent-driver mode), bridge port, internal driver path, scenario id. Second-client mode additionally needs `KRASNOKAMEN_REAL_CLIENT_SECOND_AGENT_SECRET` plus bridge URL or port. |
| `M79_ORACLE_REPORT_DIR` / `M79_ORACLE_CHECK_ROOT` | Oracle report/check roots (harness presets them under the run artifact) |
| `KRASNOKAMEN_ENTITY_BENCH_*` | Scale workload knobs (preset under artifact `bench/`) |
| `KRASNOKAMEN_VALIDATION_RUN_DIR` | Canonical run dir injected into every backend env; used by seed-review evidence |

Native-client variables (`KRASNOKAMEN_CLIENT_*`) are in the
[real-client gates prerequisites](development/real-client-gates.md#prerequisites).
Normal operator launches never set `KRASNOKAMEN_HARNESS_LOG_STDOUT`,
`KRASNOKAMEN_HARNESS_CPU_SCOPE` or the memory-cap variables beyond the recipes above;
the canonical `run` entrypoint sets `KRASNOKAMEN_HARNESS_LOG_STDOUT=1` for its
subprocesses so readiness events and per-run captured logs stay available
(`RUST_LOG` controls `logs/debug.log`, not the captured threshold).

## Decoder fuzzing (`fuzz`)

The independent `fuzz/` workspace is outside L2/code-health. Its six decoder
targets, CI budgets/toolchain, corpus retention, reproduction and owner-host
commands have one home: [decoder fuzzing](../fuzz/README.md).
CI's `ci.yml` `fuzz` job is canonical: six legs, 60 s per target on push/PR;
`fuzz-nightly.yml` runs those same legs for 900 s at `17 3 * * *` or the dispatched
`seconds` value. Keep both workflows' flags in step. `release-build` depends on
fuzz. Local dev-profile smoke is not CI release-window coverage.
Minimize findings, add owning-crate regressions and file ledger rows; never
bundle parser fixes into fuzz-tooling changes.

## Code and client tools

The Minecraft client MCP (embedded in the Loader dev client and the native
client) is an optional helper; normal work uses the repository rules, the
harness and the agent's own tools. Owner-workstation extras are never required.
Do not route provider traffic through proxies or enable MCPs the owner did not
ask for. The CodeGraph MCP server is disabled on the owner host; the CLI remains
installed for occasional questions. Build its ignored `.codegraph/` index with
`codegraph init` when needed.

### Agent runtime routing and diagnostics

- Account routing and model choice are separate. Use supported policy for
  owner-designated work accounts rather than copying credentials or restarting
  Main. Priority/reserve/reclaim and explicit pins have different semantics;
  follow the running version's
  [upstream account-policy release notes](https://github.com/can1357/oh-my-pi/blob/main/packages/ai/CHANGELOG.md).
  [Live settings reload](https://github.com/can1357/oh-my-pi/blob/main/docs/settings.md#live-file-changes)
  applies account policy to subsequent credential selection, not a restart or
  proof that a healthy session's model/account changed. Effective readback,
  fresh-selection inspection and one authorized bounded live request prove
  different things; none proves gameplay. Do not log out accounts, consume saved
  resets or copy rotating OAuth refresh tokens between machines to force routing.
- Record the **running** agent build/runtime, process/cgroup scope and timing of
  memory observations, separately from an updated installed binary. RSS minus
  JS heap is not evidence of live native allocations: accounting scopes,
  allocator-retained pages and report-generation peaks differ
  ([memory API limits](https://nodejs.org/api/process.html#processmemoryusage)).
  Empty allocator-verbose output proves nothing unless that running runtime is
  known to emit it; allocator/root-cause explanations remain hypotheses otherwise.
- Diagnostic bundles can contain credentials and transcripts. Keep raw archives
  private; share only explicitly authorized, inspected/redacted evidence.
  Public lessons describe the measurement and its limits, never account identity,
  balances, auth/config backups or session dumps.


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
  root workspace, so L2 never builds it; the [decoder fuzzing](#decoder-fuzzing-fuzz)
  jobs run it in CI, and every finding becomes a regression test in the owning crate.
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
