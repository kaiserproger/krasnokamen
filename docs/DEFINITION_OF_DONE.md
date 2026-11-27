# Definition of done

The process contract for calling work done in Krasnokamen. It does not slow
draft implementation; it stops draft work from being reported as release
quality. Validation levels and receipts: [agent tooling](AGENT_TOOLING.md);
bug closure: [ledger contract](bugs/README.md); breaker acceptance:
[QA protocol](QA_AGENT_PROTOCOL.md).

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
Other findings stay in the visible work queue under the
[bug queue and progress rule](development/agent-fleet.md#bug-queue-and-progress-reports);
they are not automatically release blockers. The owner may explicitly add
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

## Quality labels

Every plan, closeout and final response carries one label; when in doubt,
`draft`.

- `draft`: the implementation exists and the required Cargo baseline is green
  or explicitly degraded; vanilla parity, client behaviour and performance are
  not fully proven. Drafts may land partial systems and scaffolding, must list
  their stabilization debt, and may not move tags, claim replacement readiness
  or erase oracle/client gaps from docs.
- `stabilization`: the slice is being hardened: known gaps are tracked,
  oracle/client/performance evidence is being added, regressions are fixed
  before new breadth. After several drafts, schedule one.
- `release-ready`: the [stabilization criteria](#stabilization-criteria) hold
  for the scoped mechanics. Never for broad systems with only unit or focused
  harness coverage.

## Preflight

Before milestone code, a closeout, or a claim that a client gate is ready:

```sh
pwd; git rev-parse --show-toplevel; git status --short --branch
rustc --version; cargo --version; cargo fmt --version; java --version
test -d data/vanilla/reports || test -d data/vanilla   # licensed content cache
```

Also, when relevant: port 25565 is free before starting `krasnokamen`, and
ignored local files (`.analysis/`, `data/vanilla/`, `opencode.json`) stay
unstaged. The cache is built per [operating guide](OPERATING.md#vanilla-content-cache).
Oracle and graphical gates need owner-host artifacts that never ship
(`javap -version`, `.analysis/server.jar` = licensed vanilla server bundle,
`.analysis/test-world` from `tools/generate-test-world.sh`, the
[client prerequisites](development/real-client-gates.md)); without them the gate
is `blocked` or `degraded`, never a pass. Report the preflight as `full`,
`degraded` (naming the missing item and the validation it weakens) or `blocked`.

## Evidence matrix

Every closeout, drafts included:

| Gate | Required wording |
|---|---|
| Cargo baseline | `correctness` (or its `regressions`, `fmt`, `code-health`, `clippy`, `test` stages) after the last code change, or why not |
| Focused tests | which behaviour they prove and why they are not pass-by-construction |
| Feature acceptance | the acceptance test's author (not the implementer) and source text; the live run's receipt ([feature acceptance](#feature-acceptance)) |
| Vanilla oracle | `wire-probe`, `javap`, decompiled source, vanilla server capture, or `not run` |
| Client gate | real-client harness profile or PrismLauncher status: owner-run, agent-run, prepared only or not run, with the client release (26.3, 26.2 or 26.1.2) |
| Performance | latency/TPS/lock-contention metrics when hot paths changed |
| Concurrency | no global-lock regression or blocking tick/network path, when relevant |
| Data/protocol facts | packet IDs/layouts, registry IDs, cache version/schema and stale/missing-data behaviour, cited or `not run` |
| Persistence | save/restart/crash-window evidence, formats touched, backup/recovery limits |
| Dependencies | new or changed dependencies, the `Cargo.lock` reason, licence/security audit, or `not run` |
| Known gaps | each deferred behaviour, as draft debt or accepted non-goal |

Never write "all green" unless every required row is green or the exceptions
stand next to the claim. Oracle or focused tests that skip for missing local
artifacts record degraded coverage; a silent skip counts only for draft unit
scaffolding. A failed owner or manual scenario stays `blocked` until the same or
a stricter real-client scenario passes; green Cargo or harness gates never
downgrade a failed client observation.

## Vanilla parity target

Krasnokamen is not a bit-perfect clone, but the release target is hard:

- At least 80 % of scoped vanilla overworld-survival mechanics that a normal
  vanilla Java client can observe. The canonical release is 26.3 (protocol 777);
  older clients are served through the translation chain, so say which release a
  claim was observed on.
- The remaining 20 % is documented as non-goal, deferred or knowingly divergent.
- A mechanic counts only when its normal path has tests and at least one
  vanilla- or client-visible evidence source.
- Packet IDs, field layouts, registry/data facts and timing claims cite local
  vanilla evidence, never memory or wiki guesses.

The 80 % is for planning, not for watering down single claims: "sign editing
parity" needs evidence about sign editing. Bug-fix tests assert vanilla or
owner-accepted behaviour, not the current implementation
([bug regression contract](bugs/README.md)).

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

## Stabilization criteria

A stabilization milestone, and any `release-ready` claim, is done only when:

- the Cargo baseline passes after the final change;
- new or changed mechanics pass their independent acceptance test and live run;
- vanilla/Krasnokamen divergence is measured or explicitly accepted;
- real-client automation or owner PrismLauncher play covers the client-visible
  scenario, otherwise the closeout stays `draft`;
- hot-path changes have measured performance budgets;
- global locks, tick blocking, async blocking and worker pools are reviewed when
  worldgen, storage, networking, entities, physics, lighting or chunk streaming
  changed.

**Client evidence** comes from the harness, which runs the actual client under an
MCP-controlled agent and ties join/rejoin, movement, block edits, inventory and
containers, combat, death/respawn, save/restart and screenshots or structured
observations to the server run ([profiles](AGENT_TOOLING.md#profiles-and-scenarios),
[procedure and traps](development/real-client-gates.md)). Manual PrismLauncher
play is owner-run. Headless mocks and protocol bots are harness evidence, not
client evidence.

**Performance and multithreading** work is done only with both speed and
correctness: baseline and new measurement of the relevant metric (chunk latency,
TPS, tick time, lock wait, queue depth, memory); a reproducible workload; no
client-visible regression under load; for concurrent code, documented ownership
boundaries and how blocking disk/network/worldgen work stays off the tick path.
Faster with weaker correctness remains `draft`.
