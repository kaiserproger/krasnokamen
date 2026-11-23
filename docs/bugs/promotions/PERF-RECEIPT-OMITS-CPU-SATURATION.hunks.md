# PERF-RECEIPT-OMITS-CPU-SATURATION — patch mode

`fix_commit = patch:c3dd61d2d25f47ceda52550f6c596b560bb6527d0772723a7111dfec9df19a4e`

Derived from `ed34d1c831`: the receipt carries the PSI cpu pressure stall total and average. The patch is committed as
`docs/bugs/fix-patches/PERF-RECEIPT-OMITS-CPU-SATURATION.patch` (the row digest is `patch:c3dd61d2d25f47ceda52550f6c596b560bb6527d0772723a7111dfec9df19a4e`;
`sha256` of the file: `c3dd61d2d25f47ceda52550f6c596b560bb6527d0772723a7111dfec9df19a4e`).

## Hunks the patch carries

| file | +start | +count | hunk |
|---|---|---|---|
| crates/mc-edge/src/host_telemetry.rs | 90 | 12 | impl HostSampler { |
| tools/coreperf/profiles.py | 1333 | 7 | def host_receipt(out: Path, *, cpu_root: Path = Path("/sys/devices/system/cpu"), |

## Original hunks left out

None beyond the fix commit's own test-file hunks (test files are never part of a fix patch).
The patch is the complete non-test hunk set of the behaviour the row names; nothing is omitted
for another row, for API a kept test calls, or for a later rewrite.

## Proving ref and line

- `cargo:mc-edge:host_telemetry::host_telemetry_tests::sampler_reads_the_throttle_counters_from_the_cgroup_fixture` — red at `crates/mc-edge/src/host_telemetry_tests.rs:137` in the promotion receipt (`docs/bugs/promotions/PERF-RECEIPT-OMITS-CPU-SATURATION.json`).
- `python:tools.coreperf.test_profiles:HostReceiptFieldsTests.test_receipt_records_cgroup_cpu_throttling` — red at `/run/media/kaiserroman/games/solaris-work/wt/ledger-control-candidate/tools/coreperf/test_profiles.py:1064` in the promotion receipt (`docs/bugs/promotions/PERF-RECEIPT-OMITS-CPU-SATURATION.json`).

## a/-side adaptation

The reversal drops the `cgroup_cpu_nr_throttled` / `cgroup_cpu_throttled_usec` fields from the receipt;
the control tree constructs an empty field map, which changes no behaviour and is the only a/-side text
outside the original pre-image.

## Claims the control does not prove

Trimmed to what is asserted today (ReviewD123Harness1 12): the receipt carries the cgroup
`cpu.stat` throttling counters (`usage_usec`, `nr_periods`, `nr_throttled`, `throttled_usec`), not
CPU PSI totals; the row names CPU saturation, and PSI totals are the memory row's field.
