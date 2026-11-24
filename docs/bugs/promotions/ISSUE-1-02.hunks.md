# ISSUE-1-02 — hunk map

Row: `ISSUE-1-02` (client, major) — "Held tool floats rather than sitting in hand." Refs:
`client:krasnokamen-client:motion::tests::view_motion_hand_is_perfectly_still_at_rest`,
`harness:scene-shot:issue1-02-held-tool` (`held_tool_at_rest_is_still`).

## Real fix and its pre-image

Client `db2e7986dfedfc2a2784f855b4be9e3a4829de77` ("fix(motion): first-person rest pose sits on the
vanilla held-item anchor (ISSUE-1-02)", 2026-11-12). Its single production hunk is
`src/motion.rs @@ -704,25 +704,20 @@ impl ViewMotion` — `hand()` loses the time-based idle sway
(`(t * 0.9).sin() * 0.004 * idle`, etc.) and its `walking` argument, so the rest pose sits exactly on
the vanilla anchor `(side*0.56, -0.52, -0.72)`. The hunk **reverse-applies at HEAD**, so the pair is a
committed reversal of the real pre-image (no hand-port):

- reversal `582a6900489480aa9a884dbb2d3fc7b699329f69` — the a/-side restored verbatim (idle sway and
  the `walking` argument); the kept unit ref fails (`negctl/issue102-unit-control.log`).
- re-land `8c392a0ac14eaa6f74135eb85b33a8c44b27031b` — the fix again; row `fix_commit`; unit ref green
  (`negctl/issue102-unit-restore.log`).
- `git diff 582a690048 8c392a0ac1` is exactly the re-land's own non-test patch.

## Other hunks of db2e7986 and why they are not in the pair

| file | hunk | reason |
|---|---|---|
| `src/player/visual.rs` | `@@ -775,6 +775,76 @@ mod tests` | test hunk (the regression lives here); kept unchanged on both sides |
| `PLAYING.md` | documentation (the rest-pose paragraph) | (c) docs |

No production hunk other than `src/motion.rs` exists in the commit.

## Gate

`scene-shot`, scenario `issue1-02-held-tool` (`tools/harness/held_tool_rest_scene.json`): a native
client stands still on a frozen-tick stone pad holding a diamond pickaxe; three frames 40 ticks apart.
`check_held_tool_rest` requires the camera unchanged and `held_tool_at_rest_is_still`: the hand region
(right half above the hotbar, `(640, 0, 1280, 660)` at 1280x720) keeps the same pixels within a
2-pixel tolerance. The pre-fix idle sway moves the hand between frames, so the control's frames differ
there. Measured counts are recorded in `details.scenario.assertions` and
`held_tool_changed_pixels` in the receipt.
