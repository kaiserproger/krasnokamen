# HARNESS-CIV-GATE-PLANNER-READ-BLOCKED-BY-STATUS-DIALOG — evidence

- Branch: `wt/D145Harness`, base `2f4d802c08eb6aa9ab18222582c9c39f13f39d55`.
- Fix commit in tree: `a0c0f8902`
  (`harness(civ): walk to a remote house door in loaded hops; retry the planner read once`) —
  `_planner_status` retries both reads once after closing a screen left over the
  game view and names a screen that survives the close (`_clear_covering_screen`,
  `_screen_on_top`).
- Regression refs already on the row:
  `PlannerReadRecoveryTests.test_a_status_dialog_over_the_view_is_closed_and_the_read_retried`
  and `PlannerReadRecoveryTests.test_a_dialog_that_never_closes_names_itself_in_the_storage_error`.

## Negative control (red)

Disposable `git worktree add --detach ... HEAD`; `a0c0f8902`'s
`tools/harness/backends/village_autonomy_qa.py` hunks reverse-applied with
`git apply -R` (clean). Command:

    python3 -m unittest tools.harness.test_civilization_seed_qa.PlannerReadRecoveryTests

Result: `FAILED (failures=2, errors=1)`.

- `test_a_status_dialog_over_the_view_is_closed_and_the_read_retried`:
  `AssertionError: 'storage_error' unexpectedly found in {... 'storage_error':
  'storage read of block 98,67,-155 got no reply in 15.0s'}` — a screen the gate
  opened is not dismissed and the read is not retried (the row's symptom).
- `test_a_dialog_that_never_closes_names_itself_in_the_storage_error`:
  `AssertionError: 1 != 2` — only one read attempt, no close/retry, so the
  surviving screen is never named.

Full log: `control-red.log`.

## Green after restore

`PlannerReadRecoveryTests` — `Ran 3 tests ... OK` (`control-green.log`). Whole
module on HEAD: `Ran 50 tests ... OK` (`../HEAD-full-module-green.log`).

## Promotion blocker (outside this lane)

`repro = "client"`: the row needs a real-client `civilization-seed-qa` receipt
on a matching pin. Exact command:

    <work>/solaris/tools/dev/scene-slot.sh <work>/solaris/tools/dev/mem-run.sh -m 8G \
      python3 -m tools.harness run civilization-seed-qa --client native --windowed
