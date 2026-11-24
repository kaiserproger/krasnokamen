# FANOUT-ROSTER-SCAN-PER-VISIBLE-PLAYER-PER-VIEWER — fix-hunk map

The production-only patch `FANOUT-ROSTER-SCAN-PER-VISIBLE-PLAYER-PER-VIEWER.patch` is the fix's non-test
hunks, SHA-256 `patch:aecd75e12dd745501c8c297dfe5d6f4784589f0c6fc96ad778c4cfb4a1e4a6e0`. Reverse-applies
on the candidate (`git apply -R --check` on `071a68f8e4`, clean tree).

## The fix and the hunk the patch carries

`crates/mc-edge/src/plan/entity_interest.rs`, `broadcast_to`: the pre-fix body scanned
`snapshot.player_roster` twice (once for the recipient's mode, once for the subject's) on every call for a
visible player entity of every viewer. The fix answers both questions from the publish-wide
`HashSet<EntityId>` built once per publish in `TickPlanIndex::spectators`. The patch reverses the body
only; the 4-parameter signature and the set itself stay (the kept test reads `index.spectators`).

## The a/-side against the real pre-image

`git show e984a1118^:crates/mc-edge/src/plan/entity_interest.rs` has the pre-fix two-scan body with a
three-parameter signature. The a/-side is the real body with the kept fourth parameter
(`spectators`), which it does not read — a warning, not an error. Every difference:

1. the kept `spectators` parameter (README rule (b): the kept tests pass `&index.spectators`);
2. nothing else; the body is the pre-image text, no counter, no `#[allow]`.

## Why this control is honest

The registered ref is
`cargo:mc-edge:plan::plan_tests::player_visibility_tests::broadcast_to_answers_from_the_publish_set_not_the_roster`.
It calls `broadcast_to` directly with a roster that contradicts the publish set (`roster(3, 3)`: both the
recipient and the subject are game mode 3) and an empty set: the fixed answer is `true` (the set decides
player 1 sees the player), the pre-fix scan answers `false`. Red on the control is the row's own symptom —
the answer came from the roster instead of the publish index — not a counter increment. The equivalence
test (`spectator_index_matches_the_roster_scan_and_is_built_once_per_publish`) stays; its roster-scan
counter and its counter assertion are deleted, so it is green on both trees and is not evidence. The one
remaining assertion, `SPECTATOR_INDEX_BUILDS == 4`, pins one index per publish; it is green on both trees
too and is not evidence.

## Observed control and restore

Observed control and restore (`20261124T033006-ledger-promote-3d_0qdu4`, `tree_dirty: false`,
`reversed_files` = `plan/entity_interest.rs`):

- control (`ledger-1-run.log`) **FAIL** — `the publish set, not the roster, decides that player 1 sees
  the player` (the pre-fix two-scan body answered the roster);
- restore green (`ledger-3-run.log`).

Earlier control (a declared-but-never-incremented counter; superseded):
`.analysis/validation/20261124T003341-ledger-promote-4v784ye8`.

## What stays out of the row

`plan_player_changes` and the wire matrix are untouched; the fix changes only which input answers the
spectator question inside the fanout. No wall-clock claim is made.
