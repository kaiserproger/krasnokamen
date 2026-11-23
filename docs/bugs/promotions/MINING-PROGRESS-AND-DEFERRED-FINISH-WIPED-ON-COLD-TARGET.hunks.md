# MINING-PROGRESS-AND-DEFERRED-FINISH-WIPED-ON-COLD-TARGET — promotion patch mapping

- patch: `promotions/MINING-PROGRESS-AND-DEFERRED-FINISH-WIPED-ON-COLD-TARGET.patch` (copy of `docs/bugs/fix-patches/MINING-PROGRESS-AND-DEFERRED-FINISH-WIPED-ON-COLD-TARGET.patch`, the reversed form used by `ledger promote --fix-patch`).
- control: reversal on the exact candidate makes the row's own ref red at its assertion; restore green (see `MINING-PROGRESS-AND-DEFERRED-FINISH-WIPED-ON-COLD-TARGET.json`).

## Hunks

tick/mining.rs at HEAD: cold_target_or_eye freeze after range_holds, cold_delivery_frame wired into take_finished, and the eye/target helpers. The control reverses these hunks and the row's new cold-neighbour test fails at its assertion; the earlier `never used` function is now wired.

## Omitted hunks and reasons

- Test files / inline `#[cfg(test)]` hunks: (b) kept so the control compiles and runs the ref.
- Other files of earlier fix commits: (a) another row's behaviour.
