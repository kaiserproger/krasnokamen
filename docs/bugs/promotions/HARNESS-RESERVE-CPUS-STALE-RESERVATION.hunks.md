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

The patch is a fix in several commits — `5675e500ff` (the reservation-file protocol: owner line,
`verdict`/`clear` under the shared lock) plus `480d512731` and `7d8f2a36d5` (unparseable owner =
stale, the `starttime` alias removal, the install-order text) — narrowed to the hunks after
`5675e500ff` for one reason: reversing the whole protocol makes every ref in
`tools.dev.test_reserve_cpus` unrunnable (the helper would have no `verdict`/`clear`), so control A
could not run at all. The reverted code is the *later* fix, which keeps the API the tests call.

Omitted from this patch: the `reserved_cpus.py` docstring/install-order lines and the `reserve-cpus`
header are text-only; the *whole-pool* message hunk of `mem-run.sh` is not reversed here (it is
`mem-run.sh`'s file — reason (a), MEM-RUN's own patch) and the row title says so.

## Proving ref and line

- `python:tools.dev.test_reserve_cpus:MemRunReservationTests.test_a_live_reservation_logs_the_cpus_it_excludes` — red at `/run/media/kaiserroman/games/solaris-work/wt/ledger-control-candidate/tools/dev/test_reserve_cpus.py:176` in the promotion receipt (`docs/bugs/promotions/HARNESS-RESERVE-CPUS-STALE-RESERVATION.json`).
- `python:tools.dev.test_reserve_cpus:MemRunReservationTests.test_a_stale_reservation_is_named_cleared_and_does_not_pin_the_scope` — red at `/run/media/kaiserroman/games/solaris-work/wt/ledger-control-candidate/tools/dev/test_reserve_cpus.py:167` in the promotion receipt (`docs/bugs/promotions/HARNESS-RESERVE-CPUS-STALE-RESERVATION.json`).
- `python:tools.dev.test_reserve_cpus:ReserveCpusTests.test_a_dead_owner_is_cleared_instead_of_blocking_the_next_reservation` — red at `/run/media/kaiserroman/games/solaris-work/wt/ledger-control-candidate/tools/dev/test_reserve_cpus.py:75` in the promotion receipt (`docs/bugs/promotions/HARNESS-RESERVE-CPUS-STALE-RESERVATION.json`).

## Claims the control does not prove

The "whole pool" fallback message hunk is not reversed by this patch, so
`test_a_reservation_covering_the_whole_pool_is_named_not_silently_dropped` is green in the control.
The row title says so; the control proves the stale/clear path only.
