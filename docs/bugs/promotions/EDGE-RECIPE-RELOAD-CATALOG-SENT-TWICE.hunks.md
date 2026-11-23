# EDGE-RECIPE-RELOAD-CATALOG-SENT-TWICE — fix-hunk map

The row's fix is the recipe-reload part of `3c2124c08c3859436994bb2ad0459669d86f07b3`
(`fix(edge): reconcile the merged battle line with main's world-queue repair`), a merge-repair commit whose
whole non-test diff is eight files (`patch:2ca601d56e12c3fea8b9d3f706bd3c71ef92fdddfe09c41483674777f638eebc`).
This row's patch, `docs/bugs/fix-patches/EDGE-RECIPE-RELOAD-CATALOG-SENT-TWICE.patch`, is **exactly that
commit's non-test hunk for `crates/mc-edge/src/plan/player_changes.rs`** — one hunk,
`@@ -135,9 +135,11 @@`, SHA-256 of the non-test hunks
`patch:8b8ac42337fbd5b2de83f105991a134dc367b2a1bc00aa46908815ce676f2973`; it reverse-applies on the
candidate.

| Original production change | Current patch equivalent |
|---|---|
| `3c2124c08` `crates/mc-edge/src/plan/player_changes.rs` `plan_change`, `PlayerChange::RecipesReloaded`: push `packets.catalog.update` instead of `packets.update` and record `viewer.recipe_catalog = Some(Arc::clone(&packets.catalog))`, so the following book send does not repeat the catalog | Byte-identical at HEAD: the patch's only hunk; b/-side is HEAD's three lines (comment, `push_reliable(… catalog.update)`, `recipe_catalog`), a/-side the pre-merge `packets.update` without the recorded catalog. |

## The commit's other non-test files, reason (a) — not this row

`crates/mc-edge/src/plan.rs`, `plan/entity_interest.rs`, `fanout.rs`, `server/pump.rs`, `server/connection.rs`,
`server/registry.rs`, `dashboard.rs` carry the stage-0 merge reconciliation (main's lost-frame repair,
F's entity priority queue/rounds, the XpQueue latency map, keepalive latency recording). They belong to
`OBSERVATION-WALK-…`/`ENTITY-WIRE-VIEW-…`/the lost-frame and keepalive rows; reversing them here would turn
the control red on those rows' assertions and their kept tests.

Test hunks (`advancements_tests.rs`, `lost_entity_frames_tests.rs`, `plan_tests.rs`, `server/tests.rs`,
`server_session_security_tests.rs`, `server_view_distance_tests.rs`, `svc_service_tests.rs`,
`translation/adjacent/play_tests.rs`) stay in the candidate and are not part of the patch.

Proving ref: `cargo:mc-edge:advancements::tests::reloaded_recipes_are_announced_once_and_feed_the_next_book`
— with the a/-side the reload sends the catalog update under the wrong packet identity and does not record
it, so the following book send repeats it (the row's symptom); the fixed tree announces it once. Green on
the restore in `20261123T212044-ledger-promote-q4olz2jm`.
