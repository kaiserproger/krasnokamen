# FANOUT-ROSTER-SCAN-PER-VISIBLE-PLAYER-PER-VIEWER — patch and hunks

Promoted in patch mode from `docs/bugs/promotions/FANOUT-ROSTER-SCAN-PER-VISIBLE-PLAYER-PER-VIEWER.patch`;
SHA-256 of its non-test hunks is the row's `fix_commit`.

## What the fix is (commit `e984a1118`)

The publish already has every input of the answer: `TickPlanIndex` builds one `HashSet<EntityId>` of
spectator entities per publish and `broadcast_to` reads it instead of scanning `player_roster` (whose
`RosterPlayer` carries a 56-byte name) twice per visible player entity per viewer. The set is threaded
through `broadcast_to`, `changed`, `enter_leave`, `delta` and `entity_candidates::collect`; the
planner's `prepare` passes `&index.spectators`.

## The control patch (README:185, 219-245)

A blanket reversal of `e984a1118` is a compile break (README:203): the kept test calls
`entity_candidates::collect(..., &spectators(&out))` and `broadcast_to(..., &index.spectators)`, and the
reversed signatures would not accept them. The patch is therefore a **control adapter** that keeps every
call signature and reverses only the scanning function to the pre-fix body:

```rust
pub(super) fn broadcast_to(
    snapshot: &WorldSnapshot,
    entity: &EntityView,
    recipient: EntityId,
    _spectators: &std::collections::HashSet<EntityId>,   // kept for the call sites
) -> bool {
    …
    #[cfg(test)]
    ROSTER_SCANS.fetch_add(snapshot.player_roster.len(), std::sync::atomic::Ordering::Relaxed);
    let spectator_viewer = snapshot.player_roster.iter().any(…);   // pre-fix scan 1
    if spectator_viewer { … } else { !snapshot.player_roster.iter().any(…) }   // pre-fix scan 2
}
```

- (a) pre-fix behaviour: the two roster scans are the code that was on `f9a8d4559669`; the a-side is
  verbatim from `entity_interest.rs` before `e984a1118`, except the parameter name.
- (b) signature: the fourth parameter stays (renamed `_spectators`) so every kept call site compiles
  unchanged; the call sites are not part of the bug.
- (c) instrumentation: the `ROSTER_SCANS` counter exists in both trees (`#[cfg(test)]` static in the
  production file since the fix commit, `pub(super)`); the a-side adds the one `fetch_add` line. The
  counter is test-only, so it cannot change production behaviour.

## The red

The kept test
`tick_tests::player_visibility_tests::spectator_index_matches_the_roster_scan_and_is_built_once_per_publish`
publishes the mixed-mode fixture (recipient/subject modes 0/0, 3/0, 0/3, 3/3) and asserts
`ROSTER_SCANS` did not move; with the a-side `broadcast_to` every call adds `player_roster.len()` (5), so
the assertion fails. The equivalence half of the same test (index answer == roster scan) stays green on
both sides by construction — it is the guard that the set answer is the scan answer, not the red.
