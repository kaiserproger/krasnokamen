# CLIENT-HELD-DROP-KEY-DOUBLE-DROP — hunk map for the committed reversal pair

Client pair: reversal `client:8f0c025e` → re-land `client:e1080bb6` (the row's `fix_commit`).
Real fix: `client:405ff485` ("fix(input): dispatch drop, pick and swap hands once per press").

## Why a reversal and not the fix commit's own patch

`405ff485` cannot be split by the ledger: its third hunk (`src/session/actions.rs`,
`@@ -2310,3 +2310,204 @@ mod fast_click_tests {`) appends a `#[cfg(test)] mod item_key_tests` block directly after an
inline test module, so the hunk straddles the test-module boundary and `non_test_patch` refuses it.
`docs/bugs/README.md` §3's reversal route (the `QA5-FAST-CLICK` precedent) splits the commit by hand:
the production hunks become the pair, the appended test module stays in both commits.

## The pair's own diff

```
git diff 8f0c025e e1080bb6        # patch-id cd57188952935296dc24f224489b631aaedcf254
```

equals

```
git show 405ff485 -- src/player.rs src/player/interaction.rs   (100 lines, same patch-id)
```

## Hunks

| file | hunk | content | disposition |
|---|---|---|---|
| `src/player.rs` | `@@ -131,10 +131,7 @@` | `HeldInput`'s `pick`, `drop`, `swap_hands` fields (`pub struct HeldInput`) | **REVERSED** — the reversal re-adds the three fields |
| `src/player.rs` | `@@ -160,10 +157,7 @@` | `HeldInput::default()` no longer initialises them | **REVERSED** |
| `src/player.rs` | `@@ -183,10 +177,7 @@` | `HeldInput::clear()` no longer clears them | **REVERSED** |
| `src/player/interaction.rs` | `@@ -406,39 +406,10 @@` | the held-key tick edge dispatching `drop`/`pick`/`swap_hands` (removed so the press edge is the only dispatcher) | **REVERSED** — the reversal restores the duplicate dispatch |
| `src/session/actions.rs` | `@@ -2310,3 +2310,204 @@` | `#[cfg(test)] mod item_key_tests` appended after `fast_click_tests` | KEPT (test hunk; the module is in both commits) |

## a/-side adaptation

None. `git apply -R` of the three production hunks applies to the wave-2 tip with intact context, so
every reversed line is the historical pre-image and the pair's delta is that patch verbatim.

## Proving ref — the held-key leg

`harness:entities:native-v2-input-menus` → `held_drop_once`.

`menus()` (survival branch) seeds five diamonds into the first main slot, exchanges that slot into the
hotbar with a physical `1` press, closes the screen (`held_q_fixture_selected`: five diamonds
selected), then `_hold_key(steps, ord("q"), 3)`: a physical XTest key-down held across three client
ticks and released, then four ticks of settling, then `minecraft_read_item_components` on the selected
stack. Vanilla drains `keyDrop` with `consumeClick` (`Minecraft#handleKeybinds`, client jar offsets
568-615), so one press is one drop: the count must be four. The pre-fix client's tick edge dispatches
the same drop a second time, so the count is three — a red `held_drop_once`.

X11 auto-repeat (the repeat delay is far longer than three ticks) cannot add a press in this window,
so the only way to a second drop is the removed tick edge.
