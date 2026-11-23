# PERF-WINDOW-READS-AMD-L2-ALIAS-AS-LAST-LEVEL-MISS-RATE — patch mode

`fix_commit = patch:4c591801cc62517529505ebcdac13e394c12b54aee756c53a4a2190fd1f27309`

Derived from `129228b11b`: the counter set moves to the host-verified AMD last-level and L2 context events. The patch is committed as
`docs/bugs/fix-patches/PERF-WINDOW-READS-AMD-L2-ALIAS-AS-LAST-LEVEL-MISS-RATE.patch` (the row digest is `patch:4c591801cc62517529505ebcdac13e394c12b54aee756c53a4a2190fd1f27309`;
`sha256` of the file: `4c591801cc62517529505ebcdac13e394c12b54aee756c53a4a2190fd1f27309`).

## Hunks the patch carries

| file | +start | +count | hunk |
|---|---|---|---|
| tools/dev/perf-window.sh | 47 | 7 | command -v perf >/dev/null || { echo "perf-window: perf not found" >&2; exit 2; |

## Original hunks left out

None beyond the fix commit's own test-file hunks (test files are never part of a fix patch).
The patch is the complete non-test hunk set of the behaviour the row names; nothing is omitted
for another row, for API a kept test calls, or for a later rewrite.

## Proving ref and line

- `python:tools.dev.test_perf_window:PerfWindowRecipeTests.test_the_counter_set_is_the_amd_last_level_events_not_the_l2_aliases` — red at `/run/media/kaiserroman/games/solaris-work/wt/ledger-control-candidate/tools/dev/test_perf_window.py:64` in the promotion receipt (`docs/bugs/promotions/PERF-WINDOW-READS-AMD-L2-ALIAS-AS-LAST-LEVEL-MISS-RATE.json`).

## Claims the control does not prove

None: the patch is the whole behaviour the row names.
