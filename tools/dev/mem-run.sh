#!/usr/bin/env bash
# Run one heavy command (cargo build/test/clippy/check, harness run, gradle) under a
# hard memory cap inside solaris-heavy.slice (all heavy commands together <=25G).
# No slots and no admission queue (owner 2026-10-28): every command starts at once in
# its own scope and registers its cgroup, expected peak and a preemptible flag;
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
# CPUWeight=50), so harness runs keep their canonical CPU budget.
# Lanes: ordinary commands run on all CPUs, at most 12G each; every cargo under
# mem-run shares the watchdog's make-style jobserver (one token per logical CPU, via
# CARGO_MAKEFLAGS), so all builds together run about nproc compilers instead of nproc
# per build. `-p` (perf lane: A/B benches, profiles) runs one at a time on CPUs 8-11 beside other
# work and is never frozen; `-x` (quiet measurement windows only, not memory) has the
# watchdog freeze every preemptible command, waits for the rest to finish, holds new
# commands at start until it ends, and runs on all CPUs. -p and -x drop
# nice. Compare -p numbers only with other -p numbers.
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
slice=solaris-heavy.slice
slice_gib=25
# CPUQuota= (none): the slice is bounded by memory; a stray CPUQuota on it once held
# every heavy command to 2 CPUs for two days (2026-10-26..28).
systemctl --user set-property --runtime "$slice" MemoryMax=${slice_gib}G MemoryHigh=infinity MemorySwapMax=2G AllowedCPUs=0-11 CPUQuota=
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
cap_kib=$(( $(mem_mib "$mem") * 1024 ))
peak_kib=$(awk -F'\t' -v k="$key" '$1 == k { v[n++] = $2 }
  END { m = 0; for (j = (n > 5 ? n - 5 : 0); j < n; j++) if (v[j] > m) m = v[j]; print m }' \
  "$dir/peaks" 2>/dev/null || echo 0)
if [ "$peak_kib" -gt 0 ]; then est_kib=$(( peak_kib * 5 / 4 )); else est_kib=$(( 4 * 1024 * 1024 )); fi
[ "$est_kib" -gt "$cap_kib" ] && est_kib=$cap_kib
if [ "$exclusive" = 1 ]; then
  # One window at a time; while it holds this lock new commands wait at start.
  exec 3>"$dir/exclusive.lock"
  flock 3
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
  # starts meanwhile always sees this command and waits for it.
  exec 3>"$dir/exclusive.lock"
  flock -s 3
  if [ "$perf" = 1 ]; then
    exec 6>"$dir/perf"
    flock 6
  fi
fi
# Target dirs: a command that sets CARGO_TARGET_DIR (in its own `env ...` or in the
# caller's environment) keeps it and takes no pool lock. Otherwise it gets a free
# shared dir slot-a..slot-z, preferring the one its source tree (git toplevel of the
# cwd) used last; crates.io dependencies stay compiled across worktrees. Cargo
# trusts mtimes, so a tree whose files are older than another tree's build in the
# same slot would reuse that tree's binaries (a false green, seen 2026-11-04). On a
# tree switch every tracked file of the new tree is touched, so its workspace
# crates rebuild. The lock files a-e are the old slot locks, so commands of the
# previous runner version never share a dir with new ones. Never waits: with all 26
# busy the command is refused.
target=${CARGO_TARGET_DIR:-}
for w in "$@"; do [[ $w == CARGO_TARGET_DIR=* ]] && target=${w#CARGO_TARGET_DIR=}; done
if [ -z "$target" ]; then
  slots=${SOLARIS_SLOT_TARGETS:-/run/media/kaiserroman/games/solaris-work/targets}
  tree=$(git rev-parse --show-toplevel 2>/dev/null || pwd)
  slot=
  for pass in own any; do
    for l in {a..z}; do
      if [ "$pass" = own ] && [ "$(cat "$slots/slot-$l/.solaris-tree" 2>/dev/null)" != "$tree" ]; then
        continue
      fi
      exec 7>"$dir/$l"
      if flock -n 7; then slot=$l; break 2; fi
      exec 7>&-
    done
  done
  [ -n "$slot" ] || { echo "mem-run: all 26 shared target dirs busy; refusing (exit 75)" >&2; exit 75; }
  export CARGO_TARGET_DIR="$slots/slot-$slot"
  target=$CARGO_TARGET_DIR
  if [ "$(cat "$target/.solaris-tree" 2>/dev/null)" != "$tree" ]; then
    if git -C "$tree" rev-parse --git-dir >/dev/null 2>&1; then
      (cd "$tree" && git ls-files -z | xargs -0 -r touch -c --)
    fi
    mkdir -p "$target"
    printf '%s\n' "$tree" >"$target/.solaris-tree"
  fi
fi
# The harness would otherwise re-run itself in its own scope outside this slice;
# tell it it is already scoped so cargo/server/client stay under this cap.
# Incremental for debug/test builds. This cargo (1.99) does not apply the
# profile default here, so it is forced by env; release builds keep it off.
# Measured once: cold mc-core --tests build 26m48s -> 19m07s; the
# edit->rebuild gain is not yet measured.
case " $* " in
  *" --release "*|*" --profile"*) incremental=0 ;;
  *) incremental=1 ;;
