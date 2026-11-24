# NATIVE-DEATH-TILT-PIVOT — hunk map for the tip reversal pair

Promotion receipt: `promotions/NATIVE-DEATH-TILT-PIVOT.json`.
Client pair: reversal `client:f2bcb7f2cb` → re-land `client:c933f82b68` (the row's `fix_commit`).

## Original commit

`client:45ceeafb` "fix(entities): death tilt turns about the feet on vanilla's deathTime curve"
(2026-11-11) predates the client rename (`f0a6abf6`, 2026-11-12), so `pin-release --client` cannot
build its parent and the control is a reversal on the current tip.

Non-test files: `src/entities/local.rs` (1 hunk), `src/entities/presentation.rs` (2 hunks).
Test/doc files of the commit, never reversed: `src/entities/animation_tests.rs` (+82),
`src/entities/crowd_tests.rs` (+42), `docs/ENTITY_MODELS.md` (+4).

## Hunks

| # | original hunk | content | disposition |
|---|---|---|---|
| 1 | `src/entities/local.rs @@ -109,7 +109,8 @@` | the local avatar's `death_tick` became `map(\|_\| e.death_tick.unwrap_or(tick))` with a comment, so the third-person player advances its tilt | REVERSED at `local.rs:115-119`: the pre-image `e.death_tick = player.health.filter(\|h\| *h <= 0.).map(\|_\| tick);` restored text for text, comment removed. `git blame -C -C -L 115,119` on `b11ca37604` names `45ceeafba` for the logic; the one-liner was later reformatted by rustfmt, so a line-level blame can name the formatting commit instead — the re-land commit itself names the restored line |
| 2 | `src/entities/presentation.rs @@ -923,10 +923,9 @@ fn root_matrix` | the origin-pivot block replaced by `matrix *= death_tilt(&entity.type_name, tick.saturating_sub(death), partial, vanilla)` | REVERSED at `presentation.rs:1237-1239`: the pre-image `let a = ((tick.saturating_sub(death) as f32 + partial) / 20.).clamp(0., 1.);` and `matrix *= Mat4::from_rotation_z(a.sqrt() * std::f32::consts::FRAC_PI_2 * vanilla);` restored. The hunk's own condition `.filter(\|_\| matches!(binding.style, RootStyle::Living))` came with `45ceeafb` itself (`git show 45ceeafb -- src/entities/presentation.rs`, the `+    if let Some(death) = entity.death_tick.filter(...)` line); `9a645aa99` later re-wrapped it across lines and `e593fa045` widened the set with `RootStyle::SulfurCube`. The widened condition is kept — reason (c) for that text only: a later commit rewrote it, and the zombie is `Living` on both sides, so it cannot change the observed frame |
| 3 | `src/entities/presentation.rs @@ -935,6 +934,35 @@` (new `LIVING_FEET` + `death_tilt`) | the feet-anchored vanilla curve | NOT reversed — reason (b): the retained ref `entities::animation_tests::death_tilt_turns_about_the_feet_on_the_vanilla_curve` (`animation_tests.rs:3304`) calls `presentation::death_tilt` at `:3382` and `:3386`; deleting it stops the control from compiling. The production mesh path no longer calls it, which is the behaviour that flips |

## a/-side adaptation

None. Both reversed files take their original pre-image text verbatim; no line outside the pre-image
is introduced. The only text that differs from the historical pre-image is later commits' own lines
kept around it (`RootStyle` filter, `let vanilla` binding), named under hunk 2 as reason (c).

## Proving ref

`harness:scene-shot:native-death-tilt` → `death_tilt_t00`
(`tools/harness/backends/death_tilt_oracle.py`): the silhouette's top row and tipping-side column at
the first dead tick must match Java 26.3 `(159, 548)` within `TOLERANCE = 2` px, and each later step
`t01..t18` against its own row of the same table (`t10` is `(385, 277)`). The scene films one zombie
killed under `/tick freeze`.

Observed: on the reversal the scene fails at `death_tilt_t00`, and the scenario report records
**all 19** silhouette edges (`death_tilt_edges` in
`20261123T235523-native-death-tilt-4s49urqh/result.json`):
`t13..t18 = (179,521) (179,523) (178,525) (178,527) (177,529) (176,531)` against Java's
`(414,274)` — the body stays high and drifts sideways instead of tipping about its feet, which is the
row's pivot symptom, and `t00 = (143,543)` against Java `(159,548)` is the curve symptom.

The companion ref `entities::animation_tests::death_tilt_turns_about_the_feet_on_the_vanilla_curve`
is also red on the reversal: its first loop calls `presentation::root_matrix`
(`animation_tests.rs:3345`), and the old curve turns the body by about 14° at the probe point where
vanilla's is 0°. It is therefore not the row's proving assertion (the row maps the scene), but it is
not "green on both sides" either.
