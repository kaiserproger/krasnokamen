# BRUSH-RESET-WRITES-DUSTED-INTO-COLD-CHUNK — promotion patch mapping

- original fix: `c7bd44dd06` (or its HEAD equivalent; see below)
- patch: `promotions/BRUSH-RESET-WRITES-DUSTED-INTO-COLD-CHUNK.patch` = the non-test hunks of the fix as they stand at the promoted HEAD,
  also kept at `docs/bugs/fix-patches/BRUSH-RESET-WRITES-DUSTED-INTO-COLD-CHUNK.patch` and passed to `ledger promote --fix-patch`.
- control: reversing this patch on the exact candidate must fail the row's own registered ref.

## Hunks

brushing.rs reset-pass guard. Left out: the `#[path = "brushing_tests.rs"] mod tests;` hunk (b) because the test is now an inline module in brushing.rs.

## Omitted hunks and reasons

- The original commit's test/test-wiring hunks are omitted: (b) the tests are kept in the candidate and
  reversing the wiring would stop the control from compiling.
- Any hunk the original commit had in files this row does not own is omitted: (a) it belongs to another
  row's behaviour.
