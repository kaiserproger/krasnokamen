# PERF-RECEIPT-OMITS-CPU-GOVERNOR-AND-FREQUENCY — patch mode

`fix_commit = patch:a84b5471d975aac297e40566d1db61a8fc40689941d945b79692fad933c8d02c`

Derived from `ed34d1c831`: the receipt and HostSample read the governor and frequency from a temporary sysfs/cgroup tree, proved by its own test. The patch is committed as
`docs/bugs/fix-patches/PERF-RECEIPT-OMITS-CPU-GOVERNOR-AND-FREQUENCY.patch` (the row digest is `patch:a84b5471d975aac297e40566d1db61a8fc40689941d945b79692fad933c8d02c`;
`sha256` of the file: `a84b5471d975aac297e40566d1db61a8fc40689941d945b79692fad933c8d02c`).

## Hunks the patch carries

| file | +start | +count | hunk |
|---|---|---|---|
| tools/coreperf/profiles.py | 1332 | 7 | def host_receipt(out: Path, *, cpu_root: Path = Path("/sys/devices/system/cpu"), |

## Original hunks left out

None beyond the fix commit's own test-file hunks (test files are never part of a fix patch).
The patch is the complete non-test hunk set of the behaviour the row names; nothing is omitted
for another row, for API a kept test calls, or for a later rewrite.

## Proving ref and line

- `python:tools.coreperf.test_profiles:HostReceiptFieldsTests.test_receipt_records_the_cpu_clock_state` — red at `/run/media/kaiserroman/games/solaris-work/wt/ledger-control-candidate/tools/coreperf/test_profiles.py:1051` in the promotion receipt (`docs/bugs/promotions/PERF-RECEIPT-OMITS-CPU-GOVERNOR-AND-FREQUENCY.json`).

## Claims the control does not prove

The a/-side `cgroup_cpu_*: None` pre-image is the receipt field the reversal drops; the governor
pair and enabled/running ratio are asserted by the same test at its own lines, which is the red
recorded above. A separate per-field red for each of the pair is not produced.
