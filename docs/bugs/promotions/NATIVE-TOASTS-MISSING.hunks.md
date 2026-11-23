# NATIVE-TOASTS-MISSING — hunk map for the tip reversal pair

Promotion receipt: `promotions/NATIVE-TOASTS-MISSING.json`.
Client pair: reversal `client:6c3ebb27ac` → re-land `client:e32052ade6` (the row's `fix_commit`).

## Original commit

`client:8deb8fd7` "feat(ui): vanilla advancement, recipe and unsecure-server toasts" (2026-11-11)
predates the client rename (`f0a6abf6`, 2026-11-12), so `pin-release --client` cannot build its
parent and the control must be a reversal on the current tip.

Files and non-test hunk counts: `src/inventory.rs` 1, `src/inventory/receive.rs` 1,
`src/session.rs` 2, `src/session/packets.rs` 2, `src/ui.rs` 1, `src/ui/advancements.rs` 1,
`src/ui/anim.rs` 1, `src/ui/packets.rs` 1, `src/ui/screens.rs` 1, `src/ui/state.rs` 8,
`src/ui/toasts.rs` 1 (new file), `src/ui/toasts_tests.rs` (test file, never reversed).

## What the pair reverses

| original hunk | content | disposition |
|---|---|---|
| `src/ui/state.rs @@ -3502,7 +3509,7 @@` | the `render` tail: `-        Ok(())` → `+        self.paint_toasts(content, out, transform)` | REVERSED — the one hunk whose removal takes the toast pixels out of the retained overlay; `git blame -C -C -L 3683,3685 src/ui/state.rs` names the current line as `8deb8fd78`, and the reversal restores that hunk's own pre-image `Ok(())` text for text |

## Hunks left out

| original hunk(s) | reason |
|---|---|
| `src/ui/toasts.rs` (new module: `ToastManager`, `Toast`, `paint_toasts`, the nine-slice, the advancement/recipe/system kinds) | (b) — the module is API the kept `src/ui/toasts_tests.rs` calls (`ToastManager`, `Toast::system`, `progress_map_order`); deleting it stops the control from compiling. Its only effect on the frame is through the reversed call above |
| `src/ui/state.rs` remaining 7 hunks (`Paint::Toast`, the `toasts`/`insecure_chat_warned` fields, `new()` defaults, the node-registration arm, the sound drain) | (b) — same tests call them, and the node registration is inert without the paint call |
| `src/ui/advancements.rs`, `src/ui/packets.rs`, `src/ui/screens.rs`, `src/ui/anim.rs` | (a) — the advancement/recipe toast rows' behaviour (they stay unit-covered and separate); `advancements.rs` also feeds the retained advancement stacking test |
| `src/inventory.rs`, `src/inventory/receive.rs`, `src/session.rs`, `src/session/packets.rs`, `src/ui.rs` | (a)/(b) — recipe-book notification plumbing, the login-packet raise and the `mod toasts` wiring: the raise is what the kept manager tests drive, and the plumbing belongs to the advancement/recipe rows |

## a/-side adaptation

None: the reversal restores the pre-image line `Ok(())` that the original hunk deleted, and the
re-land restores the added line. `git diff 6c3ebb27ac e32052ade6` is exactly the original hunk's
`+` side.

## Proving ref

`harness:scene-shot:native-toast` → `toast_panel_appeared`
(`tools/harness/backends/scene_shot_checks.py:196`): the panel's own rectangle
(x 621..1280, y 0..168 at GUI scale 3) must carry dark navy pixels two seconds after Play and none
eighteen seconds later. That is the row's symptom — no toast pixels — and it was red on the reversal
(20261123T225248-native-toast-oa0ho1kj) and green on the re-land. `toast_title_drawn` (:197) reads
the panel's yellow title ink and is the companion assertion, not the symptom.
