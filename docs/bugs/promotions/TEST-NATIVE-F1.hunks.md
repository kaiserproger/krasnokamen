# TEST-NATIVE-F1 — client reversal pair map

Fix: `client:7119fd2d1c8ac6c4be78e0084da85b92702c6f36` (`feat(client): accept F1 from the automation
seam`). The real fix predates the client rename (`f0a6abf6`, 2026-11-12): it arrived inside the bulk
commit `e0f9d563` (2026-10-30), whose other hunks are clouds, block outline, creative HUD and the torch
flame, so the whole commit cannot be reversed to build a control. Per the pre-rename route in
`docs/bugs/README.md` the control is a committed reversal on the current tip:

| commit | role | content |
|---|---|---|
| `48c08522505f82678c173673c0fbb55d27ce110f` | control | deletes exactly the two `e0f9d563` production hunks |
| `7119fd2d1c8ac6c4be78e0084da85b92702c6f36` | fix (re-land) | re-adds the same two hunks verbatim |

`git diff 48c08522 7119fd2d` is 9 added lines in `src/session/actions.rs`, identical to
`git diff e0f9d563^ e0f9d563 -- src/session/actions.rs`:

| original hunk (`e0f9d563`) | HEAD equivalent (`7119fd2d`) |
|---|---|
| `@@ -138,6 +138,14 @@` — `"hide_hud"` automation arm executing `UiIntent::Key { key: "F1", … }` | `src/session/actions.rs` `execute_ui` key match, same eight lines |
| `@@ -1097,6 +1106,7 @@` — `"f1" \| "hide_hud" => Some("hide_hud")` in `normalize_key` | `src/session/actions.rs` `normalize_key`, same line |

Every other `e0f9d563` hunk is omitted under reason (a): another row's behaviour (cloud layers, block
outline, creative HUD, torch flame and the renderer files), and reversing it would red a different
row's assertion. No test hunk: the client test
`session::mining_start_tests::f1_key_toggles_hide_hud_through_the_ordinary_key_path` already exists at
the pair's base (`a5bd1f5e`) and is unchanged by both commits. No a/-side adaptation.

## Control

- Pinned control client `pins/48c08522505f-client-x86_64-unknown-linux-gnu-incr`: the
  `native-v2-input-camera` scenario must be **red** at `f1_automation_key_accepted` (the client's
  `minecraft_press_inputs` refuses `key.keyboard.f1`: "unsupported ordinary key").
- Pinned restore client `pins/7119fd2d1c8a-client-x86_64-unknown-linux-gnu-incr`: the same scenario
  must be **passed** with the same assertion true.
- `client:krasnokamen-client:session::mining_start_tests::f1_key_toggles_hide_hud_through_the_ordinary_key_path`
  runs green in the client checkout that contains the fix; it is not the proving ref (the scene is),
  so the row names the scene in `proving_refs`.

The scene's `f1_automation_key_accepted` assertion is `ProbeSteps.call_checked`: before it, a refused
key aborted the probe with no assertion entry, so a control could never be red on the row's symptom.
