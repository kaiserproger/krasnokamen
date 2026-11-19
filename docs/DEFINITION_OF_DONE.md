# Definition Of Done

This document is the hard process contract for Krasnokamen agents. It does
not slow down draft implementation; it prevents draft work from being
reported as release quality.

## Finite announcement acceptance

Owner authorization, 2026-11-15: stabilize the already approved **21 November
2026** announcement, preserving A1–A12, A2b/A2c and the **2:45 teaser**. This
does not authorize a vanilla-only replacement or removal of differentiators.
The scoped **80% vanilla parity** rule below remains required.

The existing operational plan is `<work>/receipts/Stage3/ANNOUNCE-PLAN.md`;
current ownership, ranked dependencies and evidence are in
`<work>/receipts/Stage3/PLAN.md`. Receipt paths in this table are relative to
`<work>/receipts/`. A named owner may be parked: ownership is not permission to
execute outside the [current operating mode](development/agent-fleet.md#announcement-stabilization-mode).
Missing, old-candidate, incomplete or merely prepared evidence stays **blocked**.
The freeze remains 19 November evening; 20 November is final QA/review; the
target date does not turn a failed criterion into a pass.

| Row | Concrete end-to-end criterion | Existing gate / required repeats | Current owner / receipts |
|---|---|---|---|
| A1 | Live Horizon battle: `battle.fighting` peak and plateau, **20 TPS**, **≥30 FPS in both windows**, **≥10 server-confirmed PovRed hits per battle minute**, normal damage and following CamSpec; montage from that accepted freeze-pin take. | Existing live-take acceptance, server scan, `povred_hits.py`, per-10-second client stats and owner cut approval. Preserve exact read-only-extension disclosure and committed diff/SHA; no rendering/network/input modification. | HzBattlePrep + laptop; server dependencies Main / SpectateFix / EntityMoveDelta. `HzBattlePrep/`, `SpectateFix/`, `EntityMoveDelta/`. |
| A2 | Horizon's bots on ours, vanilla and Horizon in both parallel-tick modes: **1/10/25/50/100 bots × mixed/explore/fight**, identical recorded configuration, commands and raw results. No superiority claim for a lost comparison. | Existing Horizon-method matrix; variance; independent repeat of **2 cells**; **rerun every matrix cell on the freeze pin**, not only headline cells. | HzServerBots; `HzServerBots/`. |
| A2b | Same `mixed ×100` and `horizon-100-roam` at **1/2/4/6 physical server cores**, whole SMT pairs, bots on fixed separate cores; ours, vanilla, Horizon `parallel_tick=false/true`; TPS and MSPT p50/p95. | Existing scaling windows: **every point repeated**, second agent reruns **2 points**. An advantage enters S4 only if measured. No new Paper/Pumpkin/FerrumC campaign. | HzServerBots + PerfRoam; `HzServerBots/`, `PerfRoam/`. |
| A2c | Separately labelled internet/remote-bot scenario: laptop bots, **8 physical server cores / 16 logical CPUs**, `mixed ×100`; all bots log in, walk/explore bots move; the normal **20% idle** cohort need not move. | Existing W2/W9 owner and order: ours, vanilla, Horizon false/true, **r1 and r2**; raw route/RTT/configuration evidence. Never merge this point into the local-bot curve. | Laptop HzRemoteBots; `HzRemoteBots/`, `HzServerBots/SERVERS.md`. |
| A3 | **100+ joined players and 100k+ freely roaming mobs at 20 TPS**, Horizon's scenario, not pens or spawned-but-inactive counts. | Existing `horizon-100-roam`, quiet freeze-pin measurement and **second run**; load-correctness breaker checks no lost mobs/items. Preserve the scheduled comparison/scaling points. | PerfRoam; `PerfRoam/`. |
| A4 | Unmodified Horizon client joins and completes its existing server-capable survival/UI/event/pick/use-screen/consume/suggestions paths without disconnect or broken ordinary interaction; FPS comparison uses actual drawn frames. | Existing Horizon e2e scenarios **×3**, explore **25 bots ×3**, original failed suggestions/copper/fill cases on corrected pin; laptop 1:1 FPS comparison. Headless functional evidence is not graphical/FPS acceptance. | HzClientQA, queued HzCompatA4 fixes + laptop; `HzClientQA/`, `HzCompatA4/`. |
| A5 | Battle with `/give` and kits in flight survives SIGKILL/restart with the required entity/item/inventory census and legitimate lifecycle removals accounted for. | Existing crash campaign: **≥5 valid cuts at each of its four moments (20 total)**, including before/after checkpoint, during append and after publication; ordinary pruning, retained original WAL/crash copies. Incomplete cuts are not passes. | CrashGive; `CrashGive/`. |
| A6 | Measured public-docs-only zero-to-running-server-with-plugins path for ours and Horizon: steps, wall time, downloads and failures; merge concrete small fixes it discovers. | Existing fresh-user replay; **second independent replay of our path** from docs alone. Report losses honestly. | ConvenienceSdk; `ConvenienceSdk/`. |
| A7 | Measured zero-to-first-working-plugin path for ours and `cargo-horizon-mod`: steps, lines and time; the existing proposed concrete simplification is resolved by the owner and merged if selected. | Existing fresh-checkout plugin replay, not a generated scaffold or mocked command response. No speculative SDK redesign. | ConvenienceSdk + owner API-cut decision; `ConvenienceSdk/`. |
| A8 | Real **26.1.2 and 26.3 clients simultaneously** in one freeze-pin world: chat, see each other, transfer an item. Retain support for the full 775–777 window. | Existing mixed-version real-client gate and freeze-pin recheck; protocol bots alone do not accept the visible interaction. | Main; `Stage3/` mixed-version gate receipts. |
| A9 | Genuine paid Spanish construction, parent growth and daughter founding inheriting the committed style; same owner UUID is the approved teaser faction. Real timelapse, day/population/buildings/TPS overlay and chronicle from that run, not a single-town substitute. | Existing S1 trial in fresh recorded-seed worlds **≥2 runs**, freeze-pin footage; all **5 approved founding amendments**, old rows read `None`, no old-binary downgrade after new-field writes, independent persistence review, required negative/crash controls. | SpainCulture, queued; `SpainCulture/`. |
| A10 | Full already-selected adapted ATM data/worldgen target (**127 MVS Overworld + 3 Nether definitions**), genuinely generated and shown by a real client in a fresh world, surviving the required restart. Preserve structure sets/pools/jigsaw/processors/placement/Moog support and block/item requirements, not fixture-only admission. | Existing unchanged selected-pack count/restart case (seed **20261115**, origin chunk **(-16,-16)**, same seven-file crystal fixture), plus full selected-pack and actual-template/Nether checks; the partial fixture is not the full acceptance target. Keep all original repeats/controls. Caption **“adapted data from N ATM10 mods, EXPERIMENTAL”**; N stays **0** until actual proof; record licences, no jars/data redistribution or whole-modpack compatibility claim. | AtmNether integration; queued AtmCarry counted proof. `AtmNether/`, `AtmOverlay/`, `AtmCarry/`, `AtmPilot/`. |
| A11 | Persisted native battle regiments/formations/orders/targets/holds/phase/live-player roles recover and battle continues automatically after SIGKILL, with no reset/setup command. | Existing native/WAL regression with **fix-reverted negative control**, independent persistence review, actual Horizon-client SIGKILL continuation **≥3 repeats** on the corrected candidate; ADR and operating guide updated. | BattleResume; `BattleResume/`, `SpectateFix/`. |
| A12 | CONTRIBUTING and outside-AI guidance work without owner-host internals: build, L0/L1/L2, PR, licence/Mojang-byte rules. **24 existing first-entry issues** (within approved 20–30), labelled and containing context/files/acceptance/verification. | Fresh clean clone/user follows docs to **green L1 + sample draft PR**; breaker reads **all 24 drafts**, references refreshed to freeze pin. Main publication authorized **20 November only after those checks**, not now. | ContribReady + Main; `ContribReady/`, `ContribReady/issues/`, `Stage3/`. |

**Queue admission.** Report and file every noticed defect. Only a reproducible
safety failure (crash, data loss, duplication or access bypass), a broken
ordinary advertised path, or a concrete dependency of a frozen row
automatically blocks this announcement. Name the row and causal dependency.
Everything else stays visible backlog, not an automatic implementation slice,
QA campaign, audit or new release requirement. The owner may explicitly add
scope. Preserve negative evidence and deferred debt; never erase a failure by
renaming it or narrowing its assertion.

**Closure and stop.** All frozen rows need their required same-candidate
evidence; the blocking set must be empty, including no known release-blocking
crash/data loss/dupe/access bypass or broken ordinary advertised path. Require
final integrated `correctness`, scoped real-client proof, independent
[breaker acceptance](QA_AGENT_PROTOCOL.md), **two release reviewers**, and
honest documented limits under the existing
[release procedure](development/release-and-tags.md). These reviews count
inside the two general child slots, not an extra pool. Reuse clean pins only for the
same tree and inputs; rerun only evidence invalidated by a relevant change.

Then set `release-ready`, stop/park automated development and send **one owner
decision packet** for tag/announcement. Do not reopen broad audits, refactors,
optional mechanics, benchmarks or the historical ledger automatically.
Publication still requires owner authorization. If a criterion cannot be met,
report its exact missing proof/blocker and the owner decision required; do not
silently cut scope, reset the date or retry indefinitely. Apply the
Constitution's same-premise check before a third fix after two failed fixes.


## Quality Labels

Every plan, closeout, and final response must use one of these labels:

- `draft`: implementation exists and the required cargo baseline is
  green or explicitly degraded. Vanilla parity, manual client behavior,
  and performance are not fully proven.
- `stabilization`: the slice is being hardened. Known gaps are tracked,
  oracle/client/performance evidence is being added, and regressions are
  fixed before new breadth work.
- `release-ready`: the hard DoD below is satisfied for the scoped
  mechanics. This label is not allowed for broad systems with only unit
  or focused harness coverage.

If in doubt, label the work `draft`.

## Autonomous Preflight

Run this before milestone code, before a closeout, and before any claim
that a manual/client gate is ready. The baseline applies to any checkout:

```sh
pwd
git rev-parse --show-toplevel
git status --short --branch
rustc --version
cargo --version
cargo fmt --version
java --version
test -d data/vanilla/reports || test -d data/vanilla
```

The last line checks the git-ignored licensed content cache
([how to build it](OPERATING.md#vanilla-content-cache)). Also check, when
relevant:

- Port `25565` is free before starting `krasnokamen`.
- The current branch matches the milestone branch named in the plan.
- Local-only files such as `.analysis/`, `data/vanilla/` and `opencode.json`
  are git-ignored and will not be staged.

### Owner-host-only preflight

Oracle and graphical gates additionally need artifacts that exist only on the
owner host (git-ignored, never shipped in the repository). A
contributor without them reports the affected gate as `blocked` or `degraded`,
never as a pass:

```sh
javap -version
test -f .analysis/server.jar   # licensed vanilla server bundle used by oracle checks
test -d .analysis/test-world   # created by tools/generate-test-world.sh from that jar
```

`.analysis/test-world` may be absent when the tests that use it are expected to
skip. A real-client gate also needs the client prerequisites listed in
[real-client gates](development/real-client-gates.md).

Report the preflight as `full`, `degraded`, or `blocked`. A degraded
preflight must name the missing item and the validation it weakens.

## Evidence Matrix

Closeouts must include this matrix, even for drafts:

| Gate | Required wording |
|---|---|
| Cargo baseline | Exact `regressions`, `fmt`, `xtask code-health`, `clippy` and `test` commands (or the `correctness` harness profile, which runs all of them) run after the last code change, or why not. |
| Focused tests | Which behavior they prove, and why they are not pass-by-construction. |
| Feature acceptance | The acceptance test's author (an agent other than the implementer) and its source text; the live run's receipt (server, client or bot, scenario). See [feature acceptance](#feature-acceptance). |
| Vanilla oracle | `wire-probe`, `javap`, decompiled source, vanilla server capture, or explicit `not run`. |
| Client/manual gate | Real-client harness profile or owner-run PrismLauncher status: owner-run, agent-run, prepared only, or not run; name the client release (26.3 or legacy 26.1.2). |
| Performance | Metrics for latency/TPS/lock contention when the slice touches hot paths. |
| Concurrency | Evidence for no global lock regression or blocking tick/network path when relevant. |
| Data/protocol facts | Packet IDs/layouts, registry IDs, sidecar version/schema, and stale/missing-data behavior cited or marked not run. |
| Persistence/storage | Save/restart/crash-window evidence, storage formats touched, backup/recovery limits, and unknown-NBT/sidecar preservation status. |
| Dependencies | New/changed dependencies, `Cargo.lock` reason, license/security audit status, or explicit not run. |
| Known gaps | Exact list of deferred behavior and whether it is draft debt or accepted non-goal. |

Do not write "all green" unless every required row is green or the
exceptions are listed next to the claim.

Focused/oracle tests that skip because local artifacts are absent must
emit or record degraded coverage. Silent skip is allowed only for draft
unit scaffolding and cannot count toward stabilization or release
evidence.

A known failed owner/manual scenario is `blocked` until the same or a
stricter real-client scenario is rerun and recorded. Green cargo or
harness gates cannot downgrade a failed real-client observation.

## Vanilla Parity Target

Krasnokamen is not a bit-perfect clone, but the release target is hard:

- At least 80% coverage of scoped vanilla overworld-survival mechanics
  that a normal vanilla Java client can observe. The server's canonical
  release is 26.3 (protocol 777); 26.1.2 clients are served through the
  retained legacy profile, so state which release a claim was observed on.
- The remaining 20% must be documented as non-goal, deferred, or
  knowingly divergent Krasnokamen semantics.
- A mechanic counts toward the 80% only when its normal path is covered
  by tests and at least one vanilla/client-visible evidence source.
- Packet IDs, field layouts, registry/data facts, and gameplay timing
  claims must cite local vanilla evidence, not memory or wiki guesses.

Use the 80% target for release/stabilization planning, not as an excuse
to water down individual claims. If a closeout says "sign editing
parity", sign editing itself needs evidence; it cannot borrow coverage
from unrelated mechanics.

### Bug fixes

Follow the [bug regression contract](bugs/README.md). Tests must assert
vanilla or owner-accepted behavior, not merely the current implementation.

## Feature acceptance

Owner D60, 2026-11-19 (`receipts/Stage3/OWNER-DECISIONS-20261117-1000.md`):

- **Independent acceptance test.** A feature's acceptance test is written by a
  different agent than its implementer, from the owner decision text or
  minecraft.wiki, without seeing the implementation. That agent never edits the
  implementation. A failing acceptance test goes back to the implementer; only
  its author changes it, from the same source.
- **One live run per feature.** A feature is accepted by one live run: a real
  server with a real client or bot, playing the scenario from the owner
  decision. It replaces piles of unit tests. Unit tests stay for regressions of
  real bugs and for checks against an oracle (vanilla, the old implementation,
  a real run).

## Draft Implementation Rules

Fast breadth-first work is allowed, but must stay honest:

- Draft milestones may land partial systems, scaffolding, and local
  deterministic semantics.
- Draft closeouts must say `draft` and list stabilization debt.
- Draft work may not move `main` tags, claim replacement readiness, or
  erase oracle/manual gaps from docs.
- After several draft milestones, schedule a stabilization milestone
  dedicated to regressions, manual gates, vanilla parity, performance,
  concurrency, and test-quality cleanup.

## Stabilization DoD

A stabilization milestone is done only when:

- The cargo baseline passes after the final change.
- New or changed mechanics pass their independent acceptance test and one live
  run ([feature acceptance](#feature-acceptance)).
- Vanilla/Krasnokamen divergence is measured or explicitly accepted.
- PrismLauncher or real-client automation covers the client-visible
  scenario, or the closeout remains `draft`.
- Performance budgets are measured for hot-path changes.
- Global lock, tick blocking, async blocking, and worker-pool behavior
  are reviewed when the slice touches worldgen, storage, networking,
  entities, physics, lighting, or chunk streaming.

## Real-Client Evidence

Real-client gates are driven by the harness, which runs the actual client under
an MCP-controlled automation agent and records join/rejoin, movement, block
edits, inventory and container actions, combat, death/respawn, save/restart and
screenshots or structured observations tied to the server run. Profiles and
receipts are in [AGENT_TOOLING.md](AGENT_TOOLING.md#profiles-and-scenarios)
(`python3 -m tools.harness list` and `explain` are authoritative) and the
procedure and traps in
[real-client gates](development/real-client-gates.md). Manual PrismLauncher
play is owner-run. Headless mocks and protocol-only bots are harness evidence,
not client/manual evidence.

## Performance And Multithreading DoD

Optimizations and multithreading work are done only when they prove both
speed and correctness:

- State the baseline and new measurements: chunk latency, TPS, tick time,
  lock wait, queue depth, memory use, or other relevant metric.
- Show the workload used to measure it and keep it reproducible.
- Prove no client-visible regression under load.
- For concurrent code, document ownership boundaries and how blocking
  disk/network/worldgen work stays off the tick path.
- A faster result with weaker correctness remains `draft`.
