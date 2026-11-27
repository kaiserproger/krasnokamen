#!/usr/bin/env bash
# Run one heavy command (cargo build/test/clippy/check, harness run, gradle) under a
# hard memory cap inside solaris-heavy.slice (all heavy commands together <=25G).
# No memory slots, no admission queue and no build tokens (owner 2026-10-28, D127
# 2026-11-27: every command starts at once in its own scope; CPU is shared by the
# jobserver, never queued). Each scope registers its
# cgroup, expected peak and a preemptible flag;
# heavy-watchdog.py (a user service this script starts) freezes the youngest
# preemptible command only on real memory shortage (slice PSI or the slice hitting
# MemoryMax) and thaws frozen commands oldest first once commands exit and their
# expected remaining growth fits. Frozen means paused (cgroup v2 freezer), not killed.
# Preemptible is decided once from argv: plain cargo build/check/clippy/doc/fetch and
# cargo test/bench --no-run. Tests, harness runs, servers and clients are never
# frozen, so their timeouts stay honest; the oldest running command is never frozen.
# Expectations come from measured peaks: every successful command records its scope's
# memory.peak under a signature of its argv, and the next run of the same signature
# expects 1.25 x the largest of its last five peaks (unknown: 4G), never above its cap.
# The per-command MemoryMax and the slice cap still bound a misprediction: an overrun
# OOM-kills inside the slice, never the host. No MemoryHigh: throttling is the pressure
# signal systemd-oomd kills on (see tools/harness/__main__.py:465-478).
# Replace this file atomically (write + mv), never in place: running callers read it
# incrementally.
# The scope gets the harness's own CPU properties (CPUQuota = physical cores x 100%,
# CPUWeight=50), so harness runs keep their canonical CPU budget. Preemptible builds
# get CPUWeight=10: under contention tests and servers win the CPU five to one; builds
# only slow down. A graphical gate holding a scene slot (scene-slot.sh exports
# KRASNOKAMEN_SCENE_SLOT) gets CPUWeight=500: with dozens of test scopes at 50 a gate's
# client fell to 6-8 TPS and missed its tick deadlines (2026-11-11).
# Lanes: ordinary commands run on all CPUs, at most 12G each; every cargo under
# mem-run shares the watchdog's make-style jobserver (one token per logical CPU, via
# CARGO_MAKEFLAGS), so all builds together run about nproc compilers instead of nproc
# per build. `-p` (perf lane: A/B benches, profiles) runs one at a time on the last four CPUs beside other
# work and is never frozen; `-x` (quiet measurement windows only, not memory) has the
# watchdog freeze every preemptible command, waits for the rest to finish, holds new
# commands at start until it ends, and runs on all CPUs. -p and -x drop
# nice. Compare -p numbers only with other -p numbers.
# Release builds (`cargo build|test|run --release`, `--profile release*`) are refused
# unless the caller is tools/dev/pin-release, the harness/ledger tooling (real-client
# gates and ledger promote's own candidate and control trees) or Main explicitly
# (KRASNOKAMEN_RELEASE_BUILD_ALLOW=1): a release binary belongs to one integrated
# commit and comes from pin-release, which reuses the pin of the same tree and gives
# a different tree its own. `-x` is a scheduled measurement window: outside
# tools/dev/measurement-windows it refuses and names the next window (Main only:
# KRASNOKAMEN_WINDOW_OVERRIDE=1). Debug builds and ordinary tasks are unchanged.
# Usage: mem-run.sh [-x] [-p] [-m 8G] command args...
set -euo pipefail
trap 'echo "mem-run: failed (exit $?) at line $LINENO" >&2' ERR
mem=8G
exclusive=0
perf=0
if [ "${1:-}" = "-x" ]; then exclusive=1; shift; fi
if [ "${1:-}" = "-p" ]; then perf=1; shift; fi
if [ "${1:-}" = "-m" ]; then mem=$2; shift 2; fi
mem_mib() {
  local v=${1%[KkMmGg]} u=${1: -1}
  case "$u" in
    G|g) echo $(( v * 1024 ));;
    M|m) echo "$v";;
    K|k) echo $(( v / 1024 ));;
    *) echo $(( $1 / 1024 / 1024 ));;
  esac
}
if [ "$exclusive" = 0 ] && [ "$(mem_mib "$mem")" -gt 12288 ]; then
  echo "mem-run: per-command cap is 12G; requested $mem lowered to 12G (use -x for more)" >&2
  mem=12G
