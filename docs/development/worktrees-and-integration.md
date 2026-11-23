# Worktrees, patches and integration

[AGENTS](../../AGENTS.md) owns Git authorization: Main's standing commit/push
permission does not extend to workers; preserve owner dirt. `<work>` and
[historical provenance](README.md) are defined by the development index.

## Creating a worktree

```sh
<work>/solaris/tools/dev/wt-new <Name> <base-commit> [--repo core|client|loader|plugins]
<work>/solaris/tools/dev/wt-done <Name>   # only when authorized; run outside it
```

The [tool contract](../../tools/dev/README.md#worktree-and-release-pin-tools-owner-host)
owns creation, archival, refusal conditions and pin mechanics:

| Tool / boundary | Required practice |
|---|---|
| `wt-new` | Branch `wt/<Name>`, receipts `<work>/receipts/<Name>/`, private Gradle/XDG home and `.wt-env`; core caches are hard-linked. Load the environment explicitly |
| Cache | `data/vanilla` is a real directory, not a sidecar symlink; include `data/minecraft/advancement/` (full local jar extraction via `tools/extract-vanilla-data.sh`). Hard links share bytes: replace files, never edit in place |
| `wt-done` | Archives everything outside base into a verified patch and allowlisted `.analysis` evidence before removal; unknown ignored files cause refusal. **Before removal**, inventory retained untracked bytes, modes and symlink targets and verify restoration. `git apply --check` proves applicability, not recoverability. Missing inventory after deletion is not a recoverability claim |
| Archive isolation | Keep payloads/ignored Mojang data outside Git objects, including temporary indexes/commits; no `git add -A` archival shortcut |
| `wt-gc` | Read-only stale-worktree inventory, not deletion authority |
| Build output | Runner pool under `<work>/targets/`; gates use `pin-release`, never pooled output. Give running consumers a frozen binary and SHA when shared binary/config changes |

**Every file-tool edit/write uses an absolute worktree path.** Shell `cd` does
not retarget file tools. Put this rule in every worktree brief; parents forward
it to children. Investigate suspected Main clobber by finding successful
relative `crates/...` edits in the transcript (2026-10-16 incident).

Harness sibling discovery (`tools/harness/runtime.py::sibling_root`,
`regression.sh`, `prepare-real-client-scenario.sh`) resolves beside the main
checkout using `git rev-parse --git-common-dir`, not beside `wt/<Name>`.
Use `KRASNOKAMEN_LOADER_ROOT`, `KRASNOKAMEN_CLIENT_ROOT` or
`KRASNOKAMEN_DEFAULT_PLUGINS_ROOT` only to select a different checkout.

## When nothing may be committed

| Situation | Procedure |
|---|---|
| Seed from dirty Main | `wt-new`, then copy only audited source allowlist **and deletions**; record dirty baseline/diff. If staging a baseline, stage explicit source paths only. Never include archives, ignored content, tool state, alternate targets or `.analysis` (including nested SDK copies); `wt-new` handles vanilla separately |
| Deliver | Generate with `git diff`, never concatenate (missing trailing newline can glue the next `diff --git`). Confirm header count equals claimed files, `git apply --check`, preserve patch in receipts, apply, then `cargo fmt --all`; detect stray `diff --git a/` text afterwards |
| Layered patches | Strict chain: N applies to Main + 1..N−1. Never hand-drop “duplicates”: this previously removed earlier hunks |
| Main moved | Three-way per file: `git merge-file -L main -L base -L <slice> <main-file> <base-file> <wt-file>` (base from `git -C <wt> show :<path>` when that index still holds the seeded baseline). Field lists/appended tests may be additive; exhaustive matches, destructures and tuples need semantic merging. `git apply` fails on an untracked Main file: “does not exist in index” |
| Long-lived slice | Rebase into a freshly seeded worktree before final merge; run `cargo fmt --all` in the old tree first, not during another worker's edits |
| Shared contract | Main migrates new call sites at integration; never push a slice's contract onto sibling workers |

## Keeping the shared tree compilable

- Build/check uses its own tree; real-client profiles use their declared pin
  ([harness entrypoint](../AGENT_TOOLING.md#harness-entrypoint)). Keep unfinished
  work isolated; deliver a compiling immutable checkpoint or an explicitly
  unverified patch, never placeholders or invented compile success.
- On a red shared tree, `KRASNOKAMEN_SERVER_BINARY=<frozen>` and matching client
  environment prove **only that pin**, not the moving source.
- Attribute errors by file, mtimes and transcripts. Recover a clobbered dirty
  file only from a copy whose checksum matched before the incident in at least
  two untouched worktrees—not HEAD or the offender's scratch. Back up affected
  files; dry-run patching with `patch --dry-run -p1 -F0`.

## Integrating a branch into `main`

1. List first-parent merges with `git log --oneline --first-parent <base>..<tip>`.
   Bisect in scratch (`git checkout --detach <merge>`, later compile fixes via
   `cherry-pick -n`); run only failing tests and compare the same subset on Main.
   Baseline failures are not integration regressions.
2. Classify every failure, with evidence: stale fixture (fix test) or real
   regression (fix code). Historical 14/15 stale failures are not a presumption:
   reach/sign/border/advancement/raid/API fixtures can drift. Use canonical
   harness failure lists, not another ad-hoc suite; include Clippy expectations,
   code-health anchors and owner-guarded inline tests.
3. **One shared L2 per compatible batch** (owner D85, 2026-11-22): freeze the
   integrated tree; Main runs `correctness` through the runner once. Each row
   still needs focused checks, original repro, its own fix-only reversal and
   required real-client/review evidence. Reuse only matching tree, inputs,
   environment and scope; one row's reversal never proves another.
4. Inspect commands/exits/counts/logs in `result.json`
   ([evidence contract](../AGENT_TOOLING.md#evidence-boundaries)). After a relevant
   fix rerun failed scope then required combined gate, not unchanged greens.
   Supervisors preserve inner status (`...; rc=$?; ...; exit $rc`).
5. Before Main's critical commit obtain required independent review of exact
   identity/base/candidate/diff **including integration deltas**. Old, limited or
   peer-tree reviews keep their scope. Missing coverage blocks commit approval, not
   reachable tests or labelled uncommitted validation pins.
6. Fast-forward without losing owner dirt: back up diff and untracked bytes
   under `<work>/backups/`, identify overlap, set aside **only** overlapping
   files, `git merge --ff-only <branch>`, restore and inspect conflict markers.
   Workers commit only on their own branch when permitted, never Main or push;
   tags/releases require explicit owner authorization.

## Freezing one acceptance identity

1. Announce freeze for release/perf/QA; no new patches. Unproven work goes to
   next round, pending edits to each owner's `pending.patch`. Siblings freeze
   only if explicitly included.
2. Call Main's `tools/dev/pin-release <tree>` (`--client` for native client).
   It records HEAD, dirty/untracked hash and build inputs, refuses to pin if the
   tree changed during the build, and publishes a read-only binary plus manifest
   in `<work>/pins/<id>/`.
   The [pin contract](../../tools/dev/README.md#worktree-and-release-pin-tools-owner-host)
   owns reuse, target, incremental and environment rules. WIT changes also require
   `../solaris-default-plugins/tools/build_standard_pack.py`.
3. Announce path/SHA/manifest; all gates cite them. Never relabel preflight as
   final. Server identity is `mc-edge`'s `krasnokamen`, never retired `mc-server`
   ([ADR 0014](../decisions/0014-v1-core-retirement.md)); use the [release procedure](release-and-tags.md).

## Background jobs

Cancel looping jobs through the parent (`proc://<job>/kill`), not child runners.
Use named services/async jobs with explicit deadlines; consume completion once,
never poll. [Resource policy](heavy-commands-and-memory.md) owns admission.
