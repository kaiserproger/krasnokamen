# Development playbooks

How work is done on Krasnokamen, by contributors and on the owner host.
Authority follows [AGENTS' precedence order](../../AGENTS.md#which-rule-wins):
the [owner contract](owner-contract.md), the
[Constitution](../AGENT_CONSTITUTION.md) and AGENTS.md win over these pages, and
code, tests and observed runtime win over any prose; fix the page when they
disagree. Older wording is in Git history (`git log -p -- docs/development/<page>.md`).

## When to read what

| You are about to... | Read |
|---|---|
| add or change a vanilla gameplay feature | [Adding a feature](adding-a-feature.md) |
| change packets or protocol handling | [Protocol changes](protocol-changes.md) |
| run Cargo, Gradle or the harness on the owner host, or find out why a command is slow, frozen or killed | [Heavy commands, memory and disk](heavy-commands-and-memory.md) |
| create a worktree, deliver a patch, integrate a branch, freeze a build | [Worktrees, patches and integration](worktrees-and-integration.md) |
| launch, brief, message or recover agents | [Running agents](agent-fleet.md) |
| run or add a real-client gate, compare against vanilla, smoke a server with a client | [Real-client gates](real-client-gates.md) |
| debug a failing live gate or plugin | [Triage of failing live gates](live-gate-triage.md) |
| accept horizon LOD output | [Horizon LOD acceptance](horizon-lod-acceptance.md) |
| measure performance or profile | [Performance windows and profiling](performance-windows-and-profiling.md) |
| run protocol-bot load against a release server | [Catalog load](catalog-load.md) |
| run the imported-city functional fixture | [Constantinople fixture](constantinople-fixture.md) |
| cut a core, client or Loader release, or fix a failed tag | [Releases and tags](release-and-tags.md) |
| deploy to or collect metrics from the public test server | [Public test server operations](test-server-ops.md) |
| test the Windows or macOS client | [Windows and macOS client QA](windows-and-macos-client-qa.md) |
| move to a new Minecraft version | [Bumping the Minecraft version](minecraft-version-bump.md) |
| ask for review, attack a change, run a committee vote, or write a worker prompt | [Advisor, reviewer, breaker and committee](review-committee-and-breaker.md) |
| split a giant module or audit the architecture | [Module extraction and architecture audits](refactoring-and-audits.md) |
| learn from past mistakes | [Lessons](lessons.md) |
| work from your own fork, without the owner host | [Contributing: coding agents on a fork](../../CONTRIBUTING.md#coding-agents-on-a-fork) |

Validation levels, receipts and the harness catalogue are in
[agent tooling](../AGENT_TOOLING.md); quality labels in the
[definition of done](../DEFINITION_OF_DONE.md); bug rows in the
[ledger contract](../bugs/README.md).

## Conventions on these pages

- `<work>` is the owner host's work root,
  `/run/media/kaiserroman/games/solaris-work`, holding the sibling checkouts
  (`solaris`, `solaris-client`, `solaris-loader`, `solaris-default-plugins`;
  host paths keep these names), agent worktrees `wt/`, build outputs `targets/`,
  release pins `pins/` and agent receipts `receipts/`. Pages written for the
  owner host say so; elsewhere use your own paths.
- Host scripts live in [`tools/dev/`](../../tools/dev/README.md): the runner
  `mem-run.sh` and its watchdog, `pin-release`, `wt-new`/`wt-done`/`wt-gc`,
  `scene-slot.sh`, `perf-window.sh`, the test-server scripts in
  `tools/dev/test-server/` and the Windows-guest driver in `tools/dev/winqa/`.
- The public test server is addressed through `KRASNOKAMEN_TEST_HOST`; its
  address, SSH access and credentials stay with the owner and are never written
  here.
- Local run state (cursor, queues, receipts, committee answers, logs) lives in
  ignored `.analysis/`: optional, not documentation and not startup context.
  Handoffs follow [running agents](agent-fleet.md#handoff-across-context-loss-or-to-another-person).
- Agent roles are named here, models are not: the owner chooses them.
