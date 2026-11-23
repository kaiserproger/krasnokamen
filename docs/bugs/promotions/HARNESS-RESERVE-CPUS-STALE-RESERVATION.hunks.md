# HARNESS-RESERVE-CPUS-STALE-RESERVATION — patch mode, several commits

`fix_commit = patch:1c996e0e836d4739d763801c12875917927aee37a561aa178bc0bcde6e055371` (`docs/bugs/fix-patches/HARNESS-RESERVE-CPUS-STALE-RESERVATION.patch`, sha256 `1c996e0e836d4739d763801c12875917927aee37a561aa178bc0bcde6e055371`).

The path: the wrapper judges the reservation file through the helper (unparseable owner = stale, failing helper named with its stderr and the first line validated as a CPU list, unexpected verdict = exit 75), and the helper/script carry the owner line, the lock and the alias removal. Commits: `5675e500ff` (the first TERM pass, the guardian escalation / the owner-line
protocol), `480d512731` (the sweep skips the already-signalled child, the grace validation / the stale-owner
verdict and the shared lock), `7d8f2a36d5` (the saved-status recovery, the fail-closed reservation / the
alias removal and install-order text).

## Omitted: the `mem-run.sh` cancel/grace/token hunks — reason (a), they are HARNESS-MEM-RUN-TERM-DIRECT-CHILD-ONLY's path. The wrapper half of this patch is spliced from the base script: the reservation block reverted while the rest of the wrapper stays at HEAD, so each patch reverses independently and both together yield base.

Nothing else is omitted: every other non-test hunk of those commits is in this patch.

## Proving refs and lines (from `docs/bugs/promotions/HARNESS-RESERVE-CPUS-STALE-RESERVATION.json`)

- `python:tools.dev.test_reserve_cpus:MemRunReservationTests.test_a_failing_verdict_helper_is_named_and_the_first_line_honoured` — red at `/run/media/kaiserroman/games/solaris-work/wt/ledger-control-candidate/tools/dev/test_reserve_cpus.py:222`
- `python:tools.dev.test_reserve_cpus:MemRunReservationTests.test_a_garbage_first_line_with_a_failing_helper_refuses_the_scope` — red at `/run/media/kaiserroman/games/solaris-work/wt/ledger-control-candidate/tools/dev/test_reserve_cpus.py:235`
- `python:tools.dev.test_reserve_cpus:MemRunReservationTests.test_a_live_reservation_logs_the_cpus_it_excludes` — red at `/run/media/kaiserroman/games/solaris-work/wt/ledger-control-candidate/tools/dev/test_reserve_cpus.py:201`
- `python:tools.dev.test_reserve_cpus:MemRunReservationTests.test_a_stale_reservation_is_named_cleared_and_does_not_pin_the_scope` — red at `/run/media/kaiserroman/games/solaris-work/wt/ledger-control-candidate/tools/dev/test_reserve_cpus.py:192`
- `python:tools.dev.test_reserve_cpus:ReservedCpusProtocolTests.test_a_present_unparseable_owner_is_stale_not_immortal` — red at `/run/media/kaiserroman/games/solaris-work/wt/ledger-control-candidate/tools/dev/test_reserve_cpus.py:129`