fi
dir=/tmp/solaris-mem-slots
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
mkdir -p "$dir/run"
# Release builds are pins (owner 2026-11-15): tools/dev/release-build-policy.sh refuses
# `cargo build|test|run --release` and `--profile release*` unless tools/dev/pin-release,
# the harness/ledger tooling or Main's override asks for one. Before any slot.
# shellcheck source=release-build-policy.sh
source "$here/release-build-policy.sh"
release_build_guard "$@"
# -x is a quiet measurement window, not a flag: it pauses builds and waits for every
# running test and server, so it opens only inside a scheduled window
# (tools/dev/measurement-windows; the refusal names the next one). Main only:
# KRASNOKAMEN_WINDOW_OVERRIDE=1.
if [ "$exclusive" = 1 ]; then
  if [ -n "${KRASNOKAMEN_WINDOW_OVERRIDE:-}" ]; then
    echo "mem-run: -x: measurement-window schedule overridden (KRASNOKAMEN_WINDOW_OVERRIDE, Main only)" >&2
  elif ! python3 "$here/measurement-window.py" --config "${KRASNOKAMEN_MEASUREMENT_WINDOWS:-$here/measurement-windows}"; then
    exit 75
  fi
fi
# Signature: argv without a leading `env` (its -u NAME / -flag / VAR=value words)
# and without path-like words, so reruns of one command in different worktrees share
# their measured peaks while different packages/filters stay apart.
key_words=("$@")
i=0
if [ "${key_words[0]:-}" = env ]; then
  i=1
  while [ "$i" -lt "${#key_words[@]}" ]; do
    w=${key_words[$i]}
    if [ "$w" = -u ]; then i=$((i + 2))
    elif [[ $w == *=* || ( $w == -* && $w != -- ) ]]; then i=$((i + 1))
    else break; fi
  done
