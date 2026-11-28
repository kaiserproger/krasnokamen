# D154 — the refreshed patches, audited against their Git pre-images

Owner D154/D154b, 2026-11-28. Source: `0bf88beb5` (PatchRefresh), merged as
`e0b771447`; 21 patch files changed (20 added, one modified). The acceptance
criterion is byte equality between each hunk's minus lines and their
corresponding lines in `git show <fix>^1:<path>`. Finding an identifier or a
lookalike line somewhere in the file is not enough.

## Result: 8 of 21 are not controls, and all 8 are `open`

| row | fix | old-side mismatch |
|---|---|---|
| `FALL-001` | `35308579c9` | Returns `false` from `dragon_egg` in `vanilla_interact.rs`, a file absent at `^1`. The genuine dispatch to reverse is in `tick/input/interact.rs`. |
| `PLACE-001` | `35308579c9` | Rewrites `spawn_protection.rs`, a file absent at `^1`, instead of reversing dispatch. |
| `PERF-RECEIPT-HAS-NO-CLIENT-OBSERVED-TICK-ARRIVAL` | `2941bc91f` | Returns an unavailable-status stub from `client_tick_arrival`, which the fix introduced. The reported red was a `KeyError`. |
| `CIV-IRON-CAPTURE-OUTPUT-COLLECTED` | `d7f98cd4` | Restores a fabricated `"source": "output_slot",` line; the parent uses `milestones.setdefault` and has no such line. |
| `HARNESS-REGRESSIONS-CLIENT-REBUILDS` | `a39164dc9` | Restores a `# No usable pin:` fallback and a direct path return; the parent built, checked, and pinned the binary. |
| `HARNESS-VOICE-RECONNECT-CAPABILITY-REGISTRATION` | `0857a48c5` | Restores `return (after["voice"] …` inside `voice_reconnect`; the parent has an inline verdict assignment, not this helper. |
| `SETTLEMENT-AUTONOMY-VILLAGE-ROTATION-STARVES-TICK-GATES` | `44aa3d95a` | Restores a `gate_due` stub using `tick.is_multiple_of(20)`; the parent has no `gate_due` function. |
| `TEST-NATIVE-HEALTH` | `ef67d139e` | Restores a weaker `HealthInterval.finish`; the parent has no `HealthInterval` class. |

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

All seven withdrawn by `48f50c842` are `open` on `main`. The last column
names the expected red or what prevents it from being tested; these are not
observed results from corrected reversals. The perf patch was withdrawn separately.

| row | real pre-image | expected red or missing control |
|---|---|---|
| `FALL-001` | `7fe4905282fc:crates/mc-core/src/tick/input/interact.rs` has neither `dragon_egg` call: `dig` goes straight to the `Finish` check, and the `use_item_on` chain ends at `item_frame::use_on`. The helper is absent in the historical parent but intentionally stays in the current control tree; the unrelated `painting::use_on` addition stays too. | `crates/mc-core/src/interact_parity_tests.rs:75` `interact_parity_dragon_egg_use_teleports`. `…conserves_block_before_falling` (line 590) calls the helper directly at 593, so it can stay green. |
| `PLACE-001` | `7fe4905282fc:crates/mc-core/src/tick/input/interact.rs`: the `dig` guard goes from `!plugin_player::allows_block` directly to `has_reservations_for_block`, without `spawn_protection::protected`. | `interact_parity_spawn_protection_with_operator`; verify it exercises the exact reverted dispatch. |
| `CIV-IRON-CAPTURE-OUTPUT-COLLECTED` | `ed1d50b9571b:tools/harness/backends/civilization_seed_qa.py`: direct assignment from `capture_iron_menu(probe, artifact, pos, advanced)`; the helper opens the menu and waits for the ingot, without a simulation hold or a pre-menu live inventory read. | `test_furnace_output_collected_before_the_menu_does_not_abort_the_run`; verify the live `civilization-seed-qa:iron_smelted` verdict separately. |
| `HARNESS-REGRESSIONS-CLIENT-REBUILDS` | `c7786910eaa6:tools/harness/regressions.py`: `_release_server` ran `cargo build --release` before pinning the result; it was not a fallback used only for missing pins. | `ReleasePinTests.test_missing_pin_is_refused_and_names_exact_pin_release_command`; the required red is the attempt to build instead of refusing. |
| `HARNESS-VOICE-RECONNECT-CAPABILITY-REGISTRATION` | `b97808d41f83:tools/harness/backends/screens_voice_qa.py`: the inline predicate checked `after["voice"]` for an all-false state; the parent has no `voice_reconnect` helper. | `test_voice_reset_allows_fresh_capability_registration_only_after_reconnect` is the oracle for the symptom, but it calls the new helper directly. A control over the real pre-image needs a caller-level ref that runs on both trees. |
| `TEST-NATIVE-HEALTH` | `e18e747624d7:tools/harness/backends/native_matrix.py`: neither `HealthInterval` nor its dispatch in the lane existed. There was no older `HealthInterval.finish` that accepted lossy samples. | No valid red measured. `test_health_interval_requires_live_complete_below_budget_samples_and_clean_log` references the new class directly. Exercise the lane with lossy metrics and assert the refusal through an entry point shared by both trees. |
| `SETTLEMENT-AUTONOMY-VILLAGE-ROTATION-STARVES-TICK-GATES` | `7925c7bdc6eb:crates/mc-core/src/settlements/autonomy.rs`: no `gate_due`; the existing ref calls it and cannot compile without it. The row identifies `9d1da7f8306e` as the original starvation pre-image; the clock-only reversal failed the batch bound, not progress. | Still needs a shared-entry test that drives `autonomy::tick` and goes red because a pending village's gate never advances, not merely because the batch bound changes. |

