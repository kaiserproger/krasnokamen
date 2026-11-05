# Worktrees, patches and integration

[AGENTS.md](../../AGENTS.md) "Git, local state and sibling repositories" sets
the rules: worktrees under `<work>/wt/`, absolute paths, no commits without
authorization, preserve the owner's dirty files. This page is the procedure.

## Creating a worktree

```sh
git -C <work>/solaris worktree add --detach <work>/wt/<Name> <base-commit>
```

- **Absolute paths in every edit/write tool call.** Agent file tools resolve
  relative paths against the session root (the main checkout), not the
  worktree; `cd` in bash does not change that. A child agent corrupted the main
  checkout this way on 2026-10-16. Put the rule in every worktree brief and have
  parents forward it to children. Audit a suspect agent by searching its
  transcript for successful edits/writes with relative `crates/...` paths.
- **`data/vanilla` must be a real directory**, not a symlink: the registry
  refuses symlinked sidecar dirs. Copy or hard-link it from another tree:
  `mkdir -p <wt>/data && cp -al <other>/data/vanilla <wt>/data/vanilla`
  (create `data/` first, otherwise the contents land in the wrong place). It
  must contain `data/minecraft/advancement/`; extract a full set with
  `tools/extract-vanilla-data.sh` from the local server jar.
- **`SOLARIS_LOADER_ROOT`** must point at a Loader checkout in worktree gates;
  without it the `real_client_manifest` tests in `mc-test-harness` fail on the
  missing Gradle adapter, which is environment, not code.
- Heavy outputs go under `<work>/targets/`; the runner's pooled target dirs
  handle this (see [heavy commands](heavy-commands-and-memory.md)).
- When the main checkout's binary or config changes under running agents,
  give them a frozen copy of the binary plus its SHA-256 instead.

## When nothing may be committed

Older runs worked on an uncommitted main tree. The patterns still apply to any
no-commit slice:

- **Seed from a dirty tree:** `git worktree add --detach <wt> HEAD`, copy the
  tracked and untracked non-ignored files
  (`git ls-files -co --exclude-standard -z | rsync -a --from0 --files-from=-`),
  propagate deletions (`git ls-files -d`), remove empty crate directories left
  by deleted crates (the `crates/*` glob breaks cargo), copy `data/vanilla`,
  then `git add -A` so the baseline is staged and the agent's work is the
  unstaged `git diff` plus untracked files. Exclude alternate target dirs and
  `.analysis/`; nested `sdk/rust/target` and `.analysis` copies filled the disk
  twice.
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

- Every harness gate builds the server from the tree it runs in. Agents editing
  a shared tree keep unfinished modules unwired (no `mod x;` for a missing
  file) and wire only compiling code. One owner runs the combined
  `cargo check -p mc-edge` and announces "compiles"; others don't repeat it.
- When the shared tree is red, run gates against a frozen server with
  `SOLARIS_SERVER_BINARY=<frozen>` plus the client environment file of the last
  green gate.
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
4. Use `cargo test --workspace --all-targets --no-fail-fast` once to get the
   whole failure list instead of one failure per L2 run. Typical post-merge
   breakage: unfulfilled `#[expect(clippy::...)]`, collapsible `if let` chains,
   stale code-health ownership anchors after renames, inline `#[cfg(test)]`
   modules in rule-guarded files (move to sibling `*_tests.rs`).
5. Run L2 under the runner (see AGENTS.md); read `result.json` exits (stage
   order fmt, clippy, code-health, test). After a failure rerun only the
   affected tests, then the full gate once. When the gate runs as a supervised
   service, keep the inner exit code (`...; rc=$?; ...; exit $rc`).
6. One read-only reviewer on the fix commits.
7. Fast-forward `main` without touching the owner's dirty tree: back up
   `git diff` and untracked files under `<work>/backups/`, find the overlap
   between dirty files and the branch's changed files, set aside only the
   overlapping files, `git merge --ff-only <branch>`, restore them, check for
   conflict markers. Do not push without explicit permission.

## Freezing one acceptance identity

For any gate set that must describe one build (release, perf round, QA wave):

1. Announce the freeze; no new patches. Unproven work goes to the next round.
   Pending edits go to the agents' own `pending.patch`; sibling repos are not
   frozen unless stated.
2. Fingerprint the source (tracked + untracked non-ignored files, excluding
   `target/`, `.analysis/`, `.git/`, `data/vanilla/`, logs, tmp, `__pycache__`):
   `find ... -print0 | sort -z | xargs -0 sha256sum | sha256sum`.
3. Build once (release server, plus plugin pack when WIT changed:
   `../solaris-default-plugins/tools/build_standard_pack.py`), copy the binary
   to an immutable path named with its short SHA.
4. Recompute the fingerprint after the build; seal only if it is unchanged,
   otherwise discard the binary and build from a snapshot.
5. Announce path + SHA-256 + fingerprint. Every gate and receipt cites them.
   Never overwrite a busy frozen binary or relabel a preflight package as final.
   Compare binary SHAs when agents reuse private target dirs: cargo can return
   the previous binary.

## Background jobs

- Cancel a looping job through its parent (`proc://<job>/kill`), not by killing
  child runner processes; a surviving `for` loop starts the next item.
- Long builds and gates run as named services or async jobs with an explicit
  deadline; consume the completion once, never poll.
