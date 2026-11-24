# NATIVE-REPLACEMENT-INPUT-CLEARED — hunk map for the committed reversal pair

Promotion receipt: `promotions/NATIVE-REPLACEMENT-INPUT-CLEARED.json`.
Client pair: reversal `client:365939accb` → re-land `client:2635fc2ac5` (the row's `fix_commit`).

## Why a reversal and not the pre-image pin

`895c63f3` is the historical fix commit of five rows at once, so `non_test_patch` of it is the whole
38-file change and no control can red one row's assertion while differing from the restore by that
whole patch. Wave 1 also found the historical pre-image client unpinnable in the private graph.
The committed reversal pair keeps the tip, reverses only this row's production hunk and re-lands it
in a fresh commit, which is the same pair shape as `MODEL-ITEM-BARS` / `MODEL-OPUS-15` / QA5-FAST-CLICK.

## Original commit

`client:895c63f3` "fix(client): native control lease, seat attachment, camel geometry and UI release"
(this row's part is the one hunk below).

## Hunks

| file | hunk | content | disposition |
|---|---|---|---|
| `src/session/actions.rs` | `@@ -26,6 +26,9 @@ impl Session` | `InputIntent::ReleaseAll` clears the pending UI release it already applied (`ui.release_pending = false`) | REVERSED — three added lines deleted |

## a/-side adaptation

None: the three removed lines are exactly the commit's own added lines and the surrounding context
is unchanged, so `git apply -R` yields the historical pre-image and
`git diff 365939accb 2635fc2ac5` is exactly that patch
(patch-id `85a41c27706bb941c596535ec061566b15dc80f2`).

## Proving ref (and why the row is still open)

`harness:entities:native-v2-lease-navigation` → `replacement_input_retained`: after controller A's
sneak action is cancelled, controller B's accepted forward hold must still be in `inputs.held`.

The control client passed that assertion **3/3** (receipts `20261124T114355…-kncl5zo`,
`…114642…-miqu459j`, `…114914…-lkwpj5r5`), so the scene does not discriminate for this row: nothing in
it forces the UI synchronisation that replays `UiState::release_pending`. The attempted fix was a
step that opened and closed the inventory (`UiState::execute` consumes the pending release), but a
screen change releases gameplay input on **both** clients by design — the re-landed client failed
the new `replacement_input_survives_ui_sync` too (`20261124T131141-native-v2-lease-navigation-oelqyuq9`),
so the step was removed instead of shipping a non-discriminating assertion. A real-client path that
runs a UI synchronisation **without** changing the screen is the missing piece; the client unit ref
`session::observation::debug_tests::synchronous_release_does_not_clear_replacement_input_on_ui_sync`
remains the fast oracle.
