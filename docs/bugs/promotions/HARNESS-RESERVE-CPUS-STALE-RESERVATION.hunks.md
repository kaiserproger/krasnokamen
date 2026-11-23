# HARNESS-RESERVE-CPUS-STALE-RESERVATION — patch mode

`fix_commit = patch:f0dab557b9b203492e275eee61d7e9ecdca4268cabf3d35f8c1dfa4a9a6b6d79`

Derived from `5675e500ff`: a stale reservation is named, its corpse cleared under the shared lock. The patch is committed as
`docs/bugs/fix-patches/HARNESS-RESERVE-CPUS-STALE-RESERVATION.patch` (the row digest is `patch:f0dab557b9b203492e275eee61d7e9ecdca4268cabf3d35f8c1dfa4a9a6b6d79`;
`sha256` of the file: `f0dab557b9b203492e275eee61d7e9ecdca4268cabf3d35f8c1dfa4a9a6b6d79`).

## Hunks the patch carries

| file | +start | +count | hunk |
|---|---|---|---|
| tools/dev/mem-run.sh | 373 | 26 | fi |
| tools/dev/reserve-cpus | 6 | 19 |  |
| tools/dev/reserve-cpus | 27 | 36 | for part in ${cpus//,/ }; do |

## Original hunks left out

None beyond the fix commit's own test-file hunks (test files are never part of a fix patch).
The patch is the complete non-test hunk set of the behaviour the row names; nothing is omitted
for another row, for API a kept test calls, or for a later rewrite.

## Proving ref and line

- `python:tools.dev.test_reserve_cpus:MemRunReservationTests.test_a_live_reservation_logs_the_cpus_it_excludes` — red at `/run/media/kaiserroman/games/solaris-work/wt/ledger-control-candidate/tools/dev/test_reserve_cpus.py:176` in the promotion receipt (`docs/bugs/promotions/HARNESS-RESERVE-CPUS-STALE-RESERVATION.json`).
- `python:tools.dev.test_reserve_cpus:MemRunReservationTests.test_a_stale_reservation_is_named_cleared_and_does_not_pin_the_scope` — red at `/run/media/kaiserroman/games/solaris-work/wt/ledger-control-candidate/tools/dev/test_reserve_cpus.py:167` in the promotion receipt (`docs/bugs/promotions/HARNESS-RESERVE-CPUS-STALE-RESERVATION.json`).
- `python:tools.dev.test_reserve_cpus:ReserveCpusTests.test_a_dead_owner_is_cleared_instead_of_blocking_the_next_reservation` — red at `/run/media/kaiserroman/games/solaris-work/wt/ledger-control-candidate/tools/dev/test_reserve_cpus.py:75` in the promotion receipt (`docs/bugs/promotions/HARNESS-RESERVE-CPUS-STALE-RESERVATION.json`).

## Claims the control does not prove

The "whole pool" fallback message hunk is not reversed by this patch, so
`test_a_reservation_covering_the_whole_pool_is_named_not_silently_dropped` is green in the control.
The row title says so; the control proves the stale/clear path only.
