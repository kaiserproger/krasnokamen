# PatchRefresh — rebuild stale `fix_commit`s as reverse-applying patches

Base: `3d5dd3f402f4091e213525ab9274e2a720459f65` (`main` when the lane started; the
2216e1069 tree the 25-row audit names is an ancestor). Worktree
`/run/media/kaiserroman/games/solaris-work/wt/PatchRefresh`, branch `wt/PatchRefresh`.

## Why every row needed this

Each named `fix_commit` is a **merge commit**, so `git diff <sha>` is empty and
`git show <sha> | git apply -R --check` can never succeed; `git diff <sha>^1 <sha>` is the
real fix, and none of those diffs reverse onto this head any more (files moved or were
rewritten). Patch mode is the only promotable form.

Patch convention: `a/` side = pre-fix tree, `b/` side = HEAD, so `git apply -R <patch>` on
this head yields the pre-fix tree. Each row's `fix_commit` digest is `ledger`'s own identity —
sha256 of the patch's **non-test hunks** (`tools.harness.regressions._non_test_digest`). All
patches here are production-only, so that digest equals `sha256sum <patch>` (verified for
every converted row). Header labels are `a/`…`b/` because `ledger._split_non_test` parses
`diff --git a/… b/…`; a `git diff -R` patch with `b/`…`a/` labels is rejected as unparseable.

How red was measured: copy `tools/` to a scratch dir, `git apply -R <patch>` there, run the
row's python refs; for crate refs apply the patch in the worktree, run
`tools/dev/mem-run.sh -m 6G cargo test -p <crate> --lib -- <filter>`, then restore. Green on
restore was measured at this head separately (see the crate section).

## Per row

