# HARNESS-MEM-RUN-TERM-DIRECT-CHILD-ONLY — patch mode, several commits

`fix_commit = patch:08df9f05193f63924b70b07fece8bdabc32f7dee1cc5e2344e74dc8976507373` (`docs/bugs/fix-patches/HARNESS-MEM-RUN-TERM-DIRECT-CHILD-ONLY.patch`, sha256 `08df9f05193f63924b70b07fece8bdabc32f7dee1cc5e2344e74dc8976507373`).

The path: the wrapper cancels a scope by TERMing the direct child first, forwarding TERM to the other scope members, and asking the guardian to escalate; the child status a fast trap could reap past is recovered; the cancel grace is validated at argument parsing. Commits: `5675e500ff` (the first TERM pass, the guardian escalation / the owner-line
protocol), `480d512731` (the sweep skips the already-signalled child, the grace validation / the stale-owner
verdict and the shared lock), `7d8f2a36d5` (the saved-status recovery, the fail-closed reservation / the
alias removal and install-order text).

## Omitted: the reservation block and `reserved_cpus.py`/`reserve-cpus` hunks of the same commits — reason (a), they are HARNESS-RESERVE-CPUS-STALE-RESERVATION's path (its own patch now carries them).

Nothing else is omitted: every other non-test hunk of those commits is in this patch.

## Proving refs and lines (from `docs/bugs/promotions/HARNESS-MEM-RUN-TERM-DIRECT-CHILD-ONLY.json`)

- `python:tools.dev.test_mem_run_term:CancelledScopeTests.test_a_cancelled_child_receives_exactly_one_term` — red at `/run/media/kaiserroman/games/solaris-work/wt/ledger-control-candidate/tools/dev/test_mem_run_term.py:206`
- `python:tools.dev.test_mem_run_term:CancelledScopeTests.test_a_term_ignoring_descendant_does_not_hold_the_scope` — red at `/run/media/kaiserroman/games/solaris-work/wt/ledger-control-candidate/tools/dev/test_mem_run_term.py:166`
- `python:tools.dev.test_mem_run_term:CancelledScopeTests.test_the_token_is_held_during_the_scope_and_free_after_a_cancellation` — red at `/run/media/kaiserroman/games/solaris-work/wt/ledger-control-candidate/tools/dev/test_mem_run_term.py:225`
- `python:tools.dev.test_reserve_cpus:MemRunReservationTests.test_a_non_numeric_cancel_grace_is_named_before_the_scope_starts` — red at `/run/media/kaiserroman/games/solaris-work/wt/ledger-control-candidate/tools/dev/test_reserve_cpus.py:246`
