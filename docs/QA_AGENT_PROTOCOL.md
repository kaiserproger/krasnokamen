# Krasnokamen break-it QA protocol

A gate that walks the author's happy path proves only that path. In the
2026-10-31 RC2/RC3 work, broader player-path checks found what repeated happy
paths had missed: fresh-install singleplayer lacked a file the new server
required in its pinned content bundle, the 26.3 Loader adapters crashed on stale
mixin targets, and a NeoForge Loader client replied on an undeclared channel.
This protocol makes such attacks part of acceptance; a green receipt remains
evidence only for the exact path and artifact it exercised.

The reviewer is independent of the implementer; selection follows
[AGENTS.md](../AGENTS.md), and unavailable required coverage stays blocked unless
the owner changes the requirement. Follow the
[bug regression contract](bugs/README.md). Every agent-launched Java/Minecraft
client uses `ALSOFT_DRIVERS=null` (or `wave` writing to the games disk when audio
must be recorded).

## Roles

- **Builder** implements the change and its focused regressions and gates.
- **Breaker** is a separate agent whose only goal is a reproducible failure. It
  gets the change, its claims (commit message, release notes, ADR text) and this
  protocol, not the builder's test plan, and never edits production code.
- **Main** routes findings to the builder and decides on release. Player-visible
  work is not accepted (no `fixed` ledger row, no release-ready claim) and a
  release candidate is not tagged until a breaker has reported on it. Pushing
  unaccepted work to `main` is allowed ([owner contract](development/owner-contract.md)).

The oracle is a normal player's or admin's expectation and vanilla 26.3 evidence
(minecraft.wiki, `javap`); for Krasnokamen features, the owner's accepted
behaviour. Implementation and green tests are not the oracle. "Found nothing"
is valid only with the attacks actually tried.

## Attack matrix

For every change, run at least one concrete attack per class the change can
plausibly affect, and write down why each skipped class does not apply.

| Class | Attack | Example miss it catches |
|---|---|---|
| Zero state | fresh `HOME`/data root and empty caches: the frozen staging package before the tag, downloaded release assets after, never the dev tree | content bundle missing `en_us.json` |
| Previous release | start on the previous release's world, content cache, config and client data | RC1 cache and worldgen revision refused |
| Every advertised variant | every platform, loader, OS and client the notes promise: Fabric/NeoForge/Forge, native/vanilla Java, Linux/Windows/macOS, 26.3 and older supported releases | Loader mixins, NeoForge channel declaration |
| Every branch | each outcome the change introduces or alters (started/queued/refused, funded/unfunded, owner/non-owner) at least once on a real client | queue gate only saw an immediate `Started` |
| Interruption | restart, `kill -9`, disconnect or relog mid-action; resume after | pending order lost on restart |
| Resources and limits | empty and full inventory/storage, zero/max values, max level, radius edge, chunk/simulation boundary | builder idle in an inactive chunk |
| Actors | non-owner, operator, a second player at once, a player in another dimension | owner notification across dimensions |
| Hostile input | malformed, out-of-order, repeated, oversized or unicode input; commands with wrong arguments | — |
| Time and variance | another seed, a second run, a slower host (`taskset` 2 CPUs), a long session | autonomy gate passed once, failed next |
| Claims | every sentence in the commit message, release notes or README backed by a receipt, otherwise reworded | "vanilla-exact", "caches keep working" |

## Gate hygiene

A gate counts as evidence only when all of these hold:

1. **Negative control.** A new or changed behavioural gate fails without the fix
   or feature, at the expected assertion; record that receipt. Missing fixtures,
   compile errors and client-startup failures are not a negative control for an
   unrelated gameplay assertion.
2. **No pre-existing pass.** Each assertion observes something the action
   produced: a fresh event, a new identity, a changed counter. State that could
   exist before the action (an earlier project of the same type, an old chat
   line) does not count.
3. **Branch coverage.** The real client exercises every player-visible outcome
   the change introduces or alters; other outcomes keep protocol/native checks.
   Record unexercised branches; unit tests do not substitute for claimed
   real-client coverage.
4. **Variance.** Timing-, seed- or concurrency-dependent gates report a pass rate
   over at least three runs or seeds. One pass is not reliability; one failure
   is a finding until explained.
5. **Artifact identity.** Before the tag, record hashes of a frozen package built
   through the shipping packaging path; after publication, download the real
   assets and check hashes, contents and a clean install. A local binary beside
   dev data or a green CI build does not prove shipped installation.

### Crash census evidence

- Preserve a stopped snapshot, the original WAL/checkpoint/census hashes and the
  original verdict before reopening. Opening a database may repair an unclean
  image: inspect or repair a hash-verified copy, never the primary evidence.
