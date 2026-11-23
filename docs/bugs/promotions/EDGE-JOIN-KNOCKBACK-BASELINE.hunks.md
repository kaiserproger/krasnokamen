# EDGE-JOIN-KNOCKBACK-BASELINE — fix-hunk map

The row's fix is `d7d631092caa` (`fix(edge): fence knockback epochs at queue admission`), with
`2fe160103` completing the publication/registration orders. This curated HEAD-equivalent patch is
`docs/bugs/promotions/EDGE-JOIN-KNOCKBACK-BASELINE.patch`, SHA-256 of its non-test hunks
`patch:6f470534ef003f6c663be4ef1a4d556f6392449bcec324bfe815df96816a3cb0`; it reverse-applies on the
candidate.

A blanket reversal of `d7d631092` does not build: its production hunks add `Core::published_knockback_count`
and `ConnectionHandles.knockback_baseline`, which the pump reads and the row's own kept refs call
(`register(…, || server.hub.published_knockback_count(connection))`). The patch keeps the whole observation
API and reverses only the admission-fence decision:

| Original change | HEAD equivalent in the patch |
|---|---|
| `crates/mc-core/src/core.rs`: `KnockbackPublication`, `CoreInner.knockback_publication`, `Core::published_knockback_count` | **Kept, reason (b)**: the kept refs and the production registration closure call `published_knockback_count`; reversing it stops the control from compiling. |
| `crates/mc-core/src/core/tick_loop.rs` publication of the epoch map | **Kept, reason (a)/(b)**: `dimensions_tests.rs` and `explosion_queue_tests.rs` assert `player_knockback_ticks`, another row's behaviour. |
| `crates/mc-edge/src/server/registry.rs` `register`: `handles.knockback_baseline = published_knockback_count();` | **Reversed**: the a/-side keeps the closure parameter (the kept refs pass it) and seeds the baseline with `0` — the pre-fix first-sight seeding, so a pre-registration impulse is treated as skipped. `let _ = published_knockback_count;` is the a/-side adaptation that keeps the (b) parameter consumed. |
| `crates/mc-edge/src/server/pump.rs` `close_skipped_knockback`: first sight seeds `handles.knockback_baseline` | **Reversed**: a/-side seeds `0` (the pre-fix `unwrap_or(0)`), so the fence cannot suppress the pre-registration delta. |
| `crates/mc-edge/src/server/pump.rs` `publish_and_route`'s pre-registration filter (`last_tick == out.tick && count <= knockback_baseline`) | **Reversed**: a/-side has no such filter (the pre-fix code replayed the old publication's impulses), so an impulse published before registration reaches the new client. |
| `ConnectionHandles.knockback_baseline` field and `ConnectionHandles::new` | **Kept, reason (b)**: `crates/mc-edge/src/server/tests.rs:422` asserts `knockback_baseline == 7`; the field stays (written 0 by the a/-side). |
| The commit's many `*_tests.rs` changes | test files, not hunks. |
| `docs/architecture/runtime.md`, `docs/decisions/0013-core-model-v2.md` | documentation. |

## Proving refs

- `cargo:mc-edge:server::admin_tests::explosion_queue_tests::published_knockback_epochs_count_ticks_and_fence_delayed_queue_registration`
  — with the a/-side the delayed registration fences nothing and the pump's first processed old output
  lowers/mismatches the fence: red at `explosion_queue_tests.rs:747` (`assert_eq!(seen[&connection], 3, "old outputs must not lower the
  fence")`, observed `left: 1`, `right: 3`) (the row's symptom: impulses published before registration are counted as skipped), and the
  pre-registration impulse then reaches the client (`pre-registration impulses must not be delivered`).
- `cargo:mc-edge:server::admin_tests::explosion_queue_tests::first_processed_output_after_registration_still_closes_a_skipped_impulse`
  — stays green in the control: it asserts that a genuinely skipped post-registration impulse still closes,
  which the a/-side does not change. It is registered to prove the restore does not weaken that half.

Both are real-client-path-adjacent production tests (`Core` publications through the pump), not setup
assertions; the a/-side compiles and the red is an observed assertion failure.
