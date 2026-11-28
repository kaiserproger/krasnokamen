# HARNESS-CIV-GATE-LOCAL-SCAN-UNLOADED — evidence

- Branch: `wt/D145Harness`, base `2f4d802c08eb6aa9ab18222582c9c39f13f39d55`.
- Fix commit in tree: `457d777785256049be238a0244249b16db27347c`
  (`fix(harness): a local building waits for its scanned chunks (HANDOFF-48 A)`).
- Production hunks: `tools/harness/backends/civilization_seed_qa.py`
  (`_wait_scan_loaded` helper applied to the local path of `building_blocks`).
- Regression ref added:
  `python:tools.harness.test_civilization_seed_qa:CivilizationEvidenceTests.test_local_building_waits_for_its_chunks_before_the_scan`.

## Negative control (red)

Command (from a disposable `git worktree add --detach ... HEAD`, the fix's
non-test hunks reverse-applied with `git apply -R`; applied cleanly):

    python3 -m unittest tools.harness.test_civilization_seed_qa.CivilizationEvidenceTests.test_local_building_waits_for_its_chunks_before_the_scan

Result: `FAILED (failures=1)` — `AssertionError: 'minecraft_scan_blocks' !=
'minecraft_wait_for_loaded_block'` (the local path scans before the chunk wait).
Full log: `control-red.log`.

## Green after restore

Same command on the restored tree: `Ran 1 test ... OK`. Full log:
`control-green.log`. Whole module on HEAD: `Ran 50 tests ... OK`
(`../HEAD-full-module-green.log`).

## Promotion blocker (outside this lane)

`repro = "client"`, so promotion needs a real-client receipt for one of the
row's refs plus a `pin-release` server whose manifest matches the candidate
fingerprint. Exact command on the owner host:

    <work>/solaris/tools/dev/scene-slot.sh <work>/solaris/tools/dev/mem-run.sh -m 8G \
      python3 -m tools.harness run civilization-seed-qa --client native --windowed

The python ref above is harness-only evidence; it cannot substitute for the
client receipt required by the `repro = "client"` rule.
