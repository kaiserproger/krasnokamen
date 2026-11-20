# JOURNAL-PRUNE-ERRORS-SILENT — negative-control patch

Fix: `docs/bugs/fix-patches/JOURNAL-PRUNE-ERRORS-SILENT.patch`
(`fix_commit = "patch:<sha256 of that file>"`).

## Original commit

`8bf3987c4` — "journal hygiene, prune surfacing and the fsync window knob".
This patch carries only the failure accounting inside `prune_batches_through`:
the part that turns a filesystem refusal into a counted, named, dashboard-visible
failure. The tmp sweep from the same commit belongs to
`JOURNAL-TMP-NEVER-PRUNED`.

## Hunk mapping

| Patch hunk | HEAD `file:line` | What it is |
|---|---|---|
| the only hunk | `crates/mc-core/src/journal.rs:1580-1623` | `prune_batches_through`: `report.failed`/`report.error`/`report.retained_bytes` on an unreadable directory and on a refused removal, plus `record_prune_failure` and the `PRUNE_RETAINED_BYTES` reset |

`git apply -R` restores the silent shape (`let Ok(entries) = … else { return }`,
`let Ok(entry) = … else { continue }`, `if remove_file(...).is_ok()`) and stops
calling `record_prune_failure`, so the report and the dashboard counter never
see a refusal.

## Exclusions

- `PruneReport` (with `failed`, `retained_bytes`, `error`) and
  `journal_prune_stats` stay on both sides: the kept tests call them, so
  reversing them stops the control from compiling (reason (b)).
- The tmp sweep is `JOURNAL-TMP-NEVER-PRUNED` (reason (a)).
- `record_prune_failure` and `PRUNE_RETAINED_BYTES` stay defined; the reversed
  tree leaves them unreferenced (a `dead_code` warning, not an error).

## a/-side adaptation

None.

## Proving ref

`cargo:mc-core:journal_tests::prune_reports_and_counts_a_refused_removal` — red at
`crates/mc-core/src/journal_tests.rs:2036`
(`assert_eq!(report.failed, 1)` — left 0, right 1 measured). The ref also asserts
`report.error.is_some()` and the dashboard counter at `:2046`; the first
assertion is the row's symptom (a refusal nobody reports).

Measured: clean tree 476412c47 with only this patch reversed — journal_tests 38
passed, 1 failed, that ref (receipt
`.analysis/validation/20261120T010626-test-5eluvryo`).
