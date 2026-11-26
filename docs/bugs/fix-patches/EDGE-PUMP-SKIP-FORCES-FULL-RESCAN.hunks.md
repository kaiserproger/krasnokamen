# EDGE-PUMP-SKIP-FORCES-FULL-RESCAN — promotion patch map

Fix: a publication chain the pump feeds with every output it drains or stale-skips, and an index
that bridges the base most viewers hold instead of rescanning every viewer.

## Original commits (oldest first)

Ported from `perf1m-r3k5` onto the D123 candidate; each commit is in the candidate tree.

| commit | what it closes |
|---|---|
| `a02a366a5` | per-publish plan counters (`ViewerState::rescans`, `entities_scanned`, move tallies) — the observable this row's tests read |
| `de430453c` | deterministic pen scenario fixture (`plan/pen_scenario_tests.rs`) |
| `3088f1423` | the plan line is reachable, timed and free when off |
| `27c776117` | red reproduction for the skipped-publish rescan |
| `824d74231` | scope the skip test as an index-level reduction |
| `5406bf8ae` | `PublicationChain` + `TickPlanIndex::with_chain`: bridge a coalesced publish with a delta chain instead of rescanning |
| `c68d5966a` | compact per-dimension history, net delta built at bridge time (`PublicationHistory`) |
| `726a3f050` | faction net state, real travel handler, no id copies |
| `2d4f3a620` | a current viewer no longer preempts the chain for a lagging one |

## Carried hunks (the patch)

`crates/mc-edge/src/server/pump.rs` `@@ -99,9 +99,11 @@ next_publication` — the two
`history.record(&out)` calls (the stale-skip branch and the published branch). They are the real
lines of `5406bf8ae` in their HEAD form (the compacted `PublicationHistory::record(&out)` of
`c68d5966a`). Without them the chain is never fed, so no base can be bridged and every publish
rescans: exactly the pre-fix outcome, with the API the kept tests call still present.

## Omitted hunks and why

Every other production hunk of the nine commits is omitted as **(b) API the kept tests call, or
plumbing that produces the tested behaviour**: the kept tests construct `PublicationHistory`,
`TickPlanIndex::with_chain`, `ViewerState::rescans`/`entities_scanned` and the pen fixture, so
reversing those hunks stops the control from compiling instead of turning an assertion red.

| file | hunks | reason |
|---|---|---|
| `crates/mc-core/src/boundary/views.rs` | `@@ -44,6 +44,13` | (b) `EntityViews` helper the chain calls |
| `crates/mc-core/src/core.rs` | `@@ -559,6 +559,9`, `@@ -1333,6 +1336,7`, `@@ -1497,6 +1501,13` | (b) broadcast capacity the history is bounded by |
| `crates/mc-edge/src/faction_teams.rs` | `@@ -64,7 +64,7` | (b) faction delta the net bridge compares |
| `crates/mc-edge/src/fanout.rs` | `@@ -119,6 +119,28`, `@@ -143,23 +165,57`, `@@ -172,7 +228,28`, `@@ -206,6 +283,43`, `@@ -246,8 +360,8`, `@@ -272,8 +386,8` | (b) index build/selection the kept index tests call |
| `crates/mc-edge/src/plan.rs` | `@@ -76,7 +76,7` | (b) module export |
| `crates/mc-edge/src/plan/entity_interest.rs` | `@@ -32,6 +32,13`, `@@ -76,7 +83,26` | (b) `rescans_gap`/`rescans_no_history` counters the refs assert |
| `crates/mc-edge/src/plan/player_changes.rs` | `@@ -994,6 +994,10` | (b) rescan reasons |
| `crates/mc-edge/src/plan/shared_moves.rs` | `@@ -60,23 +60,51`, `@@ -90,20 +118,37` | (b) move tallies the plan line reports |
| `crates/mc-edge/src/plan/tick_index.rs` | `@@ -9,7 +9,7`, `@@ -35,7 +35,7`, `@@ -47,9 +47,278`, `@@ -96,6 +365,9`, `@@ -108,6 +380,9`, `@@ -167,48 +442,89` | (b) `PublicationChain`, `PublicationHistory`, `bridge`, `with_chain` |
| `crates/mc-edge/src/plan/viewer.rs` | `@@ -133,6 +133,18`, `@@ -208,6 +220,10` | (b) the counters and `rescan_reason` field |
| `crates/mc-edge/src/server.rs` | `@@ -2074,6 +2074,10` | test module registration (test code in a production file) |
| `crates/mc-edge/src/server/pump.rs` | `@@ -91,15 +91,24` (rest), `@@ -180,6 +189,10`, `@@ -209,7 +222,8`, `@@ -257,13 +271,14`, `@@ -638,6 +653,7`, `@@ -645,6 +661,41`, `@@ -675,7 +726,7`, `@@ -693,6 +744,7` | (b) the `history` parameter/threading the kept pump tests drive |

No hunk is omitted as (c): every original production line exists at the candidate's HEAD.

## Negative control

Reverse-applying this patch on the candidate removes the two `history.record` calls only. The
registered refs then run on a tree that compiles and where the chain is present but never fed, so
each viewer pays the full `entity_candidates::collect` scan on every publish: the pen-scenario and
pump-chain refs assert that count (`plan::plan_tests::pen_scenario_tests::*`,
`server::pump_chain_tests::*`) and go red there, not on a missing symbol.
