# D154 — the refreshed patches, audited against their Git pre-images

Owner D154/D154b, 2026-11-28. Source: `0bf88beb5` (PatchRefresh), merged as
`e0b771447`; 21 patch files changed (20 added, one modified). The acceptance
criterion is byte equality between each hunk's minus lines and their
corresponding lines in `git show <fix>^1:<path>`. Finding an identifier or a
lookalike line somewhere in the file is not enough.

## Result: 8 of 21 are not controls, and all 8 are `open`

| row | fix | first symbol the pre-image lacks | note |
|---|---|---|---|
| `FALL-001` | `35308579c9` | `crates/mc-core/src/vanilla_interact.rs` — file absent at `^1` | the patch makes `dragon_egg` return false; the fix `35308579` is what first added the two calls `crate::vanilla_interact::dragon_egg(...)` in `crates/mc-core/src/tick/input/interact.rs` (≈84, ≈177). Correct control: keep the helper, remove the two calls, test red, restore |
| `PLACE-001` | `35308579c9` | `crates/mc-core/src/spawn_protection.rs` — file absent at `^1` | same invented-preimage shape |
| `PERF-RECEIPT-HAS-NO-CLIENT-OBSERVED-TICK-ARRIVAL` | `2941bc91f` | `tools/coreperf/profiles.py`: `carries` | the fix adds `client_tick_arrival` outright; the minus side returns a stub whose status is unavailable and its red is a `KeyError` |
| `CIV-IRON-CAPTURE-OUTPUT-COLLECTED` | `d7f98cd4` | `tools/harness/backends/civilization_seed_qa.py`: `output_slot` | invented minus side |
| `HARNESS-REGRESSIONS-CLIENT-REBUILDS` | `a39164dc9` | `tools/harness/regressions.py`: `usable` | invented fallback that never existed |
| `HARNESS-VOICE-RECONNECT-CAPABILITY-REGISTRATION` | `0857a48c5` | `tools/harness/backends/screens_voice_qa.py`: `even` | invented minus side |
| `SETTLEMENT-AUTONOMY-VILLAGE-ROTATION-STARVES-TICK-GATES` | `44aa3d95a` | `crates/mc-core/src/settlements/autonomy.rs`: `rotation` | the clock-only reversal also failed the batch bound, not the row's own progress assertion |
| `TEST-NATIVE-HEALTH` | `ef67d139e` | `tools/harness/backends/native_matrix.py`: `isinstance` | the patch invents a weaker `HealthInterval` that never existed |

Each row keeps its real `fix_commit`, stays `open`, and carries the finding in its own
text. Withdrawn in `48f50c842` (seven patches) and `d35ddfee7` (the perf **patch** — the
earlier wording "withdrew the receipt" was wrong).

## Byte-exact checks pending for the other 13

`BATTLE-DISMOUNTED-RIDER-SKATES-TO-SWEEP-POINT`, `HARNESS-CIV-GATE-BIRTH-CAPTURE-HITS-THE-WALL`,
`HARNESS-CIV-GATE-LOCAL-SCAN-UNLOADED`, `HARNESS-CIV-GATE-OBSERVER-WAIT-HANGS`,
`HARNESS-CIV-GATE-PLANNER-READ-BLOCKED-BY-STATUS-DIALOG`, `HARNESS-CIV-GATE-REMOTE-SCAN-UNLOADED`,
`HARNESS-CIV-GATE-STORAGE-READ-TIMEOUT`, `HARNESS-EXCLUSIVE-WINDOW-SCENE-SLOT-DEADLOCK`,
`HARNESS-JAVA-MATRIX-26-1-2-EVENTS-NONE-ADAPTER-TYPEERROR`,
`HARNESS-JAVA-MATRIX-FOREIGN-PORT-AUTH-ACK`, `SETTLEMENT-HOUSE-BILL-STALLS-BELOW-COBBLESTONE-DEMAND`,
`VILLAGER-BABY-WADES-INTO-HEAD-DEEP-WATER`, `DIMENSION-SWITCH-KNOCKBACK-DISCONNECT`.