| row | patch sha256 | reverse applies | change | red on reverse |
|---|---|---|---|---|
| BATTLE-DISMOUNTED-RIDER-SKATES-TO-SWEEP-POINT | `ec8536013f1f…` | yes | mc-core `battle/cavalry.rs`: the seated-only `continue` is removed | all three cargo refs FAILED: `a_dismounted_rider_keeps_his_foot_drive_under_a_chase`, `…_under_a_flank`, `a_rider_whose_horse_dies_walks_from_the_next_battle_tick` |
| CIV-IRON-CAPTURE-OUTPUT-COLLECTED | `8059aa89f611…` | yes | `capture_iron_menu` loses the simulation hold and the pre-menu output read; the scenario call site returns to the direct assignment | `CivilizationEvidenceTests.test_furnace_output_collected_before_the_menu_does_not_abort_the_run` ERROR `TimeoutError: civilization-seed-qa timed out waiting for furnace_iron_output_visible` |
| DIMENSION-SWITCH-KNOCKBACK-DISCONNECT | `383a4233a210…` | yes | mc-edge `server/pump.rs`: the `dimension-switch-explosion-knockback` branch returns | `explosion_queue_tests::same_tick_dimension_switch_discards_only_old_world_knockback` FAILED |
| ENTITY-READ-ROW-248-BYTES-COPIED-PER-ENTITY-TICK | — | — | **blocker** | cannot: the discriminating ref (`step_entity_borrows_its_row`) is a `fn(&ReadRow, …)` type assertion; reversing the borrow fails to compile |
| FALL-001 | `f16c4344dfb4…` | yes | mc-core `vanilla_interact.rs`: `dragon_egg` returns to an unhandled use (`false`, no teleport) | `interact_parity_dragon_egg_use_teleports`, `interact_parity_dragon_egg_conserves_block_before_falling` FAILED |
| HARNESS-CIV-GATE-BIRTH-CAPTURE-HITS-THE-WALL | `1f8eabeda9f2…` | yes | baby capture teleports onto the baby's own cell again (no `BABY_TELEPORT_SIDES`) | `ApproachRetryTests.test_a_baby_inside_a_house_is_observed_from_beside_it` FAIL: "the observer must stand beside the baby, never on its own cell: [(120.5, 71.0, -274.5)]" |
| HARNESS-CIV-GATE-LOCAL-SCAN-UNLOADED | `344c62641f24…` | yes | `_wait_scan_loaded` helper removed; only the remote branch waits for chunks | `test_local_building_waits_for_its_chunks_before_the_scan` FAIL: `'minecraft_scan_blocks' != 'minecraft_wait_for_loaded_block'` |
| HARNESS-CIV-GATE-OBSERVER-WAIT-HANGS | `905567b16467…` | yes | `move_observer`/`OBSERVER_MOVE_TIMEOUT` removed; `building_blocks` returns to the unbounded `probe.wait` | `test_an_observer_that_never_lands_cannot_consume_the_scenario` FAIL: `None != False : a landing the server clamps is a recorded miss, not a hang` |
| HARNESS-CIV-GATE-PLANNER-READ-BLOCKED-BY-STATUS-DIALOG | `9e81b641578b…` | yes | `_planner_status` back to single attempts; `_screen_on_top`/`_clear_covering_screen` removed | `PlannerReadRecoveryTests`: `test_a_status_dialog_over_the_view_is_closed_and_the_read_retried`, `test_a_dialog_that_never_closes_names_itself_in_the_storage_error` FAIL |
| HARNESS-CIV-GATE-REMOTE-SCAN-UNLOADED | `422515c9a3dc…` | yes | `capture_target` loses the beside-building teleport and the unloaded-face retry | `test_a_remote_building_is_observed_after_the_observer_moves_there` FAIL: "the observer must move to the site before its faces are scanned" |
| HARNESS-CIV-GATE-STORAGE-READ-TIMEOUT | `777c96533d4d…` | yes | `_read_storage_container` back to a `startswith`-only reply match | `test_a_console_reply_embedded_in_a_perf_line_is_accepted` ERROR `TimeoutError: storage read of block 98,67,-155 got no reply in 2.0s` (the reply was in the log tail) |
| HARNESS-EXCLUSIVE-WINDOW-SCENE-SLOT-DEADLOCK | `2128a2f69e10…` | yes | `mem-run.sh` stops exporting `KRASNOKAMEN_EXCLUSIVE_WINDOW`; `scene-slot.sh` loses the refusal and the inherited-slot reuse | `tools.dev.test_scene_slot`: 5 failures incl. every row ref (`0 != 75` refusal, reuse, nested marker) |
| HARNESS-JAVA-MATRIX-26-1-2-EVENTS-NONE-ADAPTER-TYPEERROR | `59aab562f2b1…` | yes | `JavaMatrixClient.normalize` stops deriving `events` from `received_system_chat` (`events` is None again) | `test_java_matrix_client`: 8 failures incl. all three refs |
| HARNESS-JAVA-MATRIX-FOREIGN-PORT-AUTH-ACK | `fcbce6db9083…` | yes | `runtime._wait_port` accepts a bare `_probe_port` again (no listener-ownership proof) | `ReadinessCleanupTests.test_wait_port_does_not_accept_a_foreign_listener`, `…test_wait_port_child_exit_wins_over_foreign_listener` FAIL |
| HARNESS-REGRESSIONS-CLIENT-REBUILDS | `c245a8f6e989…` | yes | `_release_server` falls back to a per-run `cargo build --release` instead of refusing | `ReleasePinTests`: 9 tests red ("must never build a release server"), incl. `test_pin_of_another_tree_is_refused`, `test_missing_pin_is_refused_and_names_exact_pin_release_command`, `test_explicit_unregistered_pin_is_refused_without_building`; the two pin-accepted refs stay green by design |
| HARNESS-VOICE-RECONNECT-CAPABILITY-REGISTRATION | `388b240efe89…` | yes | `voice_reconnect` returns to the after-state-only predicate (a fresh registration passes) | `test_voice_reset_allows_fresh_capability_registration_only_after_reconnect` FAIL `False != True` |
| PERF-RECEIPT-HAS-NO-CLIENT-OBSERVED-TICK-ARRIVAL | `73aa68e50526…` | yes | `client_tick_arrival` returns the pre-fix absence instead of the per-window distribution | `test_client_tick_arrival_reports_each_window_distribution` ERROR `KeyError: 'windows'` |
| PLACE-001 | `1d6aa62e0cec…` | yes | mc-core `spawn_protection::protected` returns false again | `interact_parity_spawn_protection_with_operator` FAILED |
| SETTLEMENT-AUTONOMY-VILLAGE-ROTATION-STARVES-TICK-GATES | `3edbf86e2524…` | yes | `gate_due` back to the tick-numbered 20-tick gate (the per-village clock is gone) | `every_pending_village_takes_its_gate_clock_turn` FAILED (queue of 8 starves half its villages) |
| SETTLEMENT-HOUSE-BILL-STALLS-BELOW-COBBLESTONE-DEMAND | `a56fc51469c3…` | yes | `upgrade_project` back to the whole-bill union (`with_joint` of the House reserve) computed per building | `growth_tests`: `the_house_reserve_is_netted_per_item_of_the_upgrade_bill`, `a_house_upgrade_is_not_reserved_against_itself`, `a_birch_village_admits_its_upgrade` FAILED; the two refusal refs stay green |
| SVC-BRIDGE | — | — | **blocker** | 3174-line port commit with no focused pre-image at HEAD; the graphical refs need Fabric+SVC artifacts absent on this host |
| TEST-NATIVE-HEALTH | `199573190211…` | yes | `HealthInterval.finish` accepts an incomplete/lossy snapshot again (no overrun, skipped-tick, gap, latest-tick or instance checks) | `test_health_interval_requires_live_complete_below_budget_samples_and_clean_log`: 6 red assertion/subtest cases |
| TIME-LEGACY-FROZEN-CLOCK-CONVERSION | — | — | **blocker** | fix created `native_checkpoint/game_clock_upgrade.rs`; its pre-image is absence, so any `a/` side is a stub (the withdrawn `patch:f55fc4af`) |
| VILLAGER-BABY-WADES-INTO-HEAD-DEEP-WATER | `2cfa1d18cfbf…` | yes | mc-core `allocator::body_clear` stops extending the scan to the head row for a one-block body | `routes_wade_through_foot_deep_water_but_never_deeper_or_lava` FAILED |
| HARNESS-ADMIN-TEST-TEMP-DIR-LEAK | — | — | **test-only** | the guard is test scaffolding in `crates/mc-edge/src/admin_tests.rs`; no production hunk, row left open |

