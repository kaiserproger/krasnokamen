# CLIENT-CONTROL-Q-WHOLE-STACK-NOT-SENT — pair and hunk map

Row: `docs/bugs/rows/CLIENT-CONTROL-Q-WHOLE-STACK-NOT-SENT.toml` (source: review m4 of
REVIEW-wave2-11). Vanilla `Minecraft.handleKeybinds` calls `player.drop(Screen.hasControlDown())`;
the native client built the world-level drop intent with `whole_stack: false` unconditionally
(`src/session/actions.rs:182` before the fix).

## Fix commit

`client:584f70baaca4de852af3d1b4e943d535ab5cec96` — "fix(input): drop the whole selected stack when
Ctrl is held".

| file | production hunk |
|---|---|
| `src/api.rs` | `InputIntent::Control { down }`: the window reports Ctrl on its own, but a gameplay key intent carries no modifiers |
| `src/session.rs` | `Session::control_down` field + initialiser |
| `src/session/actions.rs` | `ReleaseAll` clears it; the `Key` arm records a named `key.keyboard.*control` event; the drop arm reads `whole_stack: self.control_down` |
| `src/lib.rs` | `WindowEvent::ModifiersChanged` reports `state.control_key()` to the session |
| `src/session/actions.rs` (`#[cfg(test)] mod item_key_tests`) | `control_held_drop_sends_the_whole_stack_ordinal`: the real input path is driven and the PLAYER_ACTION wire ordinal is read (4 = drop the stack, 5 = drop one item) |

## Control commit

`client:7cc14fefc2ca...` — `git revert` of `584f70ba` on the same branch chain
(`wt/D123ClientNativeF-qcontrol`). Its `a/-` side is the fix commit's own pre-image (the fixed lines
are removed and the historical `whole_stack: false` line is restored verbatim), so the control is a
committed reversal and not a hand re-break. `git diff 7cc14fef 584f70ba` is exactly the fix's change
to four files (76 insertions, 1 deletion); the test module added by the fix is carried by the same
file, and `--client-control-receipt` only needs the graphical ref, so the control client is not
required to run it.

Pins: restore `pins/584f70baaca4-client-x86_64-unknown-linux-gnu-incr`,
control `pins/7cc14fefc2ca-client-x86_64-unknown-linux-gnu-incr`.

## Scene

`harness:entities:native-v2-control-q-stack` (`tools/harness/backends/native_control_q.py`), its own
scenario id. Two legs on the same key path: a plain `Q` press drops one item (the stack reads 4 and
the ground holds one entity of 1), then `debug give` re-sets the slot to five (it *sets*, it does not
add) and one `minecraft_press_inputs` call holds Ctrl and Q together. The client predicts no drop, so
the scene waits for the server's own slot update and then asserts the count outcome: the ground holds
**one** entity of 5 (`control_q_whole_stack_dropped`), not five entities of one, and the slot reads 0.
A pre-fix client leaves 4 in the slot and drops one item, so the registered assertion records false —
an observed assertion failure, not a timeout.