fi
key=
for w in "${key_words[@]:$i}"; do
  [[ $w == */* ]] && continue
  key+="${key:+ }$w"
done
cmd=("${key_words[@]:$i}")
if [ "${cmd[0]:-}" = nice ]; then
  if [ "${cmd[1]:-}" = -n ]; then cmd=("${cmd[@]:3}"); else cmd=("${cmd[@]:1}"); fi
fi
# A bad cancel grace must fail in a second, not after an -x freeze (N12).
_grace=${KRASNOKAMEN_MEMRUN_CANCEL_GRACE:-15}
if ! awk -v g="$_grace" 'BEGIN { exit !(g ~ /^[0-9]+([.][0-9]+)?$/ && g + 0 > 0 && g + 0 <= 86400) }'; then
  echo "mem-run: KRASNOKAMEN_MEMRUN_CANCEL_GRACE must be a finite positive number of seconds no larger than 86400, got '$_grace'" >&2
  exit 75
fi
# Owner D127 (2026-11-27) removed build tokens entirely: no compile/gate/run admission
# throttles remain. Every command starts at once; the shared jobserver (one token per
# logical CPU) shares cores without queueing, and each scope's MemoryMax plus
# heavy-watchdog bound memory. tools/dev/fleet-status shows what is running.
slice=solaris-heavy.slice
# Capacity is derived from the host (owner host since 2026-11-10: Aeza, 16 CPUs, 90 GiB):
# the slice gets all RAM but 16 GiB kept for agents and the system, and every CPU, but
# stays 8 GiB under the parent solaris.slice MemoryHigh when the owner set one (Aeza
# 2026-11-11: 72G high / 80G max, 1400% CPU). The agents themselves (omp, scope
# solaris-agents.scope) run in that parent too, so the heavy cap OOM-kills inside the
# slice before the parent throttles the agents and every heavy command at once.
ncpu=$(nproc --all)
total_gib=$(awk '/MemTotal/{print int($2/1048576)}' /proc/meminfo)
slice_gib=$(( total_gib - 16 ))
# A small host (a 16 GiB CI runner) keeps half its memory, never a negative cap.
[ "$slice_gib" -ge $(( total_gib / 2 )) ] || slice_gib=$(( total_gib / 2 ))
parent_high=$(systemctl --user show solaris.slice -p MemoryHigh --value 2>/dev/null || true)
if [[ "$parent_high" =~ ^[0-9]+$ ]] && [ $(( parent_high / 1073741824 - 8 )) -lt "$slice_gib" ]; then
  slice_gib=$(( parent_high / 1073741824 - 8 ))
fi
perf_cpus="$((ncpu - 4))-$((ncpu - 1))"
# The slice is bounded by memory. Its CPUQuota is cleared on every call unless the
# owner pins one in $dir/cpu-quota (e.g. "600%"): a stray CPUQuota once held every
# heavy command to 2 CPUs for two days (2026-10-26..28), so only that file sets it.
slice_quota=
if [ -r "$dir/cpu-quota" ]; then
  slice_quota=$(tr -d '[:space:]' < "$dir/cpu-quota")
  [[ "$slice_quota" =~ ^[1-9][0-9]*%$ ]] || { echo "mem-run: $dir/cpu-quota must be like 600%, got '$slice_quota'" >&2; exit 64; }
fi
systemctl --user set-property --runtime "$slice" MemoryMax=${slice_gib}G MemoryHigh=infinity MemorySwapMax=2G AllowedCPUs=0-$((ncpu - 1)) CPUQuota=$slice_quota
# After a reboot the slice is inactive and has no cgroup: start it, and fail closed
# if it still has none (an empty path would account the root cgroup's scopes).
systemctl --user start "$slice"
cgroup=$(systemctl --user show "$slice" -p ControlGroup --value)
[ -n "$cgroup" ] || { echo "mem-run: $slice has no cgroup; refusing" >&2; exit 75; }
[ -p "$dir/events" ] || mkfifo "$dir/events" 2>/dev/null || [ -p "$dir/events" ]
# Without the watchdog nothing is frozen (the slice cap still bounds memory): ordinary
# commands run with a warning, -x refuses (nobody would answer its quiet wait).
if ! systemctl --user is-active -q solaris-heavy-watchdog.service; then
  systemctl --user reset-failed solaris-heavy-watchdog.service 2>/dev/null || true
  systemd-run --user --quiet --unit=solaris-heavy-watchdog -p Restart=on-failure \
    python3 "$here/heavy-watchdog.py" "$dir" "/sys/fs/cgroup$cgroup" 2>/dev/null || true
  if ! systemctl --user is-active -q solaris-heavy-watchdog.service; then
    [ "$exclusive" = 1 ] && { echo "mem-run: solaris-heavy-watchdog.service is not running; -x refused" >&2; exit 75; }
    echo "mem-run: WARNING: solaris-heavy-watchdog.service is not running; nothing will be frozen under memory pressure" >&2
  fi
fi
jobs_default=5
if systemctl --user is-active -q solaris-heavy-watchdog.service && [ -p "$dir/jobserver" ]; then
  export CARGO_MAKEFLAGS="-j --jobserver-auth=fifo:$dir/jobserver"
  jobs_default=$(nproc)
fi
# Wake the watchdog. Opened read-write: never blocks, even if it is restarting.
notify() { local fd; exec {fd}<>"$dir/events"; printf '%s\n' "$1" >&"$fd"; exec {fd}>&-; }
cap_kib=$(( $(mem_mib "$mem") * 1024 ))
peak_kib=$(awk -F'\t' -v k="$key" '$1 == k { v[n++] = $2 }
  END { m = 0; for (j = (n > 5 ? n - 5 : 0); j < n; j++) if (v[j] > m) m = v[j]; print m }' \
  "$dir/peaks" 2>/dev/null || echo 0)
if [ "$peak_kib" -gt 0 ]; then est_kib=$(( peak_kib * 5 / 4 )); else est_kib=$(( 4 * 1024 * 1024 )); fi
[ "$est_kib" -gt "$cap_kib" ] && est_kib=$cap_kib
nested_scope() {  # is this runner inside a running mem-run scope (its wrapper is registered in $dir/run)?
  local p=$PPID
  while [ "${p:-0}" -gt 1 ]; do
    [ -e "$dir/run/$p" ] && return 0
    p=$(awk '$1 == "PPid:" {print $2}' "/proc/$p/status" 2>/dev/null) || p=0
  done
  return 1
}
nested=0
[ "$exclusive" = 0 ] && nested_scope && nested=1
if [ "$exclusive" = 1 ]; then
  # One window at a time; while it holds this lock new commands wait at start.
  exec 3>"$dir/exclusive.lock"
  flock 3 9>&-
  # Lock order: a scene slot (scene-slot.sh) is taken before this lock, never inside the
  # window. Gates hold scene slots while they wait for this lock (shared), so a window
  # command waiting for a slot deadlocked the whole fleet for ~50 min (2026-11-14).
  # Descendants see the marker; scene-slot.sh then only reuses an inherited slot.
  export KRASNOKAMEN_EXCLUSIVE_WINDOW=$$
  # Opened before the watchdog is told, so its "quiet" line cannot be lost.
  [ -p "$dir/quiet" ] || { rm -f "$dir/quiet"; mkfifo "$dir/quiet"; }
  exec 4<>"$dir/quiet"
  echo $$ > "$dir/exclusive"
  echo "mem-run: -x: freezing build-only commands, waiting for the others to finish" >&2
  notify exclusive
  read -r -u 4 _
  exec 4>&-
else
  # Held (shared) until the wrapper has registered its scope, so a -x window that
  # starts meanwhile always sees this command and waits for it. A command nested in a
  # running scope (a ledger promote's builds and gates) does not wait: the window
  # already waits for its ancestor, so waiting here deadlocked both (2026-11-12).
  # The lock fd is not inherited by the flock child (9>&-): a runner killed while
  # waiting must not leave an orphaned `flock -s 3` holding the -x window open.
  exec 3>"$dir/exclusive.lock"
  [ "$nested" = 1 ] || flock -s 3 9>&-
  if [ "$perf" = 1 ]; then
    exec 6>"$dir/perf"
    flock 6 9>&-
  fi
fi
# Disk exhaustion guard (owner D127, 2026-11-27): free space is a safety limit, not a
# launch budget. The 250 GiB floor is gone; the owner keeps >=280 GiB free by
# *cleaning*, never by blocking runs. Automatic reclaim runs below
# KRASNOKAMEN_DISK_GC_BELOW_GB (default 300): an ordinary command first asks slot-gc to
# rotate idle slots, drop superseded incremental sessions and dead units, then
# rechecks. slot-gc takes each slot's pool lock itself (busy slots are skipped) and
# never delays the command by more than KRASNOKAMEN_SLOT_GC_TIMEOUT seconds (default
# 120), and never fails it: an error is printed and the command starts anyway. Not run
# for -x and -p, whose windows are quiet measurements. The only refusal is the
# last-resort exhaustion guard below KRASNOKAMEN_DISK_REFUSE_BELOW_GB (default 20):
# there world/WAL writes could fail, so the command does not start (exit 75).
gc_root=${KRASNOKAMEN_SLOT_TARGETS:-/run/media/kaiserroman/games/solaris-work/targets}
gc_below=${KRASNOKAMEN_DISK_GC_BELOW_GB:-300}
refuse_below=${KRASNOKAMEN_DISK_REFUSE_BELOW_GB:-20}
disk_free() { df --output=avail -BG "$gc_root" 2>/dev/null | tail -1 | tr -dc 0-9; }
for v in "$gc_below" "$refuse_below"; do
  case $v in
    ''|*[!0-9]*) echo "mem-run: disk guard thresholds must be whole GiB, got gc='$gc_below' refuse='$refuse_below'" >&2; exit 75 ;;
  esac
done
gc_free=$(disk_free)
if [ -n "$gc_free" ] && [ "$gc_free" -lt "$refuse_below" ]; then
  echo "mem-run: ${gc_free}G free on $gc_root, below the ${refuse_below}G exhaustion guard (owner D127); refusing to start (world/WAL writes would fail: free space first with slot-gc --delete, or reports cleanup)" >&2
  exit 75
fi
if [ -n "$gc_free" ] && [ "$exclusive" = 0 ] && [ "$perf" = 0 ] && [ "$gc_free" -lt "$gc_below" ]; then
  echo "mem-run: ${gc_free}G free, below ${gc_below}G: running slot-gc --delete on idle slots" >&2
  gc_target=$(( gc_below > 10 ? gc_below - 10 : 0 ))
  if gc_out=$(timeout "${KRASNOKAMEN_SLOT_GC_TIMEOUT:-120}" "$here/slot-gc" --delete --min-free-gb "$gc_target" --lock-dir "$dir" 3>&- 6>&- 9>&- 2>&1); then
    printf '%s\n' "$gc_out" | sed 's/^/mem-run: slot-gc: /' >&2
  else
    gc_st=$?
    printf '%s\n' "$gc_out" | sed 's/^/mem-run: slot-gc: /' >&2
    echo "mem-run: slot-gc failed (exit $gc_st); continuing" >&2
  fi
  gc_free=$(disk_free)
  if [ -n "$gc_free" ] && [ "$gc_free" -lt "$refuse_below" ]; then
    echo "mem-run: ${gc_free}G free on $gc_root after slot-gc, below the ${refuse_below}G exhaustion guard; refusing to start" >&2
    exit 75
  fi
fi
# Target dir and its slot locks: see slot-select.sh (ownership of pooled slots,
# tree switches, the use lock slot-gc respects).
# shellcheck source=slot-select.sh
source "$here/slot-select.sh"
select_slot "$@"
# Incremental: debug/test builds always; release builds only in the normal lane
# (a re-pin after a small change should cost less CPU: one uncontrolled 2026-11-11
# measurement, size unproven; the binary has more codegen units and can run slower,
# so perf windows use none). -p and -x
# lanes and custom `--profile` builds are never incremental. This cargo (1.99) does
# not apply the profile default here, so it is forced by env. Measured once: cold
# mc-core --tests build 26m48s -> 19m07s.
if [ "$exclusive" = 1 ] || [ "$perf" = 1 ]; then
  incremental=0
else
  case " $* " in
    *" --profile"*) incremental=0 ;;
    *) incremental=1 ;;
  esac
fi
# sccache through tools/dev/sccache.sh while the shared cache daemon runs (never plain
# `RUSTC_WRAPPER=sccache`: it hashes the per-slot CARGO_TARGET_DIR into every key and refuses
# CARGO_INCREMENTAL=1; see docs/development/heavy-commands-and-memory.md). The daemon only
# fronts the cache (client_side_mode): compiles run here, inside this scope. A caller's own
# `env RUSTC_WRAPPER=` (old briefs) still wins, uncached.
sccache_unit=solaris-sccache.service
sccache_bin="$HOME/.cargo/bin/sccache"
if [ -x "$sccache_bin" ] && [ -x "$here/sccache.sh" ] && systemctl --user is-active -q "$sccache_unit"; then
  export RUSTC_WRAPPER="$here/sccache.sh" SCCACHE_CLIENT_SIDE=1
else
  export RUSTC_WRAPPER=
fi
# The harness would otherwise re-run itself in its own scope outside this slice;
# tell it it is already scoped so cargo/server/client stay under this cap.
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-$jobs_default}" CARGO_INCREMENTAL="$incremental" KRASNOKAMEN_HARNESS_CPU_SCOPE=1
cores=$(lscpu -p=core,socket | grep -v '^#' | sort -u | wc -l)
props=(-p MemoryMax="$mem" -p MemorySwapMax=1G -p CPUQuota="${KRASNOKAMEN_MEM_RUN_CPU_QUOTA:-$((cores * 100))}%" -p CPUWeight=50)
niceness=19
cpu_pin=()
# Preemptible (the watchdog may freeze it under memory pressure) only for plain
# builds, decided once from argv: cargo build/check/clippy/doc/fetch and cargo
# test/bench --no-run. Tests, harness runs, servers and clients never are.
preemptible=0
if [ "${cmd[0]:-}" = cargo ]; then
  sub=
  for w in "${cmd[@]:1}"; do [[ $w == [-+]* ]] || { sub=$w; break; }; done
  case "$sub" in
    build|b|check|c|clippy|doc|fetch) preemptible=1 ;;
    test|t|bench) [[ " ${cmd[*]} " == *" --no-run "* ]] && preemptible=1 ;;
  esac
fi
# A nested build is part of its running ancestor (a promote's server build): freezing it
# for a -x window would stall the ancestor the window waits for.
[ "$nested" = 1 ] && preemptible=0
# Ordinary commands share the perf CPUs too, so the perf lane wins them by weight:
# CPUWeight 10000 vs 50 per build scope (diagnostic isolation; quiet numbers need -x).
[ "$perf" = 1 ] && { niceness=0; cpu_pin=(taskset -c "$perf_cpus"); preemptible=0; props=("${props[@]/CPUWeight=50/CPUWeight=10000}"); }
[ "$exclusive" = 1 ] && { niceness=0; cpu_pin=(); preemptible=0; }
# CPUs held by tools/dev/reserve-cpus (owner 2026-11-16, Spain trials) stay out of every
# new scope except one started with KRASNOKAMEN_RESERVED_CPUS=use; running scopes keep
# their placement, and the reservation ends with the command that holds it. A reservation
# whose owner died (SIGKILL, OOM, a wrapper kill) is a corpse: say so and clear it, never
# silently pin every later scope away from those CPUs (or drop the restriction when the
# stale list covers a whole pool) — #G4, CS341. A one-line file from an older writer has no
# owner and is honoured as-is (mixed-version compatibility).
reserved=
if [ -e "$dir/reserved-cpus" ]; then
  # A failing helper must not silently drop a live reservation: name it, keep its stderr in a
  # variable (a shared .err file races between wrappers), validate the first line as a CPU list and
  # honour that much (ReviewD123Harness1 6, 4 N12).
  err=
  errlog=$(mktemp "${TMPDIR:-/tmp}/mem-run-reserved.XXXXXX")
  if ! verdict=$(python3 "$here/reserved_cpus.py" verdict "$dir" 2>"$errlog"); then
    err=$(cat "$errlog")
    rm -f "$errlog"
    first=$(head -n 1 "$dir/reserved-cpus")
    if ! printf '%s' "$first" | grep -Eqx '[0-9]+(-[0-9]+)?(,[0-9]+(-[0-9]+)?)*'; then
      echo "mem-run: cannot judge $dir/reserved-cpus ($err) and its first line is not a CPU list; refusing this scope" >&2
      exit 75
    fi
    echo "mem-run: cannot judge $dir/reserved-cpus; honouring its first line '$first' as a CPU list: $err" >&2
    verdict="keep $first"
  else
    err=$(cat "$errlog")
    rm -f "$errlog"
    [ -n "$err" ] && echo "mem-run: reserved-cpus verdict said: $err" >&2
  fi
  case "$verdict" in
    keep\ *) reserved=${verdict#keep } ;;
    stale\ *)
      echo "mem-run: ignoring stale CPU reservation '${verdict#stale }' (its owner is gone or its pid was reused); clearing $dir/reserved-cpus" >&2
      python3 "$here/reserved_cpus.py" clear "$dir" >&2 || true
      # Clearing re-reads under the lock, so it never removes a reservation written meanwhile:
      # if one is there now, it is live and this scope honours it.
      verdict=$(python3 "$here/reserved_cpus.py" verdict "$dir" 2>/dev/null || echo none)
      [ "${verdict%% *}" = keep ] && reserved=${verdict#keep }
      ;;
    none) : ;;
    *)
      echo "mem-run: reserved-cpus verdict '$verdict' is not none/keep/stale; refusing this scope" >&2
      exit 75
      ;;
  esac
fi
if [ -n "$reserved" ] && [ "${KRASNOKAMEN_RESERVED_CPUS:-}" != use ] && [ "$exclusive" = 0 ]; then
  declare -A held=()
  for part in ${reserved//,/ }; do for ((c = ${part%-*}; c <= ${part#*-}; c++)); do held[$c]=1; done; done
  pool=0-$((ncpu - 1)); [ "$perf" = 1 ] && pool=$perf_cpus
  allowed=
  for part in ${pool//,/ }; do for ((c = ${part%-*}; c <= ${part#*-}; c++)); do [ -n "${held[$c]:-}" ] || allowed+=${allowed:+,}$c; done; done
  if [ -n "$allowed" ]; then
    cpu_pin=(taskset -c "$allowed")
    echo "mem-run: reserved CPUs $reserved are excluded from this scope (cpus=$allowed)" >&2
  else
    echo "mem-run: reserved CPUs $reserved cover this whole pool; the reservation cannot restrict it, so the scope keeps cpus=$pool" >&2
  fi
fi
[ "$preemptible" = 1 ] && props=("${props[@]/CPUWeight=50/CPUWeight=10}")
[ -n "${KRASNOKAMEN_SCENE_SLOT:-}" ] && [ "$perf" = 0 ] && props=("${props[@]/CPUWeight=*/CPUWeight=500}")
echo "mem-run: scope ${props[*]} nice=$niceness exclusive=$exclusive perf=$perf cpus=${cpu_pin[*]:-all} slice=$slice target=$target expect=$((est_kib / 1024))MiB key='$key'" >&2
# The wrapper registers its scope, expectation, preemptible flag and target dir with
# the watchdog (the watchdog never freezes a command whose target dir another running
# command uses: cargo's build lock would chain them), releases the shared -x lock,
# runs the command without the runner's lock fds (stdin kept; TERM/INT/HUP
# forwarded), then records the scope's peak. A -x wrapper also ends the window.
# The guardian is a separate user service, outside both the scope and the job's
# process tree, so a job kill cannot kill it before it tears the scope down.
guard_dir=$(mktemp -d "${TMPDIR:-/tmp}/solaris-mem-guard.XXXXXXXX")
unit="krasnokamen-mem-run-${guard_dir##*.}.scope"
# The grace was validated next to argument parsing; pass it on unchanged.
grace=$_grace
mkfifo "$guard_dir/events" "$guard_dir/ready"
if ! systemd-run --user --quiet --collect --unit="${unit%.scope}-guard" \
  python3 "$here/mem-run-guard.py" "$guard_dir" "/sys/fs/cgroup$cgroup/$unit" "$$" "$PPID" \
  "$grace" \
  || ! timeout 10 bash -c 'read -r ready < "$1/ready"' _ "$guard_dir"; then
  rm -rf "$guard_dir"
  echo "mem-run: failed to start scope guardian" >&2
  exit 75
fi
if ! python3 "$here/mem-run-guard.py" --send-locks "$guard_dir"; then
  echo "mem-run: failed to transfer locks to scope guardian" >&2
  exit 75
fi
export MEMRUN_GUARD_DIR="$guard_dir"
# systemd-run would otherwise expand $NAME (and turn $$ into $) in the argv.
export MEMRUN_DIR="$dir" MEMRUN_KEY="$key" MEMRUN_EST_KIB="$est_kib" MEMRUN_PREEMPTIBLE="$preemptible" MEMRUN_EXCLUSIVE="$exclusive" \
  MEMRUN_TARGET="$(realpath -m "$target")"
exec systemd-run --user --scope --quiet --unit="$unit" --expand-environment=no --slice="$slice" "${props[@]}" \
  nice -n "$niceness" "${cpu_pin[@]}" bash -c '
notify() { local fd; exec {fd}<>"$MEMRUN_DIR/events"; printf "%s\n" "$1" >&"$fd"; exec {fd}>&-; }
cg=$(sed -n "s/^0:://p" /proc/self/cgroup)
[ -w "/sys/fs/cgroup$cg/cgroup.kill" ] || { echo "mem-run: scope cgroup.kill is not writable" >&2; exit 75; }
printf "%s\t%s\t%s\t%s\n" "$cg" "$MEMRUN_EST_KIB" "$MEMRUN_PREEMPTIBLE" "$MEMRUN_TARGET" > "$MEMRUN_DIR/run/$$"
[ "$MEMRUN_EXCLUSIVE" = 1 ] || { flock -u 3; exec 3>&-; }
notify start
"$@" <&0 3>&- 6>&- 7>&- 8>&- 9>&- & c=$!
# A cancellation must reach the whole scope, not only the direct child: a descendant that ignores
# TERM would otherwise hold the wrapper in `wait` and the scope alive until
# a SIGKILL (#G5, CS341). Forward TERM to every process in the scope cgroup and ask the guardian to
# escalate to cgroup.kill after its grace, so the wrapper always ends.
cancel() {
  # One forwarding pass: the scope members get this TERM too, so re-entering the trap would
  # recurse; a later signal is already covered by the guardian escalation.
  [ "${cancel_done:-0}" = 1 ] && return
  cancel_done=1
  # The direct child first, unconditionally: a failing cgroup.procs read (or an unusable
  # guardian fifo) must never leave the wrapper in `wait` with nothing signalled (#G5, CS341).
  kill -TERM "$c" 2>/dev/null || true
  # The guardian owns escalation: tell it (its own fifo, not the watchdog events stream).
  local fd
  if exec {fd}<>"$MEMRUN_GUARD_DIR/events"; then
    printf "cancel\n" >&"$fd" ||
      echo "mem-run: cancel: guardian fifo write failed; the scope TERM still applies, no cgroup.kill escalation" >&2
    exec {fd}>&-
  else
    echo "mem-run: cancel: cannot open $MEMRUN_GUARD_DIR/events; the guardian cannot escalate to cgroup.kill" >&2
  fi
  local pid
  for pid in $(cat "/sys/fs/cgroup$cg/cgroup.procs" 2>/dev/null); do
    # The direct child got its TERM above. A second one can land inside its cleanup (the harness turns
    # every TERM into KeyboardInterrupt) and cut a result.json write, so skip it and this wrapper.
    if [ "$pid" = "$$" ] || [ "$pid" = "$c" ]; then continue; fi
    kill -TERM "$pid" 2>/dev/null || true
  done
}
trap cancel TERM INT HUP
st=0
wait "$c" || st=$?
# A cancel trap can outlive the child: bash reaped it before this wait, so the wait reports
# 128+signal instead of the status the child exited with. Bash keeps the last reaped status, so asking
# for it again recovers it - but only for such a status, and a 127 ("no such job") is never accepted.
if [ "${cancel_done:-0}" = 1 ] && [ "$st" -ge 128 ]; then
  saved=0
  wait "$c" || saved=$?
  [ "$saved" != 127 ] && st=$saved
fi
while kill -0 "$c" 2>/dev/null; do wait "$c" || st=$?; done
# Only a complete, successful run measures the command; an early failure or a
# cancellation would teach a too-small peak.
peak=$(cat "/sys/fs/cgroup$cg/memory.peak" 2>/dev/null || echo 0)
[ "$st" = 0 ] && [ "$peak" -gt 0 ] && printf "%s\t%s\n" "$MEMRUN_KEY" $((peak / 1024)) >> "$MEMRUN_DIR/peaks"
rm -f "$MEMRUN_DIR/run/$$"
[ "$MEMRUN_EXCLUSIVE" = 1 ] && rm -f "$MEMRUN_DIR/exclusive"
notify exit
exec {guard_fd}<>"$MEMRUN_GUARD_DIR/events"
printf "done\n" >&"$guard_fd"
exec {guard_fd}>&-
exit "$st"' mem-run "$@"
