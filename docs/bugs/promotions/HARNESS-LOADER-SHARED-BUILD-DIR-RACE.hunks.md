# Forward-ported Loader isolation fix

Base: `5315128eeb4cfcd19a6158db6498307ac674cad4`. Adjacent patch equals `../fix-patches/HARNESS-LOADER-SHARED-BUILD-DIR-RACE.patch`; SHA-256 is computed after `ledger._split_non_test`. No tests reversed and no a/-side adaptation.

| Original commit/hunk | HEAD equivalent or omission |
| --- | --- |
| `3aca2c4ecd39ce4d054f41c067b85060d1316e1f`, runtime.py +1019 | runtime.py live Gradle import, write_gradle_init invocation and unconditional --init-script; retained in patch hunks 1–2. Closes shared class output paths. |
| same, java_launcher.py +155 | write_gradle_init API remains (b): retained launcher tests call it directly; not red evidence. |
| same, java_launcher.py +188 | prepare_fabric_launch was deleted by `b6f1ac655` and merged by `3c2279125` (c). Prepared Java now uses runtime.start_client, whose equivalent isolation hunks are retained above. |
| `495367bce0098dc32bb2811098069dca0513f16d`, runtime.py +1019 and +1032 | live import and --project-cache-dir retained in both patch hunks. Closes shared Gradle executionHistory/Loom metadata. |
| same, java_launcher.py +155 | gradle_project_cache/write_gradle_init API remains (b), used by kept tests. |
| same, java_launcher.py +200 | prepare_fabric_launch deleted (c), same replacement live path as above. |

Docs and tests are excluded by ledger rules. Proving ref: `python:tools.harness.test_runtime_profiles:ProfileTaskTests.test_every_gradle_launch_isolates_endpoint_builds_even_with_explicit_heap`, `tools/harness/test_runtime_profiles.py:400` onwards: spawned command must isolate both outputs and project cache. Direct helper tests retained for API integrity are not red evidence. Receipts in `receipts/HarnessFix4/HARNESS-LOADER-SHARED-BUILD-DIR-RACE.*.log` include the combined reversal and each runtime wiring change separately.
