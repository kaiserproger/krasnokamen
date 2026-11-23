#!/usr/bin/env bash
# perf-window.sh <pid> [seconds] [outdir] — one maintained perf capture of a running server.
#
# Maintained home for the box recipe that used to live only in receipts/Perf1M/perf-window.sh
# (PERF-WINDOW-HAS-NO-CALL-GRAPH-AND-NO-MAINTAINED-SCRIPT) with the AMD last-level events instead
# of the generic L2 aliases (PERF-WINDOW-READS-AMD-L2-ALIAS-AS-LAST-LEVEL-MISS-RATE).
#
# One fixed counter set and three sequential windows of `seconds` each (a capture therefore takes
# about three times the requested duration: the windows never overlap, so a statistic is never
# counted over another window's warm-up):
#   1. perf stat -M PipelineL1 --per-thread  -> stat-topdown.txt (the top-down verdict);
#   2. perf stat -x, over the explicit counter set -> stat.csv. `-x,` writes CSV: with --per-thread
#      a line is comm,pid,unit,event,value,percentage,metric, so field 6 is the enabled/running
#      percentage (field 5 without --per-thread) — a multiplexed counter shows below 100 there and
#      must not be quoted as an exact count;
#   3. perf record -g --call-graph fp --per-thread over the same counters -> perf.data, then
#      perf report --stdio -g "graph,0.5,caller,0.5" (inclusive callers), a self-time report and
#      perf script (flame-graph input) -> callgraph.txt, self.txt, script.txt.
#
# `--per-thread` record maps one buffer per thread per event, so the default 8 MiB RLIMIT_MEMLOCK of
# a user session is exhausted by a many-threaded server (verified: record refused with a memlock
# error and no `-m`, succeeded with `-m 16`). The buffer is capped at PERF_WINDOW_MMAP_PAGES pages
# (default 16) instead; raise it together with `ulimit -l` if a capture reports lost samples.
#
# The server must come from a pin-release built with RUSTFLAGS="-C force-frame-pointers=yes":
# without frame pointers the sampled stacks stop at the leaf, so callgraph.txt has no caller
# columns and the capture cannot say which caller causes the fills. Such a capture is not
# evidence; rerun against a frame-pointer pin rather than quoting self time as attribution.
#
# Refusals (exit 2, nothing captured): a missing pid, a pid that is not running, a non-Linux host,
# no perf, or any of the three perf commands exiting non-zero (see its own log in <outdir>).
set -euo pipefail

usage() { echo "usage: perf-window.sh <pid> [seconds] [outdir]" >&2; exit 2; }
[ $# -ge 1 ] || usage
pid=$1
seconds=${2:-60}
out=${3:-.analysis/perf-window-$(date -u +%Y%m%dT%H%M%SZ)}
[[ $pid =~ ^[0-9]+$ ]] || { echo "perf-window: '$pid' is not a pid" >&2; exit 2; }
[[ $seconds =~ ^[0-9]+$ ]] && [ "$seconds" -ge 1 ] || { echo "perf-window: seconds must be a positive integer" >&2; exit 2; }
[ -d "/proc/$pid" ] || { echo "perf-window: no process $pid" >&2; exit 2; }
[ "$(uname -s)" = Linux ] || { echo "perf-window: Linux only" >&2; exit 2; }
command -v perf >/dev/null || { echo "perf-window: perf not found" >&2; exit 2; }

# The explicit AMD events first: `cache-references`/`cache-misses` alias to L2 on AMD64 and must
# never be quoted as last-level numbers again. The names are the ones this host (AMD, kernel 7.x)
# advertises: `perf stat -e l2_misses.all -- true` refuses it as unknown, while
# `l2_cache_req_stat.ls_rd_blk_c` is `data cache request miss in L2` (perf list). The LLC number is
# `ls_dmnd_fills_from_sys.all`; the two L2 counters are context for it, never a substitute.
events=(ls_dmnd_fills_from_sys.all l2_request_g1.all l2_cache_req_stat.ls_rd_blk_c cycles instructions)
mmap_pages=${PERF_WINDOW_MMAP_PAGES:-16}
mkdir -p "$out"

{
  echo "# perf-window $(date -u +%Y-%m-%dT%H:%M:%SZ) pid=$pid seconds=$seconds (three sequential windows)"
  echo "# events: ${events[*]} mmap_pages=$mmap_pages"
  echo "# command: $0 $*"
  echo "# kernel: $(uname -r)"
  perf --version 2>/dev/null || true
  echo "# requirement: server built with RUSTFLAGS=-C force-frame-pointers=yes (caller columns)"
} > "$out/header.txt"

# 1. Top-down verdict first (cheap), 2. the explicit counters, 3. the sampled call graph.
perf stat -M PipelineL1 --per-thread -p "$pid" -- sleep "$seconds" > "$out/stat-topdown.txt" 2>&1 || {
  echo "perf-window: perf stat -M PipelineL1 failed (the host must advertise PipelineL1); see $out/stat-topdown.txt" >&2
  exit 2
}
perf stat -x, -e "$(IFS=,; echo "${events[*]}")" --per-thread -p "$pid" -- sleep "$seconds" > "$out/stat.csv" 2>&1 || {
  echo "perf-window: perf stat over the explicit counters failed; see $out/stat.csv" >&2
  exit 2
}
perf record -p "$pid" -F 999 -g --call-graph fp --per-thread -m "$mmap_pages" \
  -e "$(IFS=,; echo "${events[*]}")" -o "$out/perf.data" -- sleep "$seconds" > "$out/record.log" 2>&1 || {
  echo "perf-window: perf record failed; see $out/record.log" >&2
  exit 2
}

# 3. Inclusive callers, self time, and the raw stream for flame graphs.
perf report -i "$out/perf.data" --stdio --no-children -g "graph,0.5,caller,0.5" > "$out/callgraph.txt" 2>&1 || {
  echo "perf-window: perf report (callers) failed; see $out/callgraph.txt" >&2
  exit 2
}
perf report -i "$out/perf.data" --stdio --no-children > "$out/self.txt" 2>&1 || {
  echo "perf-window: perf report (self) failed; see $out/self.txt" >&2
  exit 2
}
perf script -i "$out/perf.data" > "$out/script.txt" 2>&1 || {
  echo "perf-window: perf script failed; see $out/script.txt" >&2
  exit 2
}
# A self-time-only capture cannot say which caller causes the fills: a graph report with children
# prints caller lines as `            |`-prefixed branches and `            --<percent>--` leaves.
# The pattern is checked against real captures (receipts/G2/laptop-perf-20k/report-{fp,dwarf}.txt
# have callers; the flat BattleBaseline reports do not).
if ! grep -qE '^ +(\||-{2}[0-9])' "$out/callgraph.txt"; then
  echo "perf-window: no caller frames in callgraph.txt (rebuild the server with RUSTFLAGS=-C force-frame-pointers=yes and rerun); files in $out" >&2
  exit 2
fi
echo "perf-window: capture complete: $out" >&2
