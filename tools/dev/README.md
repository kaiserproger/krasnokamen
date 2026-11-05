# Local development tools

These Linux host tools are intended for coordinated Solaris development, not production deployment.

- `mem-run.sh` runs commands in a memory-capped systemd user scope, shares Cargo target directories and a jobserver, and starts the watchdog. `-p` serializes the performance lane; `-x` requests a quiet measurement window and needs Main's GO. On the shared host every checkout, worktree and sibling repo calls Main's copy by absolute path (`<work>/solaris/tools/dev/mem-run.sh`), never its own: mixed runner versions would share one lock/registration directory.
- `heavy-watchdog.py` monitors slice memory pressure and freezes eligible builds, then thaws them. The runner launches it as `solaris-heavy-watchdog.service`.
- `scene-slot.sh` serializes graphical scene runs through three host-wide flock slots.
- `instrumentation-guard.py -- COMMAND ...` runs trusted instrumentation under the runner and Linux Landlock, allowing writes only in a private `/dev/shm` directory. Preserve that directory while inspecting outputs.
- `test-server/` contains the local metrics collector and remote fresh-world deployment, service, and soak tools; see `test-server/README.md` before use.
- `winqa/` contains a noVNC driver; run `npm install --prefix tools/dev/winqa` once, then `node tools/dev/winqa/cs-daemon.js`, and follow `winqa/RECIPE.md`.

## Fresh machine setup

1. Clone the repository and use `tools/dev/` directly; preserve executable permissions. Install Bash, Python 3, Git, rsync, util-linux (`flock`, `taskset`, `lscpu`), systemd user services, and the development toolchain. Landlock ABI 3 or later is needed for the instrumentation guard.
2. Use cgroup v2 with a working delegated systemd user manager. The runner starts `solaris-heavy.slice`, sets its runtime memory cap to 25G, swap cap to 2G, `AllowedCPUs=0-11`, and clears its `CPUQuota`; individual scopes receive a CPU quota based on physical cores (override with `SOLARIS_MEM_RUN_CPU_QUOTA`, a percentage number). Adjust the machine's setup or the script for a different CPU topology or capacity. The `-p` lane specifically requires CPUs 8-11.
3. Ensure `/tmp/solaris-mem-slots` is writable and shared by cooperating runs; it contains locks, FIFOs, registrations and peak history. Set `SOLARIS_SLOT_TARGETS` to a writable shared target pool on a suitable disk. The default is the original host's target path. Explicit `CARGO_TARGET_DIR` bypasses slot selection.
4. The first runner invocation starts the transient `solaris-heavy-watchdog.service` from the adjacent Python file; no separately installed unit is required. Verify the user manager can start it and access the slice's cgroup files. Ordinary runs warn if it cannot start; `-x` refuses to run without it.

The original ignored `bin/` contained a host-specific `cargo-component` 0.21.1 executable, not project source. Install that version separately when needed (`cargo install cargo-component --version 0.21.1 --locked`); it is not vendored here.