The preliminary screen ignored indentation and comments; it does not accept any
of these patches as controls. `PatchRefresh` is checking the original blobs against
the exact parent hunks. This includes the modified dimension-switch patch
(`5810902c7^1`, `crates/mc-edge/src/server/pump.rs`). The byte comparison and the
red/green run are separate requirements.

## FALL-001 rebuild

The withdrawn patch rewrote the helper to return `false`. The correct control
reverses only the `dig` and `use_item_on` dispatch in
`crates/mc-core/src/tick/input/interact.rs`, leaving the current helper intact.
Only `interact_parity_dragon_egg_use_teleports` must go red on this reversal;
`interact_parity_dragon_egg_conserves_block_before_falling` can stay green because
it calls the helper directly. The rebuild and red/green proof are pending.

## The seven withdrawn rows, per row (owner D154b item 3)

All seven are `open` on `main` with their real `fix_commit` restored.

| row | real pre-image | test that must red when the dispatch is reversed |
|---|---|---|
| `FALL-001` (`35308579c9`) | `crates/mc-core/src/tick/input/interact.rs` **without** the two `crate::vanilla_interact::dragon_egg(...)` calls (≈84 and ≈177); the fix added them. Helper `vanilla_interact::dragon_egg` exists on both sides and stays. | `crates/mc-core/src/interact_parity_tests.rs:75` `interact_parity_dragon_egg_use_teleports`. **Not** required: `interact_parity_dragon_egg_conserves_block_before_falling` (line 590) calls the helper directly at 593. |
| `PLACE-001` (`35308579c9`) | same commit and same shape: `tick/input/interact.rs` without the spawn-protection calls | `interact_parity_spawn_protection_with_operator` |
| `PERF-RECEIPT-HAS-NO-CLIENT-OBSERVED-TICK-ARRIVAL` (`2941bc91f`) | `tools/coreperf/profiles.py` has **no** `client_tick_arrival` at all — the fix creates it, so the pre-image is absence and no patch can express it | `tools/coreperf/test_profiles.py` `test_client_tick_arrival_reports_each_window_distribution` cannot be the control (it imports the new name); needs a pre-fix retained receipt as golden input |
| `CIV-IRON-CAPTURE-OUTPUT-COLLECTED` (`d7f98cd4`) | `tools/harness/backends/civilization_seed_qa.py` before the iron menu hold: the pre-menu read and the direct assignment | `harness:civilization-seed-qa:iron_smelted` and `test_furnace_output_collected_before_the_menu_does_not_abort_the_run` |
| `HARNESS-REGRESSIONS-CLIENT-REBUILDS` (`a39164dc9`) | `tools/harness/regressions.py` that **built** a release server when no pin was registered, then pinned | `ReleasePinTests.test_pin_of_another_tree_is_refused`, `…test_missing_pin_is_refused_and_names_exact_pin_release_command`; the two pin-accepted refs stay green by design |
| `HARNESS-VOICE-RECONNECT-CAPABILITY-REGISTRATION` (`0857a48c5`) | `tools/harness/backends/screens_voice_qa.py` with the after-state-only predicate | `test_voice_reset_allows_fresh_capability_registration_only_after_reconnect` |
| `TEST-NATIVE-HEALTH` (`ef67d139e`) | `tools/harness/backends/native_matrix.py` whose `HealthInterval.finish` accepted an incomplete or lossy snapshot | `test_health_interval_requires_live_complete_below_budget_samples_and_clean_log` |
| `SETTLEMENT-AUTONOMY-VILLAGE-ROTATION-STARVES-TICK-GATES` (`44aa3d95a`) | `crates/mc-core/src/settlements/autonomy.rs` with the tick-numbered 20-tick gate and no `gate_due`; the row's own test calls `gate_due`, so it cannot compile against the pre-image | a new control test driving `autonomy::tick` through state both trees accept, red at the PROGRESS assertion (`SpainF3` is writing it); the clock-only reversal reds the batch bound instead, which is not the row's symptom |