## Final byte-exact verdict (2026-11-28 18:35 host)

The `PatchRefresh` lane ran the authoritative comparison — each hunk's minus payload plus its
context block located contiguously in `git show <fix>^1:<path>`, insertion-only hunks judged on
the plus payload — and the split is **9 controls, 12 not controls**, not the 8 my earlier screen
reported. Merged as `5e2c27bdd`.

Not controls (all `open`, `fix_commit` reverted to the real commit, patch files kept as
evidence): `CIV-IRON-CAPTURE-OUTPUT-COLLECTED`, `HARNESS-CIV-GATE-BIRTH-CAPTURE-HITS-THE-WALL`,
`HARNESS-CIV-GATE-OBSERVER-WAIT-HANGS`, `HARNESS-EXCLUSIVE-WINDOW-SCENE-SLOT-DEADLOCK`,
`HARNESS-JAVA-MATRIX-26-1-2-EVENTS-NONE-ADAPTER-TYPEERROR`,
`HARNESS-JAVA-MATRIX-FOREIGN-PORT-AUTH-ACK`, `HARNESS-REGRESSIONS-CLIENT-REBUILDS`,
`HARNESS-VOICE-RECONNECT-CAPABILITY-REGISTRATION`,
`PERF-RECEIPT-HAS-NO-CLIENT-OBSERVED-TICK-ARRIVAL`, `PLACE-001`,
`SETTLEMENT-AUTONOMY-VILLAGE-ROTATION-STARVES-TICK-GATES`, `TEST-NATIVE-HEALTH`.
Each failure names the invented line: an `output_slot` read (`CIV-IRON`), a `_BabyLeftView`
message, an `OBSERVER_STAND_MARGIN` branch, a lock-order comment not in the pre-image, the
fix's own `xp` mapping used as the old side, a `selector.select(...)` loop, an invented build
fallback, a re-indented predicate, an invented `client_tick_arrival` body, `protected -> false`
where the file does not exist, `gate_due` absent tree-wide, and an invented lenient
`HealthInterval`.

Controls (9): `FALL-001` (rebuilt: only the two dispatch hunks removed, `0 10` in
`tick/input/interact.rs`, red at `interact_parity_tests.rs:79`, `..._conserves_block_before_falling`
green as allowed), `DIMENSION-SWITCH-KNOCKBACK-DISCONNECT`,
`HARNESS-CIV-GATE-LOCAL-SCAN-UNLOADED`, `HARNESS-CIV-GATE-PLANNER-READ-BLOCKED-BY-STATUS-DIALOG`,
`HARNESS-CIV-GATE-REMOTE-SCAN-UNLOADED`, `HARNESS-CIV-GATE-STORAGE-READ-TIMEOUT`,
`SETTLEMENT-HOUSE-BILL-STALLS-BELOW-COBBLESTONE-DEMAND`,
`VILLAGER-BABY-WADES-INTO-HEAD-DEEP-WATER`, `BATTLE-DISMOUNTED-RIDER-SKATES-TO-SWEEP-POINT`.