- Bound the observation with the
  [persistence authority and clocks](architecture/persistence.md#recovery-evidence-and-clocks),
  not a network reply or pump statistic. Compare dimension-scoped UUIDs, entity
  IDs and exact entity/player holder/slot/item images; reject identity reuse,
  history gaps or an unproved game-time/WAL mapping.
- G3 covers entity and player holders only, not block-entity containers
  (chests, furnaces, hoppers, shulkers). Its expected custody image precedes
  `restore_entity`/live hydration and `encode_checkpoint` but shares the replay
  decoder and ordered WAL fold with recovery, so it does not check those stages;
  a second export through the same restore path is not an independent oracle.
  Exercise a restore-only loss in a non-soldier holder on a separate pinned tree:
  the same retained copy must pass with the candidate and disagree with the
  control. G1 checks the client-witnessed issued kit and custody across phases:
  its two exports may agree under a restore-only loss, but reconciliation must
  stay not-passed; keep that control too.
- A missing short-lived entity is not explained by "it probably expired":
  establish a conservative admitted-lifecycle bound or an ordered durable
  removal. A removal overlapping a multi-tick pre-census interval does not prove
  it followed the observation. Unverifiable cases stay **incomplete**, never
  passes obtained by dropping those entities.
- A removal control alters a derived census copy, never the world or saved
  evidence, and must detect loss of an observed, unexpired entity. Offline
  reconstruction and source review keep their own scope; neither becomes a live
  crash campaign or a combined-candidate review.

These are evidence requirements, not a claim that every profile runs a crash
campaign: select the candidate's
[declared scenario](AGENT_TOOLING.md#profiles-and-scenarios); the
[native drills](architecture/persistence.md#hazards-and-defense-layers) keep
their bounded fault model. A logged filesystem event establishes observed
ordering, not an atomic syscall breakpoint. Record missing cut moments, never
invented counts.

## Release-candidate breaker pass

**Before the tag (frozen candidate).** Stage the package through the release's
packaging path; record source revision, binary/package hashes, content pins and
plugin pack identity, and freeze them for the run. On the staged package run the
applicable clean-install, previous-release upgrade and platform/Loader attacks,
including the real launcher and singleplayer for a native-client release; run
`loader-live` on advertised Loader platforms and `title-connect` on the native
client when the candidate affects them. Audit release claims against receipts;
a missing platform or environment is blocked coverage, not a pass. macOS has no
local machine: its only pre-tag evidence is the `rc-check/<tag>` CI run of the
core tests ([releases](development/release-and-tags.md#core-release-cut)), which
covers no client or package.

**After publication, before announcement (distribution smoke).** Download the
real assets, verify checksums, inspect package contents and metadata, record the
artifact identities, and exercise clean install/start/join with those files and
the public manifest, without dev-data overrides; shipped content and plugin pins
must resolve. Record CI-produced bytes that differ from the staged package
separately and rerun affected functional attacks only when code, dependencies,
configuration or package/content identity meaningfully changed. Check
release-page links and upgrade instructions. A failed distribution smoke blocks
the announcement even when pre-tag gates passed.

## Breaker procedure

1. Read `AGENTS.md`, this protocol and the change: diff, claims, changed gates.
2. Build the attack list from the matrix: concrete steps per applicable class,
   a reason per skipped class; spend most time on the classes nearest the change.
3. Attack real binaries (release server for real-client gates) through the
   canonical harness; use a focused runner only when no existing one expresses
   the attack. Before the first destructive, security or migration attack make
   disposable local copies, previous-release worlds included; never touch the
   owner's originals or the live server. Private ports, data roots and Xvfb per
   breaker; kill only your own processes. Browser and noVNC viewers need their
   own proven headless isolation (a harness Xvfb does not isolate a browser in
   another process): never open a headed viewer on the owner's desktop, change
   its focus or use host input; if isolation cannot be proven, leave that GUI
   check blocked. Track and close whole owned browser instances.
4. For a failure, keep the first failing receipt and reduce it to a repro. Rerun
   once only when it adds evidence, on disposable state; never recreate
   corruption or repeat a destructive action on live data. Report `repro n/m` or
   why confirmation was unsafe.
5. Check gate hygiene on every gate the change added or modified.
6. Run a bounded independent exploratory attack not copied from the builder's
   script or this checklist; record hypothesis, action and result even when it
   holds.
7. At close, report attempted and planned-but-unexercised attacks and branches.
   A time-box is not coverage: a required attack left unexercised makes the
   verdict `blocked`, with its prerequisite and next step.

The builder fixes findings with a regression that goes red when the fix is
reverted, and a confirmed consumer-visible failure joins the nearest gate or
scenario. The breaker never fixes what it judges. After a relevant fix, rerun the
failed attack; no second independent review is needed for the same checkpoint.
P0/P1 findings or blocked required coverage prevent acceptance; P2/P3 findings
stay visible with an explicit disposition.

## Severity

- **P0**: crash, disconnect, data loss/corruption, dupe, security, or the player
  cannot do the advertised thing at all.
- **P1**: needs a non-obvious workaround, or the advertised claim is false.
- **P2**: wrong against vanilla or the docs, with an obvious workaround.
- **P3**: cosmetic.

An OpenAL/audio error under Xvfb is environment noise only when the client still
enters and stays in Play. GLFW/display failures, server disconnects, correction
storms, ghost state, missing publications and bridge timeouts are findings.

## Result contract

```yaml
verdict: pass | changes | blocked
target: <commit/tag/asset hashes attacked>
attacks:
  - class: <matrix class>
    steps: <what was done>
    result: held | broke
skipped_classes: [{class, reason}] # genuinely inapplicable only
unexercised_required: [{attack, branch, reason, next_step}]
exploratory_attacks: [{hypothesis, action, result, evidence}]
gate_hygiene: [{gate, negative_control, pre_existing_pass, branches, runs}]
findings:
  - severity: P0 | P1 | P2 | P3
    title: <concrete issue>
    repro: <command or steps>
    repro_rate: <n/m>
    expected: <vanilla/doc/claim>
    actual: <observed>
    evidence: [<server log, client log, result.json, screenshots>]
```

The YAML also carries the common result fields of
[running agents](development/agent-fleet.md#briefs) (`base`,
`changed_files: []`, `validation`, `gaps`, `asks`, `noticed`). The final line is
`QA_RESULT=PASS`, `QA_RESULT=CHANGES` or `QA_RESULT=BLOCKED`; `changes` is the
fail verdict. The CodexPro overlay (`tools/codexpro-pi-overlay/`) reads that
token: never report a noted defect as `PASS`.

Windows-guest QA through noVNC uses the persistent page driver in
[Windows and macOS client QA](development/windows-and-macos-client-qa.md).
