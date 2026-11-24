# QA5-FAST-CLICK — hunk map for the committed reversal pair

Promotion receipt: `promotions/QA5-FAST-CLICK.json`.
Client pair: reversal `client:2e56cf16` → re-land `client:530556e8` (the row's `fix_commit`).

## Why a reversal and not the pre-image pin

`a2d0c81d`'s parent `9a645aa9` is post-rename and still pinnable, but `ledger promote` refuses
`fix_commit = client:a2d0c81d`: `non_test_patch` splits the commit's hunk in `src/session/actions.rs`
that appends the inline test module, and that hunk's changed lines fall on both sides of the
`#[cfg(test)] mod fast_click_tests` boundary (the added `///` doc comment sits outside the module
range, the `#[cfg(test)]` line and everything after it inside), so it refuses with "a hunk of
src/session/actions.rs straddles its #[cfg(test)] module; split the fix commit". The same pair
shape as HC-2/HC-3/MODEL-OPUS-15 is therefore used on the tip: one commit deletes the guarded
production behaviour, the next re-lands it verbatim and becomes the row's `fix_commit`.

## Original commit

`client:a2d0c81d` "fix(input): retain attack and use press edges until the next tick"
(4 files, 128 insertions, 19 deletions; `docs/AGENT_TESTING.md` is a doc hunk and is not part of the
behaviour).

## Hunks of `client:a2d0c81d`

| file | hunk (`@@` header) | content | disposition |
|---|---|---|---|
| `src/player.rs` | `@@ -135,6 +135,10 @@ pub struct HeldInput` | `attack_clicks` / `use_clicks` on `HeldInput` (vanilla `KeyMapping.consumeClick`) | REVERSED — fields deleted; nothing else reads them once the tick stops copying them |
| `src/player.rs` | `@@ -160,6 +164,8 @@ impl HeldInput` | their `from_keys` defaults (0) | REVERSED |
| `src/player.rs` | `@@ -287,6 +293,10 @@ pub struct Player` | the same two counters on `Player` | REVERSED |
| `src/player.rs` | `@@ -381,6 +391,8 @@ impl Default for Player` | their defaults | REVERSED |
| `src/player.rs` | `@@ -497,6 +509,9 @@ impl Player` | their reset ("release drops presses no tick has consumed") | REVERSED |
| `src/player.rs` | `@@ -1010,12 +1025,19 @@ impl Player` | the tick copies the taken presses into `HeldInput` unless input is blocked | REVERSED — back to the plain `HeldInput::from_keys(...)` call |
| `src/player.rs` | `@@ -1046,6 +1068,15 @@ impl Player` | `Player::click` (the press counter) | REVERSED |
| `src/player/interaction.rs` | `@@ -391,7 +391,7 @@ impl Player` | `abort_break` also skips a press already released this tick | REVERSED — `if !input.attack` again |
| `src/player/interaction.rs` | `@@ -439,29 +439,51 @@ impl Player` | `startAttack` per press, `continueAttack(false)` for a released press, `startUseItem` per press | REVERSED — one attack per held button and one use per `use_delay`, as before |
| `src/session/actions.rs` | `@@ -174,6 +174,9 @@ impl Session` | the `"attack" \| "use" => self.player.click(key)` dispatch arm | REVERSED |
| `src/session/actions.rs` | `@@ -2255,3 +2258,55 @@` | the inline `#[cfg(test)] mod fast_click_tests` (and its `///` doc comment) | OMITTED — test hunk: the kept test `session::actions::fast_click_tests::press_released_before_the_next_tick_still_starts_the_attack` is the row's unit ref and must stay to go red on the control |
| `docs/AGENT_TESTING.md` | — | documentation | OMITTED — doc hunk |

## a/-side adaptation

None. The re-land restores every reversed line verbatim: `git diff 2e56cf16 530556e8` is exactly
the production hunks above (patch-id `c9f3c8a7c5fd…`, `paths`
`src/player.rs`, `src/player/interaction.rs`, `src/session/actions.rs`), and no line outside the
original pre-image is introduced.

## Proving ref

`harness:entities:native-v2-input-menus` → `physical_short_click_breaks_block`
(`tools/harness/backends/native_player_input.py::_physical_short_click`, run before the scene freezes
the server): a physical press with its release queued inside one client tick, aimed at a
`minecraft:slime_block` (hardness 0), must leave the slot `minecraft:air`; the check is an explicit
assertion over the client's authoritative block state with a bounded 10 s wait, so a lost click fails
at that assertion rather than on a scenario timeout.

The older `survival_short_click_exact_pickup` cannot carry this row: an open screen handles a slot
click on the press event, never through the attack key mapping, so it is green on the pre-fix client
(`20261123T163307`, press+release 0.08 ms). It stays in the scene as its own coverage.
