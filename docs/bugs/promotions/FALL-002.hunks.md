# FALL-002 — promotion patch mapping

- original fix: `35308579c` (or its HEAD equivalent; see below)
- patch: `promotions/FALL-002.patch` = the non-test hunks of the fix as they stand at the promoted HEAD,
  also kept at `docs/bugs/fix-patches/FALL-002.patch` and passed to `ledger promote --fix-patch`.
- control: reversing this patch on the exact candidate must fail the row's own registered ref.

## Hunks

brushing.rs: adapted re-break of the brush handler entry (`use_on` early return). Original commit created brushing use/complete; later commits rewrote the module so no verbatim reverse exists.

## Omitted hunks and reasons

- The original commit's test/test-wiring hunks are omitted: (b) the tests are kept in the candidate and
  reversing the wiring would stop the control from compiling.
- Any hunk the original commit had in files this row does not own is omitted: (a) it belongs to another
  row's behaviour.
