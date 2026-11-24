# ISSUE-1-19 — hunk map

Row: `ISSUE-1-19` (client, major) — "Spider and zombie movement animation and speed wrong". Refs:
`cargo:mc-core:hostile_tests::zombie_chase_speed_matches_java_263_moving_interval`,
`cargo:mc-core:hostile_tests::zombie_chase_publishes_and_clears_aggressive_pose_metadata`,
`cargo:mc-core:journal_touched::tests::zombie_chase_pose_is_journaled_and_repaired_after_recovery`,
`client:krasnokamen-client:entities::animation_tests::zombie_arm_rotation_matches_the_26_3_animate_zombie_arms_formula`,
`client:krasnokamen-client:entities::animation_tests::spider_leg_swing_follows_the_26_3_spider_model_pairs`,
`harness:scene-shot:issue1-19-hostile-motion` (`zombie_chase_speed`).

## Real fix

Two commits closed the row's path:

- core `c0d78c11eb3acb4e43ca28d01b1e0ba463e2fb4c` ("feat: port client bug probes and movement tuning",
  2026-11-07): the zombie blueprint's `speed: 3.8` becomes `2.4` (Java 26.3 integrated-scene chase,
  ~0.116 blocks per client game tick), the zombie chase pose is published as the aggressive mob flag
  bit and cleared when the target is lost or on restore, and the recover/journal paths carry it.
