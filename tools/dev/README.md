# Local development tools

These Linux host tools are intended for coordinated Solaris development, not production deployment.

- `mem-run.sh` runs commands in a memory-capped systemd user scope, shares Cargo target directories and a jobserver, and starts the watchdog. `-p` serializes the performance lane; `-x` requests a quiet measurement window and needs Main's GO. On the shared host every checkout, worktree and sibling repo calls Main's copy by absolute path (`<work>/solaris/tools/dev/mem-run.sh`), never its own: mixed runner versions would share one lock/registration directory.
  Below `SOLARIS_SLOT_MIN_FREE_GB` (default 80) + 10 GiB free on the targets disk, a non-`-x`/`-p` command first runs `slot-gc --delete` on idle slots (bounded by `SOLARIS_SLOT_GC_TIMEOUT`, default 120 s) and prints what it freed; a GC error is printed and the command starts anyway.
  Its target dir comes from `slot-select.sh` (sourced): a pooled slot is owned by one tree at a time. A picked slot, or an explicit `CARGO_TARGET_DIR` on a free pooled slot, takes its pool lock; a held slot is shared only by a command of the same tree (pin-release's inner build), and another tree is refused with exit 75, because two trees in one slot let cargo reuse the other tree's binaries. Every command on a slot holds its use lock (`<l>.use`) shared, which slot-gc and a taking-over tree wait for.
- `heavy-watchdog.py` monitors slice memory pressure and freezes eligible builds, then thaws them. The runner launches it as `solaris-heavy-watchdog.service`.
- `scene-slot.sh` serializes graphical scene runs through three host-wide flock slots.
- `instrumentation-guard.py -- COMMAND ...` runs trusted instrumentation under the runner and Linux Landlock, allowing writes only in a private `/dev/shm` directory. Preserve that directory while inspecting outputs.
- `test-server/` contains the local metrics collector and remote fresh-world deployment, service, and soak tools; see `test-server/README.md` before use.
- `winqa/` contains a noVNC driver; run `npm install --prefix tools/dev/winqa` once, then `node tools/dev/winqa/cs-daemon.js`, and follow `winqa/RECIPE.md`.
- `wt-new`, `wt-done`, `wt-gc`, `pin-release` (with the shared `wtlib.py`) create, archive and audit agent worktrees and pin release binaries; `in-use` marks paths other agents rely on, `slot-gc` clears dead artifacts from idle target slots, `main-guard` reports unexpected changes in Main's source dirs; see [Worktree and release-pin tools](#worktree-and-release-pin-tools-owner-host).

## Fresh machine setup

1. Clone the repository and use `tools/dev/` directly; preserve executable permissions. Install Bash, Python 3, Git, rsync, util-linux (`flock`, `taskset`, `lscpu`), systemd user services, and the development toolchain. Landlock ABI 3 or later is needed for the instrumentation guard.
2. Use cgroup v2 with a working delegated systemd user manager. The runner starts `solaris-heavy.slice`, sets its runtime memory cap to 25G, swap cap to 2G, `AllowedCPUs=0-11`, and clears its `CPUQuota`; individual scopes receive a CPU quota based on physical cores (override with `SOLARIS_MEM_RUN_CPU_QUOTA`, a percentage number). Adjust the machine's setup or the script for a different CPU topology or capacity. The `-p` lane specifically requires CPUs 8-11.
3. Ensure `/tmp/solaris-mem-slots` is writable and shared by cooperating runs; it contains locks, FIFOs, registrations and peak history. Set `SOLARIS_SLOT_TARGETS` to a writable shared target pool on a suitable disk. The default is the original host's target path. Explicit `CARGO_TARGET_DIR` bypasses slot selection.
4. The first runner invocation starts the transient `solaris-heavy-watchdog.service` from the adjacent Python file; no separately installed unit is required. Verify the user manager can start it and access the slice's cgroup files. Ordinary runs warn if it cannot start; `-x` refuses to run without it.

The original ignored `bin/` contained a host-specific `cargo-component` 0.21.1 executable, not project source. Install that version separately when needed (`cargo install cargo-component --version 0.21.1 --locked`); it is not vendored here.

## Worktree and release-pin tools (owner host)

Four Python scripts (stdlib only) replace hand-rolled worktree setup and ad-hoc release binaries. They only work on the owner host: every path is anchored at `<work>` = `/run/media/kaiserroman/games/solaris-work`. Call Main's copy by absolute path (`<work>/solaris/tools/dev/wt-new` and so on). None of them ever modifies or removes the main checkouts `<work>/solaris`, `<work>/solaris-client`, `<work>/solaris-loader` and `<work>/solaris-default-plugins`; none creates a commit, a tar archive or a symlink.

Layout: worktree `<work>/wt/<Name>` on branch `wt/<Name>`, build output `<work>/targets/<Name>`, tool state (Gradle home, XDG dirs) `<work>/wt-home/<Name>`, receipts `<work>/receipts/<Name>/`, pins `<work>/pins/<id>/`.

### `wt-new <Name> [base] [--repo core|client|loader|plugins]`

Creates the worktree `<work>/wt/<Name>` on a new branch `wt/<Name>` from `base` (any revision of that repo; default the main checkout's HEAD; default repo `core`).

- Writes `<work>/receipts/<Name>/wt.json` (repo, branch, base sha) and `<worktree>/.wt-env`: `GRADLE_USER_HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`, `XDG_DATA_HOME`, `XDG_STATE_HOME` (all under `<work>/wt-home/<Name>`, empty at the start: a first Gradle run downloads its distribution and dependencies, and the client finds no `solaris-client` data folder until a gate or you populate one), and `WT_NAME`, `WT_REPO`, `WT_BASE`, `WT_RECEIPTS`. Load it with `. <worktree>/.wt-env`. `.wt-env` is listed in the repository's shared `.git/info/exclude`, so it never shows in `git status`.
- For `core` it provisions `data/vanilla` as a **hard-link copy** (`cp -al`) of the main checkout's `data/vanilla`. It is never a symlink: the harness rejects sidecar symlinks, and a stale cache once broke about 3000 tests. Hard links share bytes with the main checkout: replace a file, never edit it in place.
- Prints the absolute paths and the exact command prefix for heavy work: `<work>/solaris/tools/dev/mem-run.sh -m 6G env RUSTC_WRAPPER= <command>`. It sets no `CARGO_TARGET_DIR`: the runner gives the worktree a pooled target dir (shared dependency builds, per-tree marker, disk floor); twelve separate target dirs filled the disk on 2026-11-07.
- Refuses (nothing is created) when the name is invalid, `<work>/wt/<Name>`, `<work>/targets/<Name>` or `<work>/wt-home/<Name>` exists, branch `wt/<Name>` exists in any repo, `receipts/<Name>/final.patch` exists (a finished worktree used the name), the base is not a commit, or the main `data/vanilla` is missing or a symlink. A failure after the worktree was added removes what was created.

### `wt-done <Name> [--drop-untracked] [--drop-ignored] [--force "<reason>"]`

Archives and removes the worktree. Run it from outside the worktree.

- Writes `<work>/receipts/<Name>/final.patch` and `final.patch.sha256`: one binary-safe diff (`git diff --binary --full-index`, no renames, `a/ b/` prefixes) from the base to the worktree's content, so it holds the committed changes, staged and unstaged edits, and untracked files that are not ignored. The base is the one `wt-new` recorded, otherwise the merge-base with `main` (printed). The worktree's own index is not touched. Also written: `final.json` (base, head, tree, hashes, counts) and `final.commits.txt` (`git log base..HEAD`, so commit messages survive).
- Verifies before removing anything: a temporary worktree is checked out at the base, the patch is applied with `git apply --binary`, and the resulting tree must equal the worktree's tree. The temporary worktree is removed afterwards.
- Only then removes the worktree (`git worktree remove --force`), `<work>/targets/<Name>` and `<work>/wt-home/<Name>`.
- Commits that no other ref holds are kept: the branch stays (`wt/<Name>` or the branch the worktree was on), and a detached HEAD with such commits is pinned as `refs/archive/wt/<Name>`. A `wt/<Name>` branch without unique commits is deleted.
- Ignored files: evidence in any `.analysis/` dir is copied to `receipts/<Name>/analysis/<path>` before removal: reports, logs and receipts (`.json`, `.jsonl`, `.log`, `.md`, `.txt`, `.yaml`, `.toml`, `.patch`, `.diff`, `.tsv`, `.csv`, `.sha256`, `.html`, so `result.json` survives), screenshots (`.png`, `.jpg`, `.webp`, `.svg`) and repro drivers (`.py`, `.sh`, `.rs`, `.js`, `.java`, `.kt`, `.mcfunction`); worlds, caches and binaries there are dropped (the disk rule). Reproducible content (`target/`, `build/`, `.gradle/`, `__pycache__/`, `data/vanilla`, `data/vanilla-*`, `.wt-env`) is deleted silently. Any other ignored file makes `wt-done` refuse before writing anything, listing the files; copy what is needed out, or pass `--drop-ignored`, which records their names in `final.dropped-ignored.txt`.
- `--drop-untracked` leaves untracked non-ignored files out of the patch and discards them with the worktree; their names are saved in `final.dropped-untracked.txt`. Without it they are archived.
- Refuses, removing nothing, when: the name is not a registered linked worktree under `<work>/wt/` (the main checkouts are refused), the worktree is locked, another owner marked the worktree or a path inside it in the [in-use registry](#in-use-registry-in-use-addremovelist) (the owners are printed; `--force "<reason>"` overrides and logs the reason), no base can be found, a file cannot be read or indexed, untracked files exceed 100 MiB, the patch does not apply to the base, or the rebuilt tree differs (for example a nested git repository inside the worktree). A failed verification leaves no `final.patch`.

### `wt-gc [--days 7]`

Read-only report; it never deletes and never rewrites an index. Covers every registered worktree under `<work>/wt/` of the four repos (the main checkouts are ignored): age of the last change (newest of the dirty files' mtimes, the last commit not in `main`, and the worktree's HEAD reflog), dirty file count, commits not in `main`, size of its target dir and tool home. Entries unchanged for `--days` days are marked `STALE` and listed with the `wt-done` command to run (dirty files and unmerged commits are noted next to it). Pins in `<work>/pins/` are listed with age and size; older ones are marked `STALE` (delete a pin by hand, deliberately).

### `pin-release <worktree|repo-root> [--client]`

Builds or reuses one immutable release binary for a tree. Without `--client` it pins the core server (`solaris-v2`), with `--client` the native client (`solaris-client`; the tree must be a solaris-client checkout).

- Provenance: the tree's HEAD sha and the sha256 of `git diff HEAD --binary`, computed over the working tree **including untracked non-ignored files** (identical to `git diff HEAD --binary` when there are none). The pin id is `<sha12>` for a clean tree and `<sha12>-<diff8>` for a dirty one, and the dirt is recorded in the manifest.
- If `<work>/pins/<id>/manifest.json` exists the pinned path is printed and nothing is built; the pinned binary's sha256 is checked first, and a damaged pin is an error, not a rebuild.
- Otherwise it builds with `<work>/solaris/tools/dev/mem-run.sh -m 6G env RUSTC_WRAPPER= cargo build --release --locked -p mc-edge --bin solaris-v2 --message-format=json-render-diagnostics` (client: `-p solaris-client --bin solaris-client`), as a plain cargo command so `mem-run.sh` keeps it preemptible; a caller's `CARGO_TARGET_DIR` is dropped so the build always lands in a pooled slot. The runner releases the slot when that build exits, and another tree could then replace the artifact, so pin-release takes the slot's pool lock (`/tmp/solaris-mem-slots/<l>`) itself, re-marks the slot for this tree (touching its tracked files, as the runner does on a tree switch) if another tree used it meanwhile, reruns the same build with `CARGO_TARGET_DIR=<slot>` (usually a no-op) and copies the binary while still holding the lock. The artifact path comes from cargo's JSON output. The binary is copied to `<work>/pins/<id>/`, and `manifest.json` records `repo`, `head`, `diff_sha256`, `dirty`, `untracked_files`, `source`, `binary`, `binary_sha256`, `built_at`, `command`. A concurrent call for the same tree waits for the build instead of repeating it.
- stdout carries only `SOLARIS_SERVER_BINARY=<path>` (client: `SOLARIS_CLIENT_BINARY=<path>`, the variables the harness reads), so `export "$(pin-release <tree>)"` works; everything else goes to stderr.
- Pins are immutable: an existing pin is never overwritten or rebuilt, the new pin directory is renamed into place atomically, and files are read-only. It refuses a tree that is not the right repository, a build that fails or reports no executable, and a tree that changed during the build (nothing is pinned).

### In-use registry: `in-use add|remove|list`

`<work>/receipts/.in-use.jsonl` is an append-only JSONL: one line per `add`, `remove` or audit event, never rewritten; readers fold it per (path, owner) (`wtlib.in_use_fold`). It lives in `receipts/` because that is already the durable evidence store and a worktree name cannot start with a dot. A mark is `{path, owner, kind, since}`; kinds are `worktree`, `pin`, `client_root`, `loader_root`, `receipt_binary`.

- Writers: `wt-new` marks its worktree (owner = the worktree name), `pin-release` marks the pin it printed (owner = the tree that asked), and agents or harness runs call `in-use add <path> --owner <name> --kind client_root|loader_root|receipt_binary` for a client or loader checkout and for a receipt binary a gate executes, and `in-use remove <path> --owner <name>` when done. `in-use list [--json]` prints the folded marks.
- Readers: `wt-done` refuses to remove a worktree (or one holding a marked path) that another owner marked, printing the owners; `--force "<reason>"` overrides and logs a `force` line with the reason. After a removal it drops every mark on the worktree. `wt-gc` shows the marks in an `IN-USE` column and notes them next to a stale worktree.
- `slot-gc` ignores the registry: slots are caches.

### `slot-gc [--delete] [-v] [slot...]`

Dry run by default. In pooled slots `<work>/targets/slot-<l>` (or the slots named), safe cleanup runs first; capacity rotation can additionally evict idle caches when disk space is low.

- **Build units**: `deps/<stem>.d` and its matching output files are candidates only when at least one absolute source is in a removed `<work>/wt/<name>` or `/tmp/<x>` and all other absolute sources are in removed trees or the target pool. Registry, toolchain and live-tree sources prevent deletion. Profile-level binaries are not touched by this class.
- **Incremental crate dirs**: attribution requires a matching full dep-info stem, a removed-tree root source mentioned in the cache, and no detected existing-tree mention. Unattributed dirs are kept by this class.
- **Superseded sessions**: in each incremental crate dir, remove finalized sessions older than the newest (rustc's order: the base-36 timestamp in the name) and their locks; never remove `-working` sessions or a session whose `.lock` rustc holds (it takes an exclusive flock first, as rustc's own session GC does).
- **Capacity rotation**: if free space remains below `SOLARIS_SLOT_MIN_FREE_GB` + 10 GiB, visit idle slots in least-recently-used order (newest fingerprint or pool-lock mtime). Evict each slot's incremental dirs first, then its remaining contents except `tmp/` if space is still short. A slot with a held pool lock or a process using it as `CARGO_TARGET_DIR` is skipped. `--no-rotate` disables this class. Unlike safe cleanup, rotation can discard live-tree caches and profile-level binaries.
- **Locking**: with `--delete` each slot is processed under two exclusive locks: its pool lock (`/tmp/solaris-mem-slots/<l>`, taken by `mem-run.sh` for a slot it selects and by `pin-release` around its build) and its use lock (`<l>.use`, held shared by every `mem-run.sh` command whose target is that slot, including an explicit `CARGO_TARGET_DIR` and pin-release's inner build). A slot with either held is skipped and reported; a runner command that starts during a pass on its slot waits for it. The `CARGO_TARGET_DIR` process scan, repeated once the locks are held, covers cargo run outside the runner, except one that starts after that scan. A dry run only probes the locks, so it does not keep a slot from the runner while it scans.
- `-v` lists every unit and incremental dir it would remove or keep as unattributed. Check the `removed tree` lines against `git worktree list` of the four repos before the first `--delete`.

### `main-guard [--baseline]`

For Main at checkpoints; no daemon. `main-guard --baseline` writes `<work>/receipts/.main-guard-baseline.json`: every modified or untracked non-ignored file under `crates/`, `sdk/` and `tools/harness/backends/` of `<work>/solaris` with a content hash. A plain `main-guard` compares against it and prints `NEW <path>` (dirty now, not in the baseline), `CHANGED <path>` (content differs from the baseline) and `GONE <path>` (dirty then, clean now: committed or reverted); exit status 1 when anything is NEW or CHANGED. Read-only: git runs with optional locks off.

Tests: `python3 tools/dev/test_slot_gc.py`, `python3 tools/dev/test_slot_select.py` and `python3 tools/dev/test_registry.py`.
