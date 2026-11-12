# RAIDER-TARGET-SCAN-EVERY-TICK-COLLAPSE — cb95c270 hunk map onto main HEAD 8f74d443

Main authorized patch mode for this row: cb95c270's raiders.rs hunks no longer reverse on
HEAD (a9a0de49 and 60b70a27 edited inside them), and its fix_commit tree cannot build a
control (reversing its non-test hunks removes `SIGHT_CHECKS` and `Runtime.raider_targets`,
which raids_tests.rs uses). The fix patch holds only the HEAD equivalents of cb95c270's
non-test behaviour hunks.

Fix patch: `receipts/RaiderPromote/raider-fix.patch`,
`fix_commit = "patch:7142a8be1da970d315bdfa38456fe72df6b67115ca4ddbf285cb6aad6a4367ca"`
(sha256 of its non-test hunks = sha256 of the whole file; `_split_non_test` keeps it whole).

| cb95c270 hunk | what it does | HEAD equivalent | in the fix patch |
|---|---|---|---|
| raiders.rs @@ -139 (`let target = target_in(state, slot, &grid, 64.0)` -> `choose_target(...)`) | **behaviour**: held target, 1-in-10 search cadence, follow-range radius | raiders.rs:230-237 (the `choose_target` call) | **yes**. Reversed form: `let target = target_in(state, slot, &grid, 64.0);` plus `let held = target.map(\|(target, _)\| HeldTarget { target, unseen: 0 });`. The second line exists only because a9a0de49 (raiders.rs:238-240) reads `held`. A target from `target_in` was seen this tick, so a9a0de49's gate is a no-op and attacks match pre-cb95c270. |
| raiders.rs @@ -139, same hunk (`match held { insert / remove }`) | stores the memory that `choose_target` reads | raiders.rs:241-248 | no: storage only; with the call reversed nothing reads it (tests may) |
| entity_types.rs @@ -450 (vindicator/evoker 12, illusioner 18, pillager 32 `follow_range`) | **behaviour**: 26.1.2 FOLLOW_RANGE -> `sight_range`, the search radius | crates/mc-data/src/entity_types.rs:453-475 | **yes**, identical text |
| raiders.rs @@ -75 (`SEARCH_INTERVAL`, `UNSEEN_MEMORY_TICKS`, `HeldTarget`, `candidate`, `search_due`, `choose_target`) | definitions; behaviour only through the call | raiders.rs:81-180 (`SEARCH_INTERVAL` became `pub(crate)` in 60b70a27 for rare_mobs_tests) | no: definitions (dead in the control; warnings only) |
| raiders.rs @@ -88 (comment, `raider_targets.retain`, `candidate()` refactor of the candidate list) | comment; pruning of gone ids; same candidate set | raiders.rs:184, 194-199, 203-207 | no: comment, storage pruning, behaviour-neutral refactor |
| raiders.rs @@ -228 (`illusioner(state, id, slot, target)`) and @@ -269 (`illusioner` takes the target, filters it to 16) | illusioner uses the raider's target | raiders.rs:337 (now `target.filter(\|_\| seen)` from a9a0de49), 374-381 | no: with the reversed call the target is the nearest visible within 64 and illusioner keeps it only within 16, the same victim as the old `target_in(..., 16.0)` |
| special_mobs/mod.rs @@ -244 (`Runtime.raider_targets`) | storage field | special_mobs/mod.rs:249 | no: storage API the tests read |
| special_mobs/mod.rs @@ -454 (`SIGHT_CHECKS` thread-local + increment, `#[cfg(test)]`) | test probe | special_mobs/mod.rs:462-468 | no: test probe the tests read |
| raids_tests.rs, docs/COMPATIBILITY.md, docs/bugs/ledger.toml | tests/docs | — | no (kept test file / docs) |

No cb95c270 behaviour hunk lacks a HEAD equivalent.
