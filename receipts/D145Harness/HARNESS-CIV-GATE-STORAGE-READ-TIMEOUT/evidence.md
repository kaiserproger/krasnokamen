# HARNESS-CIV-GATE-STORAGE-READ-TIMEOUT — evidence

- Branch: `wt/D145Harness`, base `2f4d802c08eb6aa9ab18222582c9c39f13f39d55`.
- Fix commit in tree: `a0c0f8902`
  (`harness(civ): walk to a remote house door in loaded hops; retry the planner read once`) —
  the `_read_storage_container` matcher accepts the console reply anywhere in a
  line, not only at its start. (`457d77778` added the diagnostic TimeoutError
  that names the container and the log tail.)
- Regression ref already on the row:
  `python:tools.harness.test_civilization_seed_qa:PlannerReadRecoveryTests.test_a_console_reply_embedded_in_a_perf_line_is_accepted`.

## Negative control (red)

Disposable `git worktree add --detach ... HEAD`; the fix's
`tools/harness/backends/village_autonomy_qa.py` hunks reverse-applied with
`git apply -R` (clean). Command:

    python3 -m unittest tools.harness.test_civilization_seed_qa.PlannerReadRecoveryTests.test_a_console_reply_embedded_in_a_perf_line_is_accepted

Result: `ERROR` — `TimeoutError: storage read of block 98,67,-155 got no reply
in 2.0s; log tail: 'perf: ... post=0.0398, 67, -155 has the following block
data: {Items: [...], id: "minecraft:barrel", ...}'`. The reply is on disk but
the startswith-only matcher never sees it: the row's exact symptom. Full log:
`control-red.log` (the same log also holds the two planner-dialog refs, which
this reversal reds as well).

## Green after restore

`PlannerReadRecoveryTests` — `Ran 3 tests ... OK` (`control-green.log`). Whole
module on HEAD: `Ran 50 tests ... OK` (`../HEAD-full-module-green.log`).

## Promotion blocker (outside this lane)

`repro = "client"`: the row needs a real-client `civilization-seed-qa` receipt
(the 15 s storage timeout is a live gate path; no cargo/source control exists
for a filesystem wait). Exact command:

    <work>/solaris/tools/dev/scene-slot.sh <work>/solaris/tools/dev/mem-run.sh -m 8G \
      python3 -m tools.harness run civilization-seed-qa --client native --windowed
