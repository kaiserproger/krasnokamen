# Worktrees, patches and integration

[AGENTS.md](../../AGENTS.md) "Git, local state and sibling repositories" sets
the rules: worktrees under `<work>/wt/`, Main's standing commit/push
authorization does not extend to workers, and the owner's dirty files stay intact.
This page is the procedure.

## Creating a worktree

```sh
<work>/solaris/tools/dev/wt-new <Name> <base-commit> [--repo core|client|loader|plugins]
<work>/solaris/tools/dev/wt-done <Name>   # verified final.patch, then removal
```

`wt-new` creates the branch `wt/<Name>`, the hard-link `data/vanilla`,
`<work>/receipts/<Name>/` and a private Gradle/XDG home; `wt-done` archives
everything not in the base as one verified patch, copies `.analysis` evidence
(`result.json`, logs, reports, screenshots) into the receipts and refuses while
other ignored files remain, all before it removes anything; `wt-gc` lists stale
worktrees. Details: [tools/dev](../../tools/dev/README.md).

Before removal, the [existing archive procedure](../../tools/dev/README.md#worktree-and-release-pin-tools-owner-host)
needs an inventory of retained untracked content, file modes and symlink targets,
plus a verified restoration. `git apply --check` proves applicability, not that
those bytes/metadata can be restored. Missing inventory after deletion is not a
recoverability claim. Archive payloads and ignored Mojang material stay outside
Git objects, including temporary indexes/commits that are never pushed; use the
allowlisted evidence archive, not `git add -A` as an archival mechanism.
Release binaries for gates come from `tools/dev/pin-release`, never from a
pooled target dir.

- **Absolute paths in every edit/write tool call.** Agent file tools resolve
  relative paths against the session root (the main checkout), not the
  worktree; `cd` in bash does not change that. A child agent corrupted the main
  checkout this way on 2026-10-16. Put the rule in every worktree brief and have
  parents forward it to children. Audit a suspect agent by searching its
  transcript for successful edits/writes with relative `crates/...` paths.
- **`data/vanilla` must be a real directory**, not a symlink: the registry
  refuses symlinked sidecar dirs (`wt-new` hard-links it). It must contain
  `data/minecraft/advancement/`; extract a full set with
  `tools/extract-vanilla-data.sh` from the local server jar.
- **Sibling checkouts resolve from the main checkout.** In a worktree the
  harness (`tools/harness/runtime.py` `sibling_root`, and the shell runners
  `regression.sh` and `prepare-real-client-scenario.sh`) finds `krasnokamen-loader`,
  `krasnokamen-client` and `krasnokamen-default-plugins` beside the main checkout, via
  `git rev-parse --git-common-dir`. No `KRASNOKAMEN_*_ROOT` export is needed; set
  `KRASNOKAMEN_LOADER_ROOT`, `KRASNOKAMEN_CLIENT_ROOT` or `KRASNOKAMEN_DEFAULT_PLUGINS_ROOT`
  only to select another checkout.
- Heavy outputs go under `<work>/targets/`; the runner's pooled target dirs
  handle this (see [heavy commands](heavy-commands-and-memory.md)).
- When the main checkout's binary or config changes under running agents,
  give them a frozen copy of the binary plus its SHA-256 instead.

## When nothing may be committed

Older runs worked on an uncommitted main tree. The patterns still apply to any
no-commit slice:

- **Seed from a dirty tree:** create the isolated worktree with `wt-new`, then
  copy only the audited source-file allowlist and propagate its deletions.
  Record the dirty baseline and diff before handing it off. Stage explicit
  source paths if a staged baseline is needed; never sweep archives, ignored
  content or tool state into an ordinary or temporary Git index.
  `wt-new` handles the ignored vanilla cache separately. Exclude alternate
  target dirs and `.analysis/`; nested `sdk/rust/target` and `.analysis`
  copies filled the disk twice.
- **Deliver patches with `git diff` only**, never hand-concatenated. A file
  without a trailing newline glues the next `diff --git` header onto its last
  line. Before applying: `grep -c '^diff --git' P` equals the claimed file count,
  then `git apply --check P`, keep a copy in the receipts, apply,
  `cargo fmt --all`. After applying, grep the tree for a stray `diff --git a/`.
- **Layered patches** from one agent are a strict chain (patch N applies on
  main + 1..N-1). Never "drop duplicates" by hand; a later patch applied over
  an earlier one silently removed its hunks once.
- **Main moved after the patch base:** merge per file with
  `git merge-file -L main -L base -L <slice> <main-file> <base-file> <wt-file>`
  (base from `git -C <wt> show :<path>`). Struct-literal field lists and
  appended tests are usually additive; exhaustive matches, destructures and
  tuple-shape changes need hand-combined code. `git apply` fails on files that
  are untracked in main ("does not exist in index").
- A slice running for hours while main moves rebases itself into a freshly
  seeded worktree before the final merge; run `cargo fmt --all` in the old
  tree first so formatting churn does not become conflicts.
- Shared-contract changes by one slice are not pushed onto siblings; Main
  migrates the new call sites at merge.

## Keeping the shared tree compilable

- Build/check profiles use the tree they run in; real-client profiles consume the
  pinned server selected by their declaration, not an implicit shared-tree build
  ([harness entrypoint](../AGENT_TOOLING.md#harness-entrypoint)). Keep a worker's
  unfinished edits isolated and hand off a compiling immutable checkpoint or an
  explicitly unverified patch; no placeholders or claimed compile success.
- When the shared tree is red, use `KRASNOKAMEN_SERVER_BINARY=<frozen>` and its
  matching client environment only for that recorded pin's scope; an old green
  binary does not prove the moving source compiles.
- Group a broken build's errors by file and map files to owners via mtimes and
  agent transcripts. Restore a clobbered uncommitted file only from a copy
  whose checksum matched before the incident in at least two untouched
  worktrees, never from HEAD or the offender's scratch copies. Back up affected
  files before merging a patch and dry-run it with `patch --dry-run -p1 -F0`.

## Integrating a branch into `main`

1. List the merge sequence with `git log --oneline --first-parent <base>..<tip>`.
2. Bisect failures by merge in a scratch worktree (`git checkout --detach
   <merge>`, cherry-pick later compile fixes with `-n`) and run only the
   failing tests. Run the same subset on the current `main` tip: failures
   already there are not from this integration.
3. Classify every failure as **stale fixture** (behaviour became correct,
   fix the test) or **real regression** (fix the code), with evidence per test.
   In one night run 14 of 15 failures were stale fixtures. Typical stale
   fixtures after vanilla-correctness work: reach distances, sign faces,
   border lerp targets, plugin API version constants, advancement condition
   shapes, raid chunk tickets.
4. Use the canonical harness's complete failure list, not an extra ad-hoc workspace
   suite. Typical post-merge breakage includes unfulfilled `#[expect(clippy::...)]`,
   stale code-health anchors and inline tests in owner-guarded files.
5. Main owns the integrated L2 under the runner; read `result.json` commands,
   exits, counts and logs ([gate contract](../AGENT_TOOLING.md#evidence-boundaries)).
   After a relevant fix rerun the failed scope, then the required combined gate;
   do not repeat unchanged greens. Supervised jobs preserve the inner exit code
   (`...; rc=$?; ...; exit $rc`).
6. Obtain the review required by AGENTS.md before Main's critical code commit.
   Check reviewer identity and exact base/candidate/diff coverage, including
   integration deltas; old, limited or peer-tree reviews keep their original scope.
   Missing required coverage blocks commit approval, not reachable tests or
   clearly labelled uncommitted validation pins.
7. Fast-forward `main` without touching the owner's dirty tree: back up
   `git diff` and untracked files under `<work>/backups/`, find the overlap
   between dirty files and the branch's changed files, set aside only the
   overlapping files, `git merge --ff-only <branch>`, restore them, check for
   conflict markers. Main's standing commit/push authorization comes from
   `AGENTS.md`; workers may commit on their own branch, never Main or a push.
   Tags and releases still need explicit owner authorization.

## Freezing one acceptance identity

For any gate set that must describe one build (release, perf round, QA wave):

1. Announce the freeze; no new patches. Unproven work goes to the next round.
   Pending edits go to the agents' own `pending.patch`; sibling repos are not
   frozen unless stated.
2. Pin the server with `tools/dev/pin-release <tree>` (`--client` for the
   native client). It records HEAD plus the hash of the uncommitted diff
   (untracked files included), builds once in a pooled slot, refuses to pin if
   the tree changed during the build, and publishes the binary and
   `manifest.json` to an immutable `<work>/pins/<id>/`. Build the plugin pack
   separately when WIT changed (`../solaris-default-plugins/tools/build_standard_pack.py`).
3. Announce the pin path, binary SHA-256 and manifest. Every gate and receipt
   cites them. Never relabel a preflight package as final.

The identity is the `krasnokamen` binary from `mc-edge`. Older freeze or snapshot
recipes in external skills or notes that build or copy `mc-server` describe the
retired v1 binary ([ADR 0014](../decisions/0014-v1-core-retirement.md)); they
cannot produce a current acceptance identity. The release procedure is in
[releases](release-and-tags.md).

## Background jobs

- Cancel a looping job through its parent (`proc://<job>/kill`), not by killing
  child runner processes; a surviving `for` loop starts the next item.
- Long builds and gates run as named services or async jobs with an explicit
  deadline; consume the completion once, never poll.
