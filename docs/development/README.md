# Development playbooks

How Solaris is developed on the owner's machine: procedures, pitfalls and
lessons that used to live only in agent skills, prompts and local memory.
Rules come first from [AGENTS.md](../../AGENTS.md), the
[Agent Constitution](../AGENT_CONSTITUTION.md),
[AGENT_TOOLING.md](../AGENT_TOOLING.md), the
[break-it QA protocol](../QA_AGENT_PROTOCOL.md) and the
[definition of done](../DEFINITION_OF_DONE.md); these pages do not restate
them, they explain how to follow them. When a page here disagrees with those
documents, those documents win. When an older skill, prompt or note outside
the repository disagrees with these pages, these pages win: each records the
newest rule and says what it replaced.

## When to read what

| You are about to... | Read |
|---|---|
| run Cargo, Gradle or the harness, or wonder why a command is slow, frozen or killed | [Heavy commands, memory and disk](heavy-commands-and-memory.md) |
| create a worktree, deliver a patch, integrate a branch, freeze a build | [Worktrees, patches and integration](worktrees-and-integration.md) |
| launch, brief, message or recover agents | [Running agents](agent-fleet.md) |
| run or add a real-client gate, compare against vanilla, smoke a server with a client | [Real-client gates in practice](real-client-gates.md) |
| debug a failing live gate or plugin | [Triage of failing live gates](live-gate-triage.md) |
| measure performance or profile | [Performance windows and profiling](performance-windows-and-profiling.md) |
| cut a core, client or Loader release, or fix a failed tag | [Releases and tags](release-and-tags.md) |
| deploy to or collect metrics from the public test server | [Public test server operations](test-server-ops.md) |
| test the Windows or macOS client | [Windows and macOS client QA](windows-and-macos-client-qa.md) |
| move to a new Minecraft version | [Bumping the Minecraft version](minecraft-version-bump.md) |
| ask for review, attack a change, run a committee vote, or write a worker prompt | [Advisor, reviewer, breaker and committee](review-committee-and-breaker.md) |
| split a giant module or audit the architecture | [Module extraction and architecture audits](refactoring-and-audits.md) |
| learn from past mistakes | [Lessons](lessons.md) |

## Conventions on these pages

- `<work>` is the owner's games-disk work root,
  `/run/media/kaiserroman/games/solaris-work` (AGENTS.md), holding the sibling
  checkouts (`solaris`, `solaris-client`, `solaris-loader`,
  `solaris-default-plugins`), agent worktrees `wt/`, build outputs `targets/`
  and agent receipts `receipts/`.
- Developer scripts are tracked under `tools/dev/` (the runner `mem-run.sh`, its
  `heavy-watchdog.py`, `scene-slot.sh`, `instrumentation-guard.py`, the
  test-server scripts in `tools/dev/test-server/` and the Windows-guest driver
  in `tools/dev/winqa/`).
- The public test server is addressed through `SOLARIS_TEST_HOST`; its address,
  SSH access and all credentials are held by the owner and never written here.
- Local run state (cursor, queues, receipts, committee answers, logs) stays in
  ignored `.analysis/`; it is not documentation and is not startup context.
- Agent roles are named, models are not: the owner chooses them.
