# JOURNAL-TMP-NEVER-PRUNED — negative-control patch

Fix: `docs/bugs/fix-patches/JOURNAL-TMP-NEVER-PRUNED.patch`
(`fix_commit = "patch:<sha256 of that file>"`).

## Original commit

`8bf3987c4` — "journal hygiene, prune surfacing and the fsync window knob".
It closed two rows at once, so this patch carries only the part this row owns:
the unpublished-batch sweep that `JournalWriter::open` runs. Everything else
from that commit (the prune report, the fsync ceiling, the config knob) belongs
to another row or is API the kept tests call.

## Hunk mapping

| Patch hunk | HEAD `file:line` | What it is |
|---|---|---|
| the only hunk | `crates/mc-core/src/journal.rs:680-698` | the `sweep_unpublished_batches(&dir)` call and its two warning lines inside `JournalWriter::open` |

`git apply -R` on the candidate deletes that call, so opening a journal no
longer removes `batch-*.bin.tmp` leftovers.

## Exclusions

- `sweep_unpublished_batches`, `SweepOutcome`, `is_unpublished_batch` stay: the
  kept tests call them and the *second* tmp row's test (`a_sweep_failure_is
  reported_...`) reads the outcome type; reversing them stops the control from
  compiling (reason (b)). The reversed tree leaves the sweep unreferenced, which
  is a `dead_code` warning, not an error.
- The prune report and the fsync ceiling are `JOURNAL-PRUNE-ERRORS-SILENT` and
  `WAL-FSYNC-…`: another row's behaviour (reason (a)).

## a/-side adaptation

None. `JournalWriter::open` still returns the same value; only the side effect
goes away.

## Proving ref

`cargo:mc-core:journal_tests::open_removes_unpublished_batch_leftovers` — red at
`crates/mc-core/src/journal_tests.rs:2000`
(`assert!(!leftover.exists(), "the failed append's temporary is removed")`, the
first assertion after the reopen; `:2004` and `:2010` fail with it).
Measured: clean tree 476412c47 with only this patch reversed — journal_tests 38
passed, 1 failed, that ref (receipt
`.analysis/validation/20261120T010517-test-r0emth2o`).
