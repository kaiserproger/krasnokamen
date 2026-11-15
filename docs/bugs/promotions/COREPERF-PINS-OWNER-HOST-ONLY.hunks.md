# COREPERF-PINS-OWNER-HOST-ONLY: forward-ported hunk map

Base: `5315128eeb4cfcd19a6158db6498307ac674cad4`. The adjacent patch is identical to `../fix-patches/COREPERF-PINS-OWNER-HOST-ONLY.patch`; digest uses `ledger._split_non_test`. Tests/docs stay in the control. No invented pre-image or a/-side behavior adaptation.

All original profiles.py hunk behavior is retained at tools/coreperf/profiles.py:51–69 (owner-host detection and relocation-compatible manifest path). Later incremental-pin validation is unchanged. No omitted original non-test hunk. The proving ref is the relocated-pin ref, tools/coreperf/test_profiles.py:205–209: a valid relocated pin must be accepted. Other red refs complaining about error wording are explicitly NOT evidence.

## Every original non-test hunk

The dispositions above apply to each listed hunk, including mixed hunks at sub-hunk granularity.

| Commit | Original path/hunk |
| --- | --- |
| `80191f115` | `tools/coreperf/profiles.py` `@@ -36,13 +36,18 @@ PIN_ROOT = Path("/run/media/kaiserroman/games/solaris-work/pins")` |

## Proof

Registered proving refs are run with retained tests on HEAD, with this patch reverse-applied and then restored. Receipts: `receipts/HarnessFix4/COREPERF-PINS-OWNER-HOST-ONLY.red.log` and `.green.log`. Only the symptom assertion named above carries the red; other failures are not evidence. The row stays open until Main runs ledger promote and independent review.
