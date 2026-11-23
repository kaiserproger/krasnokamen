# PERF-RECEIPT-OMITS-MEMORY-PRESSURE — patch mode

`fix_commit = patch:e28edbd9218f02c853a7a467fa07acabf5459109f0468229ad670e722f6d7124`

Derived from `ed34d1c831`: the receipt carries the PSI memory pressure total and average. The patch is committed as
`docs/bugs/fix-patches/PERF-RECEIPT-OMITS-MEMORY-PRESSURE.patch` (the row digest is `patch:e28edbd9218f02c853a7a467fa07acabf5459109f0468229ad670e722f6d7124`;
`sha256` of the file: `e28edbd9218f02c853a7a467fa07acabf5459109f0468229ad670e722f6d7124`).

## Hunks the patch carries

| file | +start | +count | hunk |
|---|---|---|---|
| crates/mc-edge/src/host_telemetry.rs | 96 | 9 | impl HostSampler { |
| tools/coreperf/profiles.py | 1334 | 7 | def host_receipt(out: Path, *, cpu_root: Path = Path("/sys/devices/system/cpu"), |

## Original hunks left out

None beyond the fix commit's own test-file hunks (test files are never part of a fix patch).
The patch is the complete non-test hunk set of the behaviour the row names; nothing is omitted
for another row, for API a kept test calls, or for a later rewrite.

## Proving ref and line

- `cargo:mc-edge:host_telemetry::host_telemetry_tests::sampler_reads_memory_pressure_from_the_cgroup_fixture` — red at `crates/mc-edge/src/host_telemetry_tests.rs:158` in the promotion receipt (`docs/bugs/promotions/PERF-RECEIPT-OMITS-MEMORY-PRESSURE.json`).
- `python:tools.coreperf.test_profiles:HostReceiptFieldsTests.test_receipt_records_memory_pressure` — red at `/run/media/kaiserroman/games/solaris-work/wt/ledger-control-candidate/tools/coreperf/test_profiles.py:1079` in the promotion receipt (`docs/bugs/promotions/PERF-RECEIPT-OMITS-MEMORY-PRESSURE.json`).

## a/-side adaptation

The reversal drops `cgroup_memory_pressure` / `host_memory_pressure`; the control tree passes empty
field maps, which changes no behaviour.

## Claims the control does not prove

Only the memory PSI field is added; a host CPU/IO PSI total is not part of this fix (the row
names memory pressure).