- client `ebe5602316566264a218a19be2e539a53b551021` ("fix(entities): zombie arms and spider legs use
  the exact 26.3 formulas (ISSUE-1-19)", 2026-11-12, two minutes before the rename `f0a6abf6`): the
  `ZombieModel.setupAttackAnimation`/`AnimationUtils.animateZombieArms` formula and the
  `SpiderModel.setupAnim` per-pair leg offsets.

`fix_commit = patch:3fc6a9b98fa6edba355f793ae2ff042265b4c616ad1ddbf3d2b8b8ebbaf814eb`, the non-test
hunks of `docs/bugs/fix-patches/ISSUE-1-19.patch`. The patch is the only core hunk with a HEAD
equivalent; the rest are omitted with the classes below.

## Carried hunk

| real hunk | HEAD equivalent |
|---|---|
| `crates/mc-core/src/blueprint.rs @@ -916,8 +916,8` (`speed: 3.8` → `2.4` with its comment) | applies verbatim at HEAD (`git apply -R --check` passes on the candidate); reversing it restores 3.8 |

## Omitted hunks of c0d78c11e and why

| file | hunks | what they do | reason |
|---|---|---|---|
| `crates/mc-core/src/hostile.rs` | `@@ -113,68 +113,80` (the `refresh_targets` closure plus the dropped-target loop), `@@ -209,6` (`retaliate` publishes), `@@ -447,6 +460,29` (`set_zombie_aggressive`) | publish/clear the aggressive flag | (c) later commits rewrote them: `3bd6e8f04` split the write into `zombie_arms_update`/`set_zombie_arms` behind the blueprint's `aggressive_arms` capability and `1ee87857b` extracted the entity-phase owner, so no HEAD equivalent of those hunks exists. At HEAD the same behaviour is `hostile.rs:525` plus the per-slot sync at `tick/entity_phase.rs:611-612` |
| `crates/mc-core/src/tick/entity_phase.rs` | `@@ -1132,6` (publish on acquire) | same | (c) rewritten into the per-slot `contains_key` sync at `tick/entity_phase.rs:611` |
| `crates/mc-core/src/core/recovery.rs` | `@@ -473,6` (clear the flag on restore) | same | (c) HEAD's `restore_entity` no longer carries it (`grep aggress core/recovery.rs` is empty): the per-slot sync clears a restored zombie on its next hostile pass |
| `crates/mc-core/src/commands.rs`, `crates/mc-core/src/command_execution.rs` | `DebugMenu` variant, parser and handler | `/debug menu` operator probe | (a) another feature of the same bulk commit (probe tooling, not the row) |
| `crates/mc-core/src/tick/containers.rs`, `mounts/menus.rs`, `settlements/ui_menus.rs`, `player_reservations_tests.rs` | several | other probes/rows of the same bulk commit | (a) |
| `crates/mc-core/src/entity_motion_tests.rs` | `@@ -142,7` | the tuned-speed expectation | test hunk (kept both sides) |
| `crates/mc-core/src/hostile_tests.rs`, `journal_touched_tests.rs` | three new tests | the row's fence | test hunks (kept both sides) |
| `tools/harness/backends/*.py`, `tools/harness/scenarios.py`, `tools/harness/test_*.py` | many | gates of other rows plus the `mob_motion` step and its oracle | (b) API the kept scene calls (`mob_motion_oracle.measure` and the `mob_motion` handler in `scene_shot_execute.py` were added by this commit): reversing them stops the control from running |

## The client half (ebe56023)

Cross-repository, so the core patch cannot carry it. Its hunks are
`src/entities/presentation.rs` (`zombie_arm_rotation`, `spider_leg_swing` and their call sites),
`docs/ENTITY_MODELS.md` and `src/entities/animation_tests.rs`. It is documented here because the row
covers the animation as well as the speed:

- The two client unit refs registered on the row are that commit's own tests; they pass on the
  candidate.
- No pixel claim is made from a reverted client. The pin `184fcf8ce5d3-7c3c198b` named in earlier
  drafts was a **composite dirty tree** (it also carried ISSUE-1-18's goat pre-image, which is why the
  goat scene run `20261124T191231` on it showed the leaning horns), and its ISSUE-1-19 run
  `20261124T191336` FAILED before the arm check was judged (`expected exactly one minecraft:zombie,
  got 2`, the fixture zombie still listed). What is measured is the fixed client only: the raised-arm
  window (665,295–730,340, ≥800 px) reads 1776–1878 px across the candidate runs, and that window is a
  rest pose. The formula change alters the *aggressive* arm terms and the spider leg offsets, which
  this scene does not drive at that point, so it carries no red control — see
  `SPIDER-GAIT-AND-ANIMATION-UNPROVEN`.
- A faithful client reversal of those hunks exists in history (`ebe56023^`) but the call sites were
  rewritten by `ebe85470` (shield-arm exception) and `54b79f73` (stage0 merge drift), so the
  hand-ported pair would test the port rather than the commit; the speed hunk is the row's observable
  symptom and is what the control reverses.

## Gate (REVIEW-wave2-12 M1/M4 rework, 2026-11-25)

The scene drives one real AI chase: the player drops to survival, a helmeted zombie is summoned 20
blocks out, and three samples 30 ticks apart span **61-69 client ticks** (the server's position sample
period is 3 ticks, so only a window many periods long measures a rate rather than the period).
`mob_chase` records four verdicts from that one window; the row's proving assertion is
`chase_speed_zombie`.

| assertion | candidate (3 runs) | control (3 runs) |
|---|---|---|
| `chase_speed_zombie` (0.105..0.128 blocks/client tick) | **0.1200, 0.1200, 0.1182** | 0.1807, 0.1814, 0.1807 — all outside |
| `chase_approach_zombie` (>= 3 blocks closed) | 7.56, 8.28, 7.92 | 9.01, 9.92, 9.26 |
| `chase_aggressive_zombie` (mob-flags bit 2) | set (flags 4) on first and last | set — a 3.8-speed chaser still hunts |
| `chase_identity_zombie` | one uuid | one uuid |

The band comes from those numbers: the candidate sits at 0.1182-0.1200 (`2.4 / 20` per tick), the
control at 0.1807-0.1814 (`3.8 / 20`), so 0.105..0.128 keeps a 0.014 margin below the candidate
maximum and 0.052 above the control minimum. The approach and aggressive-bit checks are what stop a
wanderer or a NoAI fixture from passing the speed band by luck.

- candidate core `ca398831a`, server pin `ca398831afbb-server`; receipts
  `.analysis/validation/20261124T221450-issue1-19-hostile-motion-f0r_70x4`,
  `20261124T221613-…-2w7sgs15`, `20261124T221712-…-uk0v9ty0` — all PASSED.
- control = the same tree with the patch reversed (`speed: 3.8`), server pin
  `ca398831afbb-dcc5e8fe-server` (built with `KRASNOKAMEN_LEDGER_ROOT` at the candidate); receipts
  `20261124T221822-…-6tzipx6a`, `20261124T221923-…-3s0ps2g2`, `20261124T222033-…-wtf181vm` — each
  FAILED at `chase_speed_zombie`.
- full audit (control and restore, client receipt included):
  `.analysis/validation/20261124T222831-ledger-promote-9b4ctk2m/promote.json` (`dry-run ok`): the
  control side is red on both refs (`chase_speed_zombie` and `hostile_tests.rs:156` FAIL), the
  restore side green. The ledger builds its own control pin, so `KRASNOKAMEN_SERVER_BINARY` must be
  **unset** for a re-audit: an explicit candidate binary is refused against the control tree.

### Stored receipt vs the amended audit

`docs/bugs/promotions/ISSUE-1-19.json` was written at `e25d40529`, before the m1 naming: its
`negative_control` records the cargo ref as `unproven-panic` and the scene ref red. With the row's
`proving_assertions` naming `crates/mc-core/src/hostile_tests.rs:156` the same control is red on both
refs, as the amended audit `.analysis/validation/20261124T222831-ledger-promote-9b4ctk2m/promote.json`
shows, and the row declares both refs in `proving_refs` (LEDGER-CONTROL-COVERS-ONE-REF). The receipt is
refreshed by the next promotion pass of the row; nothing else in it is stale.

Why the block was needed: the fixture window alone passed a 3.8-speed server (the teleports dominate
the displacement), which is exactly the state the row was left in after wave 1.
