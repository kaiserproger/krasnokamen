# BATTLE-LIVE-RESPAWN-FAR-FROM-FIGHT — proof state (promotion withdrawn)

Fix `f90997cc1` is in the tree. **No promotion stands** (REVIEW-wave2-15 M1 withdrew it) and no
control patch is kept: the earlier behaviour-only forms were hand re-breaks and are deleted.

Measured with the same scene and two pins: candidate `f75930bf7ffc` gave `min_enemy_distance = 11.17`
(one run), while the same candidate binary in two other runs (`032120`, `040219`) gave the control's
own outcome (`file=9`, `30.17`, `holder_sees_an_enemy_after_respawn = false`). The pre-image's contact
ref is green on the reversal because `contact_file` already existed on the a-side, i.e. **this fixture
never executes the fix's branches** (own regiment out of contact, broken, gone).

What a valid control needs: a fixture where `contact_file` returns `None` for the holder (his regiment
broken or out of contact), the witness counting only living, non-routed enemies anchored at the current
formation position, and >=3 candidate runs green / >=3 control runs red. The witness bound (20 blocks)
is unproven against that fixture and is recorded only as a measurement, not as a pass criterion.
