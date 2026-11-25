# BATTLE-LIVE-SPECTATOR-LEFT-BEHIND — proof state (promotion withdrawn)

Fix `c6fa94da3` is in the tree. **No promotion stands** (REVIEW-wave2-15 M2 withdrew it) and no control
patch is kept: the one that existed removed only the two `follow(...)` calls, which leaves the per-tick
`follow_cameras` rebind (called every tick from `tick/spectator.rs` via `tick/dimension.rs`) catching up
in <=1 tick, so the scene could not flip; it also omitted other hunks of the commit without (a)/(b)/(c)
reasons and carried invented a-side comments.

What a valid control needs: reverse **all** real non-test hunks of `c6fa94da3` (including whatever
introduced the per-tick rebind) with mechanical adaptations only, then >=3 control runs red on the
camera/gap assertion and >=3 candidate runs green.
