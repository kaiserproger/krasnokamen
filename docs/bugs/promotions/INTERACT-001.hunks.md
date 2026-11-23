# INTERACT-001 — promotion patch mapping

- original fix: `35308579c` (or its HEAD equivalent; see below)
- patch: `promotions/INTERACT-001.patch` = the non-test hunks of the fix as they stand at the promoted HEAD,
  also kept at `docs/bugs/fix-patches/INTERACT-001.patch` and passed to `ledger promote --fix-patch`.
- control: reversing this patch on the exact candidate must fail the row's own registered ref.

## Hunks

vanilla_interact.rs: adapted re-break of the axe strip/scrape/unwax entry (`axe` early return). Later commits split the handler out of interact.rs.

## Omitted hunks and reasons

- The original commit's test/test-wiring hunks are omitted: (b) the tests are kept in the candidate and
  reversing the wiring would stop the control from compiling.
- Any hunk the original commit had in files this row does not own is omitted: (a) it belongs to another
  row's behaviour.
