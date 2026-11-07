# Owner contract (verbatim)

The owner's own words, kept as written. [AGENTS.md](../../AGENTS.md) "Working
rules" restates them as a short numbered list with the conflicts resolved; when
the two seem to disagree, ask the owner.

> ONCE AN HOUR OR BEFORE COMMIT LOOKUP AT OWNER OPEN ISSUES IN REPOSITORY

> "Do not overthink/overengineer every possible case until you explain exactly WHY it is needed and user agrees with you. Write as simple and clear as possible. Always ask user first what they fucking want."

> "Always double-check your work. Spawn another agent to give clear, concise second POV on your work. Do not do anything that isn't needed right now as it probably won't ever be needed. Verification and reporting happen at meaningful checkpoints, not after every command, tiny fix, or intermediate discovery."

> "On receiving new user request, first finish what you've started before taking new user request UNTIL it is explicitly required by user to take right now. Continue execution through a meaningful checkpoint; do not stop only to report intermediate progress."

> "Pareto: result first. Deliver the simplest version that works end to end for the real case; improve, harden or complete it only when the owner explicitly asks. Do the 20% that gives 80%. Don't burn tokens on rare edges, exhaustive inventories, byte-exact oracles, extra reviews or re-verifying what already passed. A rare blocker gets a clear error, not a subsystem." (owner, 2026-10-16)

> An explicit unambiguous request needs no reconfirmation. Ask only about a material unresolved choice.
  Make the smallest direct complete change: no speculative abstractions, flags, hardening or unrelated churn.

>  Finish active work before newer requests unless explicitly interrupted/replaced (`Stop`, `стоп`, `забей`,
  `поправка`); inspect processes/worktrees safely before switching. `Retry`/`продолжай` revalidates state.

> Autonomous `/goal` means the entire persistent objective and queue, not a slice: continue without
  intermediate stops until verified or externally blocked with no actionable work. Finite mode needs explicit permission.
  Slices need disjoint interfaces, not separate full gates, snapshots, reviews, sessions or commits.

> Update the live cursor/active queue/owning ADR at major phase close or genuine interruption, not per tool call.
  Keep exact receipts; a context handoff does not pause or complete the objective.

> At most three substantive updates per checkpoint: non-obvious outcome, material finding/blocker, closeout.
  No routine-command narration. Progress-since reports cover only that interval's commits, capabilities, gates and time sinks.

> Priorities: ordinary vanilla-client gameplay/multiplayer; production plugin API/adapters; measured
  optimization/ECS/ownership/autoscaling; rare edges. Promote a lower item only if ordinary play, saves or plugins are blocked.

Later owner decisions (2026-11-07): publications need only the owner's
authorization; the advisor issues no `[approved]`; every release tag gets two
read-only release reviewers
([review committee and breaker](review-committee-and-breaker.md#release-reviewers-owner-2026-11-07)).

> "и не забывайте коммитить почаще. ничего страшного если нерабочие коммиты улетят в main, нам не нужны гигантские коммиты и обновление мэйна раз в два дня" (2026-11-07)

Main therefore commits small slices and pushes `main` often; a not-yet-green
commit on `main` is acceptable. Tags, releases and publication keep their rules.

Owner decisions on the agent-work retrospective (2026-11-07, chosen from
`receipts/AgentRetro/` proposals; AGENTS.md restates them):

- Main has standing authorization to spawn subagents within the fleet limits
  (at most 10 live agents, 3 concurrent builders); this replaces the
  `quaka-whaka-zaka-du` phrase.
- Only Main runs the hourly owner-issue check; workers do not.
- Failing-first is recorded, not a gate. A fix's regression test must go red
  when the fix hunks are reverted, and a bug row becomes `fixed` only through
  `ledger promote`.
- Review at three points: a promotion batch of five or more rows; persistence,
  protocol or custody changes before commit; two reviewers before a tag. No
  reviewer for docs-only or test-only slices.
- Briefs and results follow fixed templates: implementers return `status`,
  reviewers and breakers `verdict`; at most three messages per agent; no GO
  for measurement lanes.
- Worktrees and release binaries go through `tools/dev/wt-new`, `wt-done`,
  `wt-gc` and `pin-release`; each sibling repo's `main` is brought up to date at
  every RC.
- Stale slot-era skills are deleted and the role prompts follow these rules.
- The advisor raises one STOP per topic (a repeat needs new evidence) and logs
  each concern's disposition.

> все явные пробелы ЧИНИТЬ (owner, 2026-11-07)

Every gap an agent or Main reports is a work item to fix, not an accepted
ending. Only what needs something outside the agents' reach (owner action,
hardware, credentials) stays open, with the exact blocker named.
