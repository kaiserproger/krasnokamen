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
| 1 | `src/entities/local.rs @@ -109,7 +109,8 @@` | the local avatar's `death_tick` became `map(\|_\| e.death_tick.unwrap_or(tick))` with a comment, so the third-person player advances its tilt | REVERSED at `local.rs:115-119`: the pre-image `e.death_tick = player.health.filter(\|h\| *h <= 0.).map(\|_\| tick);` restored text for text, comment removed. `git blame -C -C -L 115,119` names `45ceeafba` |
| 2 | `src/entities/presentation.rs @@ -923,10 +923,9 @@ fn root_matrix` | the origin-pivot block replaced by `matrix *= death_tilt(&entity.type_name, tick.saturating_sub(death), partial, vanilla)` | REVERSED at `presentation.rs:1237-1239`: the pre-image `let a = ((tick.saturating_sub(death) as f32 + partial) / 20.).clamp(0., 1.);` and `matrix *= Mat4::from_rotation_z(a.sqrt() * std::f32::consts::FRAC_PI_2 * vanilla);` restored. The `if let Some(death) = entity.death_tick.filter(...)` head keeps later commits' text — `9a645aa99` rewrapped it and `e593fa045` added `RootStyle::SulfurCube` — reason (c): a later commit rewrote that code, so the HEAD equivalent is the hunk's own expression, not its pre-image condition |
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

Observed: on the reversal the scene fails at `death_tilt_t00` — the old
`clamp((elapsed + partial)/20, 0, 1).sqrt() * 90°` curve already turns the body about the model
origin on the first dead tick, where vanilla is still upright. The probe stops at the first failed
step, so the later-step pivot difference is not separately reported; the assertion is still the
row's own symptom (a body tilted where vanilla is upright, then a wrong pivot and curve). The
companion ref `entities::animation_tests::death_tilt_turns_about_the_feet_on_the_vanilla_curve`
exercises the kept `death_tilt()` directly (`animation_tests.rs:3382`, `:3386`), which neither side of
the pair changes, so it cannot be red on the reversal and is not evidence here.
