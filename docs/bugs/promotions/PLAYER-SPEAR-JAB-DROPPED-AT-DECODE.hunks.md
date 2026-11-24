# PLAYER-SPEAR-JAB-DROPPED-AT-DECODE — fix-hunk map

The patch the promotion ran with is the decode hunk alone,
`patch:e9197d123ade38000e96734a68fbcbb604f8fa1bf30bce4caa8ad8e31a503903`
(`git diff be976b139^..be976b139 -- crates/mc-edge/src/decode.rs`); the earlier five-file patch
`8ab22ab9…` is superseded because its control tree did not compile (below). `git apply -R --check`
passes on the promotion tree `1aee24a978`.

## The fix

| file | hunk |
|---|---|
| `crates/mc-edge/src/decode.rs` | `PlayerActionKind::Stab` → `PlayerInput::Stab { connection }` (wire 8) |
| `crates/mc-core/src/tick/players.rs` | `PlayerInput::Stab` → `input::combat::stab` |
| `crates/mc-core/src/tick/input/combat.rs` | `stab()`: piercing-weapon facts, minimum-charge gate, near/far attack-range raycast, damage |
| `crates/mc-data/...` | the spear's `piercing_weapon` + `attack_range` item facts |

## Control (what the promotion measured)

Reversing the decode patch reds exactly two of the row's refs, both at their own assertions
(`promote.json` `negative_control`):

- `cargo:mc-edge:decode::decode_tests::player_action_maps_dig_drop_release_swap` —
  `crates/mc-edge/src/decode_tests.rs:579`, `left: []`;
- `cargo:mc-edge:translation::adjacent::play::tests::old_client_stab_reaches_core_input_through_adjacent_translation` —
  `crates/mc-edge/src/translation/adjacent/play_tests.rs:724`, `left: []` (`protocol 775`).

The other five refs stay green on the control (they construct `PlayerInput`/item facts directly): the
row's symptom is the decode drop, and those refs are the guards for the rest of the path. The kernel
half (`combat::stab`) is **not** covered by a control, and the graphical control is a reversed server
pin, recorded `unrunnable` per Main's CIV-B1 ruling.

## Real-client proof (candidate)

`native-v2-spear-jab-decode` (scene-shot, windowed native, registered in `scenarios.py` with
`tools/harness/spear_jab_scene.json`): a survival player with a cleared inventory is given one
`minecraft:iron_spear`, looks at a NoAI cow two blocks ahead, and the `attack_entity` step lists the
cow, approaches, dispatches one ordinary attack (the spear arm queues wire-8 STAB), and then reads the
target's own received state. The step asserts `spear_jab_holds_iron_spear`, `spear_jab_accepted`
(authoritative confirmation or a health drop) and `spear_jab_damaged` (a health drop or removal), and
writes `spear_jab.json` with the before/after rows and each attempt.

Receipt on the promotion tree (pin `660717828dd4`, client `530556e864c2`):
`.analysis/validation/<stamp>-native-v2-spear-jab-decode-<id>` — see the handout's validation table.

Limits: one cow, one swing; the swing itself is the captured frame, not a pixel oracle. The pre-fix
control is the reversing patch above (the client is unchanged between the two trees).

## The control patch is the decode half only (`patch:e9197d12…`, 2026-11-25)

The five-file patch (`8ab22ab9…`) reverses cleanly on the production files but its control tree does
not compile: `crates/mc-data/src/item_components_tests.rs` (a kept test file) reads
`spear.piercing_weapon` / `spear.attack_range`, which reversing `item_components.rs` removes, and
`crates/mc-core/src/tick_tests.rs` drives `PlayerInput::Stab` through `players.rs`'s match arm. The
promotion's control patch is therefore the decode hunk alone:

```
git diff be976b139^..be976b139 -- crates/mc-edge/src/decode.rs
```

sha256 `e9197d123ade38000e96734a68fbcbb604f8fa1bf30bce4caa8ad8e31a503903`; a/- side = the real
pre-image (`PlayerActionKind::Stab => return Ok(())`, the row's "dropped at decode"), b/- side = HEAD.
`git apply -R --check` passes. Reversing it reds the row's decode ref
(`cargo:mc-edge:decode::decode_tests::player_action_maps_dig_drop_release_swap`); the other refs stay
green because they construct `PlayerInput`/item facts directly, which is expected — the row's symptom
is the decode drop, and the translation/kernel refs are the guards that the rest of the path works.
The full fix remains documented above and in the row's `source`; this file's `fix_commit` names the
patch the promotion actually reverses.
