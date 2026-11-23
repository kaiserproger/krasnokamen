# HARNESS-MEM-RUN-TERM-DIRECT-CHILD-ONLY — patch mode

`fix_commit = patch:24d942a8b353c596c10be4e663db3120e7cd68bfd56087e7372184bf6db21d94`

Derived from `5675e500ff`: cancel() forwards TERM to the direct child first, then to every other scope member. The patch is committed as
`docs/bugs/fix-patches/HARNESS-MEM-RUN-TERM-DIRECT-CHILD-ONLY.patch` (the row digest is `patch:24d942a8b353c596c10be4e663db3120e7cd68bfd56087e7372184bf6db21d94`;
`sha256` of the file: `24d942a8b353c596c10be4e663db3120e7cd68bfd56087e7372184bf6db21d94`).

## Hunks the patch carries

| file | +start | +count | hunk |
|---|---|---|---|
| tools/dev/mem-run-guard.py | 8 | 7 | import shutil |
| tools/dev/mem-run-guard.py | 25 | 7 | if sys.argv[1] == '--send-locks': |
| tools/dev/mem-run-guard.py | 34 | 16 | listener.listen(1) |
| tools/dev/mem-run-guard.py | 53 | 10 | try: |
| tools/dev/mem-run-guard.py | 73 | 14 | try: |
| tools/dev/mem-run.sh | 421 | 7 | unit="krasnokamen-mem-run-${guard_dir##*.}.scope" |
| tools/dev/mem-run.sh | 444 | 34 | printf "%s\t%s\t%s\t%s\n" "$cg" "$MEMRUN_EST_KIB" "$MEMRUN_PREEMPTIBLE" "$MEMRUN |

## Original hunks left out

The patch is a fix in several commits: `5675e500ff` (TERM to the direct child first, the cgroup sweep,
the guardian escalation), `480d512731` (the sweep skips the child it already signalled, the grace
validation) and `7d8f2a36d5` (the failed-status recovery, the fail-closed reservation fallback,
the install-order text). It is the complete base→HEAD diff of `tools/dev/mem-run.sh`, so nothing of
this file is omitted; the commit's test-file hunks are never part of a fix patch.

Omitted from this patch: the *reservation-helper* hunks of the same commits (`tools/dev/reserved_cpus.py`,
`tools/dev/reserve-cpus`) — reason **(a)**: they are HARNESS-RESERVE-CPUS-STALE-RESERVATION's own path,
and reversing them here would red that row's assertions inside this control.

## Proving ref and line

- `python:tools.dev.test_mem_run_term:CancelledScopeTests.test_a_term_ignoring_descendant_does_not_hold_the_scope` — red at `/run/media/kaiserroman/games/solaris-work/wt/ledger-control-candidate/tools/dev/test_mem_run_term.py:137` in the promotion receipt (`docs/bugs/promotions/HARNESS-MEM-RUN-TERM-DIRECT-CHILD-ONLY.json`).
- `python:tools.dev.test_mem_run_term:CancelledScopeTests.test_the_token_is_held_during_the_scope_and_free_after_a_cancellation` — red at `/run/media/kaiserroman/games/solaris-work/wt/ledger-control-candidate/tools/dev/test_mem_run_term.py:181` in the promotion receipt (`docs/bugs/promotions/HARNESS-MEM-RUN-TERM-DIRECT-CHILD-ONLY.json`).

## Claims the control does not prove

None: the patch is the whole behaviour the row names.
