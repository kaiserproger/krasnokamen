# PERF-WINDOW-HAS-NO-CALL-GRAPH-AND-NO-MAINTAINED-SCRIPT — patch mode

`fix_commit = patch:8357c04c41b86359bf919086585438d884314539d34005f5d886d6cefe90e4b7`

Derived from `129228b11b`: perf-window.sh runs perf record with -g --call-graph fp --per-thread. The patch is committed as
`docs/bugs/fix-patches/PERF-WINDOW-HAS-NO-CALL-GRAPH-AND-NO-MAINTAINED-SCRIPT.patch` (the row digest is `patch:8357c04c41b86359bf919086585438d884314539d34005f5d886d6cefe90e4b7`;
`sha256` of the file: `8357c04c41b86359bf919086585438d884314539d34005f5d886d6cefe90e4b7`).

## Hunks the patch carries

| file | +start | +count | hunk |
|---|---|---|---|
| tools/dev/perf-window.sh | 69 | 7 | perf stat -x, -e "$(IFS=,; echo "${events[*]}")" --per-thread -p "$pid" -- sleep |

## Original hunks left out

None beyond the fix commit's own test-file hunks (test files are never part of a fix patch).
The patch is the complete non-test hunk set of the behaviour the row names; nothing is omitted
for another row, for API a kept test calls, or for a later rewrite.

## Proving ref and line

- `python:tools.dev.test_perf_window:PerfWindowRecipeTests.test_the_capture_is_a_frame_pointer_call_graph_with_per_thread_attribution` — red at `/run/media/kaiserroman/games/solaris-work/wt/ledger-control-candidate/tools/dev/test_perf_window.py:73` in the promotion receipt (`docs/bugs/promotions/PERF-WINDOW-HAS-NO-CALL-GRAPH-AND-NO-MAINTAINED-SCRIPT.json`).

## Claims the control does not prove

The "no maintained script" half is the fix commit's new file, not a reversed hunk: the control
proves the `-g --call-graph fp` flag only, and the row title says so.
