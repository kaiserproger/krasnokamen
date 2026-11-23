# COMMAND-CHILD-SENDER-DROPPED — promotion patch mapping

- original fix: `d761f6961cdf70c1ed7c475b70c76ccb48144f2715f6b851163f0dfc79ea4cc9 eras` (or its HEAD equivalent; see below)
- patch: `promotions/COMMAND-CHILD-SENDER-DROPPED.patch` = the non-test hunks of the fix as they stand at the promoted HEAD,
  also kept at `docs/bugs/fix-patches/COMMAND-CHILD-SENDER-DROPPED.patch` and passed to `ledger promote --fix-patch`.
- control: reversing this patch on the exact candidate must fail the row's own registered ref.

## Hunks

command_context_deferred.rs: adapted re-break running the queued job against the root context (`job_source_is_current(state, ...)` instead of the job's dimension world). Later commits rewrote the shared-runtime path.

## Omitted hunks and reasons

- The original commit's test/test-wiring hunks are omitted: (b) the tests are kept in the candidate and
  reversing the wiring would stop the control from compiling.
- Any hunk the original commit had in files this row does not own is omitted: (a) it belongs to another
  row's behaviour.
