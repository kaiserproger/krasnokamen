# PROTO-WIKI-UNCOMPRESSED-FRAME-REJECTED — promotion patch mapping

- original fix: `dd5662e81c` (the amended fix commit on this branch; frame.rs hunk) (or its HEAD equivalent; see below)
- patch: `promotions/PROTO-WIKI-UNCOMPRESSED-FRAME-REJECTED.patch` = the non-test hunks of the fix as they stand at the promoted HEAD,
  also kept at `docs/bugs/fix-patches/PROTO-WIKI-UNCOMPRESSED-FRAME-REJECTED.patch` and passed to `ledger promote --fix-patch`.
- control: reversing this patch on the exact candidate must fail the row's own registered ref.

## Hunks

mc-protocol/src/frame.rs: drop the `dataLength == 0` threshold rejection and the removed error variant. The same commit also carries the frame parity test (a test-file hunk, kept in the control); the control reverses only the frame.rs hunk.

## Omitted hunks and reasons

- The original commit's test/test-wiring hunks are omitted: (b) the tests are kept in the candidate and
  reversing the wiring would stop the control from compiling.
- Any hunk the original commit had in files this row does not own is omitted: (a) it belongs to another
  row's behaviour.
