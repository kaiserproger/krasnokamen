# ISSUE-1-18 — hunk map

Row: `ISSUE-1-18` (client, major) — now "Goat horns lean back instead of standing upright"
(REVIEW-wave2-12 M3 narrowed it from "Sheep and goat models wrong"). Refs:
`harness:scene-shot:issue1-18-sheep-goat` (`goat_horns_upright`).

## Real fix and its pre-image

Client `b7c48e1c7de46c34bc6611d438edfec3f37039c5` ("fix(client): keep the cursor across menu drags; goat,
spider and sheep model parity", 2026-11-08) authors the goat and spider rigs; the follow-up
`b9f2a96ee384a8281351622f0c554f99a2aece32` ("Fit goat horn depth and expose gait state", same day)
moves each horn pivot one sixteenth deeper. Both predate the client rename `f0a6abf6`, so the route is
the committed-reversal pair on the current tip:

- control `b6f3177ca1c8c9c4a8e7ba79b70ac447ee0e7128` — "test(client): restore the pre-fix goat and
  spider rigs for ISSUE-1-18's control". The three files are byte-exact pre-images from history:
  `assets/krasnokamen/models/entity/goat.json` = `43362918:assets/solaris/models/entity/goat.json`
  (94 lines; leaning `-12.5`-degree horns, ear/beard at rest rotations, top-level arm/leg bones),
  `spider.json` = `b7c48e1c^:assets/solaris/models/entity/spider.json` (81 lines; two-cube legs with
  vertical feet), `tools/entity_models.py` = `b7c48e1c^:tools/entity_models.py` (the x=0-only symmetry
  rule). No a/-side line was written by hand: the files come from `git show`, with only the stage0
  rebrand paths (`assets/solaris` → `assets/krasnokamen`) already applied by the rename.
- re-land `83eef0ba6759...` — "fix(client): author the goat and spider rigs at the 26.3 geometry
  (ISSUE-1-18)", the row's `fix_commit`. Its tree is identical to `184fcf8ce5` (the lane base).
- `git diff b6f3177c 83eef0ba` is exactly the re-land's own non-test patch over the three paths, which
  the ledger re-checks (`client_control_delta.fix_patch_id 8713ea1de7d8c7fa6c74b63b20f0e9cb94953915`).

## Carried hunks and their HEAD equivalents

| real commit | hunk | HEAD equivalent |
|---|---|---|
| `b7c48e1c` | `goat.json @@ -1,91 +1,439 @@` (whole model) | `assets/krasnokamen/models/entity/goat.json` at HEAD: the same body with the vanilla texture regions, upright horns, ears/beard at the source angles |
| `b9f2a96e` | `goat.json @@ -386,7` and `@@ -412,7` (`pivot` z `-3` → `-2`, both horns) | the two `-2` values at HEAD (`right_horn`/`left_horn` pivots) — the pair's a/-side restores `-3` |
| `b7c48e1c` | `spider.json @@ -1,80 +1,342 @@` (whole model) | `assets/krasnokamen/models/entity/spider.json` at HEAD, byte-identical to the commit's blob `3e381a1c` |
| `b7c48e1c` | `tools/entity_models.py @@ -192,9 +192,13 @@`, `@@ -202,8 +206,8 @@` | `symmetry_errors` at HEAD (`tools/entity_models.py:195-211`) is the post-change text verbatim (shared-root centre); the file's other differences are the stage0 rebrand of `assets/solaris` → `assets/krasnokamen` and `SOLARIS_TEXTURES` → `KRASNOKAMEN_TEXTURES` |

## Every other non-test hunk of b7c48e1c and why it is not in this pair

| file | hunks | what they do | reason |
|---|---|---|---|
| `README.md` | `@@ -100,6` | client-settings note | documentation (skipped by the patch split) |
| `docs/AGENT_TESTING.md`, `docs/ENTITY_MODELS.md` | `@@ -101`, `@@ -254`, `@@ -356` | gate and model notes | documentation |
| `src/automation/minecraft_catalog.json` | two one-line edits | tool descriptions mention `game_time`/loading readiness | (a) another row's observation/automation work in the same bulk commit |
| `src/minecraft/entities.rs` | `@@ -447,7` | `minecraft_list_entities` gains the same-thread `game_time` used by `mob_motion` | (a) other rows' observation work |
| `src/session.rs`, `src/session/observation.rs` | `@@ -297`, `@@ -1304`, `@@ -54`, `@@ -76`, `@@ -1213`, `@@ -1231` | cursor retention state and the observation/singleplayer reads | (a) ISSUE-1-14, promoted as its own pair (`5f01c22101 → f843981bd3`) |
| `src/ui/state.rs` | `@@ -450`, `@@ -576`, `@@ -1161`, `@@ -2050`, `@@ -2093`, `@@ -2250`, `@@ -2263`, `@@ -2291` | `drag_pickup` prediction and its release paths | (a) ISSUE-1-14 |
| `src/entities/solaris_models/tests.rs` | `@@ -60,6` | model unit tests | test file, kept unchanged on both sides |
| `src/ui/drag_tests.rs` | six hunks | drag tests updated with the cursor fix | test file |
| `tools/test_entity_models.py` | `@@ -0,0 +1,17` | new symmetry unit test | test file |

## Gate

`harness:scene-shot:issue1-18-sheep-goat` on the candidate core tree `bbd66034c` (server pin
`bbd66034c88b-2d648d04-server`), one control/restore pair of real native clients:

- control (`b6f3177c`, receipt `.analysis/validation/20261124T194605-issue1-18-sheep-goat-j_yewyf8`):
  FAILED — `goat_horns_upright: false`; the horn's left edge runs 465, 464, 464, 463, 463, 463, 462
  down seven rows (a 3-pixel lean), which the oracle rejects (span ≤ 1). The remaining assertions
  (`in_play`, `teleport_arrived`) are true; the run stopped at the first failed check, which is the
  row's own.
- candidate (`83eef0ba`, receipt `.analysis/validation/20261124T194645-issue1-18-sheep-goat-yccom7p7`):
  PASSED — every assertion true, horn edges 468, 468, 468, 468, 468, 468, 468.

The goat oracle raises when the horn silhouette is missing rather than recording a false assertion, so a
control that removed the horns would not be evidence; this control keeps the horns and restores their
lean, which is the pre-fix geometry the row reports.

### What this does not prove (review m3)

The same scene also renders a sheep (`sheep_present_minecraft:sheep`, a presence check only): no hunk, no assertion and
no control covers the sheep model, so the row no longer claims it. It is
`SHEEP-MODEL-UNPROVEN`, and the scene's `limits` says so. The scene ran **three times per side** (fix 468/468/468, pre-image 465..462 in each); a static rest pose is deterministic, but re-measure both pins before moving the box or the threshold.
