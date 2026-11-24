# PLAYER-SPEAR-JAB-DROPPED-AT-DECODE — fix-hunk map

The production patch `PLAYER-SPEAR-JAB-DROPPED-AT-DECODE.patch` is the fix's non-test hunks,
`patch:8ab22ab90bb1813e9b5f46e4db8cb51bb1395b55c872ecb87eafd28ae5b8dc62`, built from git history:

```
git diff be976b139^..be976b139 -- crates/mc-core/src/tick/input/combat.rs \
    crates/mc-core/src/tick/players.rs crates/mc-data/data/required_item_components.json \
    crates/mc-data/src/item_components.rs crates/mc-edge/src/decode.rs
```

`git apply -R --check` passes on `6e177ea0e` (verified): the a/- side is the real pre-image before
`be976b139` (no STAB arm in edge decode, no `stab()` kernel, no `piercing_weapon`/`attack_range`
facts), and no later commit restructured those regions — the only files the fix touches are the
five above plus tests and `docs/COMPATIBILITY.md` (a doc line, left out of the patch).

## The fix

| file | hunk |
|---|---|
| `crates/mc-edge/src/decode.rs` | `PlayerActionKind::Stab` → `PlayerInput::Stab { connection }` (wire 8) |
| `crates/mc-core/src/tick/players.rs` | `PlayerInput::Stab` → `input::combat::stab` |
| `crates/mc-core/src/tick/input/combat.rs` | `stab()`: piercing-weapon facts, minimum-charge gate, near/far attack-range raycast, damage |
| `crates/mc-data/...` | the spear's `piercing_weapon` + `attack_range` item facts |

## Control

Reversing the patch reds the row's registered cargo refs at their own assertions, above all
`cargo:mc-edge:decode::decode_tests::player_action_maps_dig_drop_release_swap` (the decode table no
longer maps action 8) and
`cargo:mc-core:tick_tests::spear_stab_hits_at_range_only_when_charged` (no stab kernel).

The graphical control is a reversed server pin; it is recorded `unrunnable` in `promote.json`
(Main's CIV-B1 ruling: a reversed pin needs its own release build). The candidate's real-client
receipt is the scene below.

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
