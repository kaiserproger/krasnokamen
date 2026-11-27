# Krasnokamen

Rust Minecraft Java server, releases 26.1.2–26.3 (canonical 26.3). `CLAUDE.md`
links here. Owner: kaiserproger; Main is the owner's coordinating agent.

@docs/RULES.md
@docs/PITFALLS.md

If your tool does not expand `@` lines, read [docs/RULES.md](docs/RULES.md) and
[docs/PITFALLS.md](docs/PITFALLS.md) before working.
What works today and how it was checked: [docs/STATUS.md](docs/STATUS.md).
Choices and their reasons: [DECISIONS.md](DECISIONS.md). Setup, where tests go
and commits: [CONTRIBUTING.md](CONTRIBUTING.md). Every other page:
[docs/README.md](docs/README.md).

On the owner host (`/run/media/kaiserroman/games/solaris-work/solaris/tools/dev/mem-run.sh`
exists): absolute paths in every command and file tool, your own worktree from
`tools/dev/wt-new`, heavy Cargo/harness/Gradle commands through Main's
`mem-run.sh -m 6G` (`12G` for full test or correctness), release binaries for
gates from `tools/dev/pin-release`
([heavy commands](docs/development/heavy-commands-and-memory.md),
[worktrees](docs/development/worktrees-and-integration.md),
[running agents](docs/development/agent-fleet.md)). Never stage ignored local
state (`.analysis/`, `data/vanilla*`, `.serena/`, `.opencode/`, `x-ui-pro/`,
`YOLO_MODE.md`, `log.log`, `opencode.json`).

Before finishing a change: `python3 -m tools.harness run fmt`,
`python3 -m tools.harness run code-health`, the tests of what you touched
(`python3 -m tools.harness run test -p <crate> --lib <filter>`), and for a code
commit `python3 -m tools.harness run correctness` (strict Clippy with
`-D warnings` and every test; Main runs one per integrated batch). Markdown
only: `python3 tools/doclinks.py`. Update `docs/STATUS.md` when what works
changed, and `docs/PITFALLS.md` when you hit a mistake worth not repeating.
