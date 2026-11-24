# MOB-YAW-FACES-POST-COLLISION-VELOCITY — evidence map

The fix is `789097be5` (in tree, eleven cargo refs green, probed at
`.analysis/validation/20261124T092509-ledger-probe-cadi2cru`). It has **no reversing patch**: the
commit's non-test hunks no longer apply in reverse at HEAD — later commits restructured
`entity_motion.rs`, `blueprint.rs`, `special_mobs/raiders.rs`, `tick/dying.rs` and
`tick/entity_phase.rs`, and `git apply -R --check` of
`git diff 789097be5^..789097be5 -- <those files>` refuses at `entity_motion.rs:13`, `blueprint.rs:136`,
`raiders.rs:293`, `tick/dying.rs:91` and `tick/entity_phase.rs:48` (checked 2026-11-25). So commit-mode
promotion cannot build its cargo red, and the row stays `open` (fixed-unpromoted) until a reversing
patch is regenerated (the bed-patch procedure: `git apply -R --3way`, resolve the context-only
conflicts, `git diff -R` restricted to those files) or the pre-image tree `789097be5^` is pinned and
the scene run on it as a graphical control.

## Client proof (candidate)

`native-v2-mob-yaw-blocked-step` (scene-shot, windowed native), registered in `scenarios.py` with
`tools/harness/mob_yaw_blocked_step_scene.json` and the `chase_facing` analysis in
`tools/harness/backends/mob_motion_oracle.py`:

- fixture: a bare stone platform at y 69/70; one oak-fence line at x=6 (z -8..8), which zeroes the
  chase step's +x half while a 1.5-block-high fence still lets sight pass above it; a survival player
  with resistance stands at (9.5, 70, 4.5), a zombie is summoned against the fence at (5.2, 70, 0.5).
- five `timed_entities` samples three ticks apart give the zombie's received position and body yaw.
- `chase_facing` computes, per interval with real displacement, the *wanted* bearing (to the player)
  and the *resolved* step bearing (the measured displacement, which is what the wire carries — a
  remote entity's velocity field is not sent), and per sample the received yaw's gap to the wanted
  bearing.
- assertions: `mob_yaw_blocked_step_deflected` (the fixture really produced a deflected step: at least
  30° between wanted and step over at least three moving intervals — otherwise a yaw assertion could
  not tell a wanted-direction body from a step-facing one), `mob_yaw_blocked_zombie_faces_wanted`
  (max gap ≤ 15°), `mob_yaw_blocked_zombie_yaw_rate` (≤ 90°/tick, vanilla `MAX_TURN`).

The first run (`20261124T201415-…-76zh66jm`) failed `mob_yaw_blocked_step_deflected` only because the
oracle then read the *velocity field*, which the wire leaves zero; the samples showed the slide itself
(x pinned at 6.075 while z advanced 0.83 → 2.93 and yaw tracked 312° → 294°, the wanted bearing). The
oracle now uses the measured displacement; the unit tests cover both a wanted-facing body (green) and a
step-facing body (red at `…_faces_wanted`).

Receipt on the promotion tree (pin `660717828dd4`, client `530556e864c2`): see the handout's validation
table. Limits: one zombie, one fence and one camera; the comparison is against the local player's
position, so the player must stay still (the fixture does) and the head yaw is not asserted.