Converted: 21 of 25. Blockers: ENTITY-READ (compile-level control), SVC-BRIDGE (no focused
pre-image + absent Fabric artifacts), TIME-LEGACY (module created by the fix). Test-only:
HARNESS-ADMIN-TEST-TEMP-DIR-LEAK.

## Crate-section results

| tree | command | result |
|---|---|---|
| HEAD (green baseline) | `cargo test -p mc-core --lib -- battle::cavalry::tests::a_dismounted_rider_keeps_his_foot_drive battle::cavalry::tests::a_rider_whose_horse_dies settlements::allocator::tests::routes_wade… settlements::autonomy::tests::every_pending… settlements::construction::growth_tests::the_house_reserve` | `ok. 7 passed` |
| cavalry reversed | same binary, `battle::cavalry::tests` | all three refs FAILED: `a_dismounted_rider_keeps_his_foot_drive_under_a_chase`, `…_under_a_flank`, `a_rider_whose_horse_dies_walks_from_the_next_battle_tick` |
| villager reversed | `settlements::allocator::tests::routes_wade` | `routes_wade_through_foot_deep_water_but_never_deeper_or_lava` FAILED (1 failed) |
| autonomy reversed | `settlements::autonomy::tests::every_pending` | `every_pending_village_takes_its_gate_clock_turn` FAILED (1 failed) |
| house bill reversed | `settlements::construction::growth_tests` | 6 failed / 51 passed: `the_house_reserve_is_netted_per_item_of_the_upgrade_bill`, `a_house_upgrade_is_not_reserved_against_itself`, `a_birch_village_admits_its_upgrade` + three non-ref growth tests |
| HEAD (green baseline) | `cargo test -p mc-core --lib -- interact_parity_tests::interact_parity_dragon_egg_use_teleports …conserves_block_before_falling …spawn_protection_with_operator` | `ok. 3 passed` |
| FALL-001 + PLACE-001 reversed | same three filters | all three FAILED (0 passed; 3 failed) |
| HEAD (green baseline) | `cargo test -p mc-edge --lib -- explosion_queue_tests` | `ok. 10 passed` |
| dimension reversed | `cargo test -p mc-edge --lib -- explosion_queue_tests` | `same_tick_dimension_switch_discards_only_old_world_knockback` FAILED; 9 passed |

## Notes for Main

- No row `status` was changed; none was promoted. Only regression/repro references and
  `fix_commit = "patch:<digest>"` were rewritten.
- Patches are behavioural reconstructions of the fix *at HEAD*, not the original commit diff:
  the files moved, so e.g. the cavalry patch reverses only the seated guard's `continue`,
  keeping the later `exposure.exposed` bookkeeping, and the autonomy patch restores the
  tick-numbered gate through the helper the HEAD test names (`gate_due`), because that test
  identity must stay runnable.
- Two rows keep one ref green under the reverse by design: `HARNESS-REGRESSIONS-CLIENT-REBUILDS`
  (the two "pin accepted" refs test the surviving pin path) and `DIMENSION-SWITCH`
  (`dimension_reset_fences_queued_old_world_explosions_at_real_writer` exercises the writer
  fence, a different sub-fix).