esac
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-$jobs_default}" CARGO_INCREMENTAL="$incremental" SOLARIS_HARNESS_CPU_SCOPE=1
cores=$(lscpu -p=core,socket | grep -v '^#' | sort -u | wc -l)
props=(-p MemoryMax="$mem" -p MemorySwapMax=1G -p CPUQuota="${SOLARIS_MEM_RUN_CPU_QUOTA:-$((cores * 100))}%" -p CPUWeight=50)
niceness=19
cpu_pin=()
# Preemptible (the watchdog may freeze it under memory pressure) only for plain
# builds, decided once from argv: cargo build/check/clippy/doc/fetch and cargo
# test/bench --no-run. Tests, harness runs, servers and clients never are.
cmd=("${key_words[@]:$i}")
if [ "${cmd[0]:-}" = nice ]; then
  if [ "${cmd[1]:-}" = -n ]; then cmd=("${cmd[@]:3}"); else cmd=("${cmd[@]:1}"); fi
fi
preemptible=0
if [ "${cmd[0]:-}" = cargo ]; then
  sub=
  for w in "${cmd[@]:1}"; do [[ $w == [-+]* ]] || { sub=$w; break; }; done
  case "$sub" in
    build|b|check|c|clippy|doc|fetch) preemptible=1 ;;
    test|t|bench) [[ " ${cmd[*]} " == *" --no-run "* ]] && preemptible=1 ;;
  esac
fi
# Ordinary commands now share CPUs 8-11 too, so the perf lane wins them by weight:
# CPUWeight 10000 vs 50 per build scope (diagnostic isolation; quiet numbers need -x).
[ "$perf" = 1 ] && { niceness=0; cpu_pin=(taskset -c 8-11); preemptible=0; props=("${props[@]/CPUWeight=50/CPUWeight=10000}"); }
[ "$exclusive" = 1 ] && { niceness=0; cpu_pin=(); preemptible=0; }
echo "mem-run: scope ${props[*]} nice=$niceness exclusive=$exclusive perf=$perf cpus=${cpu_pin[*]:-all} slice=$slice target=$target expect=$((est_kib / 1024))MiB key='$key'" >&2
# The wrapper registers its scope, expectation, preemptible flag and target dir with
# the watchdog (the watchdog never freezes a command whose target dir another running
# command uses: cargo's build lock would chain them), releases the shared -x lock,
# runs the command without the runner's lock fds (stdin kept; TERM/INT/HUP
# forwarded), then records the scope's peak. A -x wrapper also ends the window.
# systemd-run would otherwise expand $NAME (and turn $$ into $) in the argv.
export MEMRUN_DIR="$dir" MEMRUN_KEY="$key" MEMRUN_EST_KIB="$est_kib" MEMRUN_PREEMPTIBLE="$preemptible" MEMRUN_EXCLUSIVE="$exclusive" \
  MEMRUN_TARGET="$(realpath -m "$target")"
exec systemd-run --user --scope --quiet --expand-environment=no --slice="$slice" "${props[@]}" \
  nice -n "$niceness" "${cpu_pin[@]}" bash -c '
notify() { local fd; exec {fd}<>"$MEMRUN_DIR/events"; printf "%s\n" "$1" >&"$fd"; exec {fd}>&-; }
cg=$(sed -n "s/^0:://p" /proc/self/cgroup)
printf "%s\t%s\t%s\t%s\n" "$cg" "$MEMRUN_EST_KIB" "$MEMRUN_PREEMPTIBLE" "$MEMRUN_TARGET" > "$MEMRUN_DIR/run/$$"
[ "$MEMRUN_EXCLUSIVE" = 1 ] || { flock -u 3; exec 3>&-; }
notify start
"$@" <&0 3>&- 6>&- 7>&- & c=$!
trap "kill -TERM $c 2>/dev/null" TERM INT HUP
st=0
wait "$c" || st=$?
while kill -0 "$c" 2>/dev/null; do wait "$c" || st=$?; done
# Only a complete, successful run measures the command; an early failure or a
# cancellation would teach a too-small peak.
peak=$(cat "/sys/fs/cgroup$cg/memory.peak" 2>/dev/null || echo 0)
[ "$st" = 0 ] && [ "$peak" -gt 0 ] && printf "%s\t%s\n" "$MEMRUN_KEY" $((peak / 1024)) >> "$MEMRUN_DIR/peaks"
rm -f "$MEMRUN_DIR/run/$$"
[ "$MEMRUN_EXCLUSIVE" = 1 ] && rm -f "$MEMRUN_DIR/exclusive"
notify exit
exit "$st"' mem-run "$@"
