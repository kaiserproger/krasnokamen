# DEATH-NOTIFY: patch-mode fix identity

Original fix: `7fe490528` (`fix(core): preserve native damage identity and execute damage commands`; the
BUGS.md B08 entry names the source `7b91c2b7` integrated as this commit) — the death path pushed
`PlayerChange::Died` to the dying player's own connection only inside the `show_death_messages` gate, so
turning observer broadcast off also suppressed the victim's own death screen. The fix builds the message
under the gate but always pushes `Died`. `fix_commit = "patch:616679e6…"` names the HEAD equivalent of
that hunk in `docs/bugs/promotions/DEATH-NOTIFY.patch` (a diff from the pre-fix form to the candidate;
`git apply -R` restores the pre-fix code). Re-audit:
`python3 -m tools.harness ledger promote DEATH-NOTIFY --fix-patch docs/bugs/promotions/DEATH-NOTIFY.patch --dry-run`.
Rule (docs/bugs/README.md, step 1): every original non-test hunk is carried unless it is another row's or
owner's behaviour, API the kept tests call, or code a later commit rewrote.

## Carried

- `tick/damage.rs @@ -428,52 +425,25 @@` (the death-delivery rewrite) — the HEAD equivalent is
  `crates/mc-core/src/tick/damage.rs:801-812`: `message` chosen under `show`, the unconditional
  `push_player(connection, PlayerChange::Died { message })`, and `broadcast` still gated.
  **a/-side line: reordering.** The reverse is not "prior code": it is the same HEAD lines moved back
  inside `if show` (the `message` binding moves with them), because the original hunk reordered the head of
  the death block; no line of the a/-side is invented and `death_message::component` already exists
  (created by the same `7fe490528`, hunk `tick/death_message.rs -0,0 +1,128`).
  The rest of the original hunk `-428,52 +425,25` (KEEP_INVENTORY/world-drop and `show_death_messages`
  branches, the offline/dead-connection drop fallback, the grave hook) is **present at HEAD unchanged**
  (`crates/mc-core/src/tick/damage.rs:812-900`): those lines were not in the patch because a later commit
  (`b7020b427`, "integrate death protection and entity damage state") rewrote the surrounding block and
  left them; reversing them would red the death-drop/inventory rows, not this one — (a)/(c).

## Original non-test hunks (complete)

```
## 7fe490528 non-test hunks (kept test files: tick/damage_scoreboard_tests.rs and the other *_tests.rs of the commit)
- crates/mc-core/src/advancements.rs: -1065,6 +1065,7; -1103,6 +1104,20
- crates/mc-core/src/bees.rs: -1289,6 +1289,10
- crates/mc-core/src/boundary.rs: -965,7 +965,7
- crates/mc-core/src/command_damage.rs: -0,0 +1,225
- crates/mc-core/src/command_execution.rs: -1060,7 +1060,7; -1078,6 +1078,14
- crates/mc-core/src/commands.rs: -207,6 +207,7; -326,6 +327,7
- crates/mc-core/src/dimensions_commands.rs: -22,6 +22,7; -45,6 +46,7
- crates/mc-core/src/effects.rs: -137,7 +137,7
- crates/mc-core/src/enchantment_thorns.rs: -52,6 +52,11
- crates/mc-core/src/entities.rs: -1077,7 +1077,7
- crates/mc-core/src/entity_loot.rs: -118,6 +118,11; -201,6 +206,22; -234,7 +255,7; -433,7 +454,9; -481,7 +504,9
- crates/mc-core/src/intent.rs: -99,6 +99,14
- crates/mc-core/src/lib.rs: -47,6 +47,7
- crates/mc-core/src/physical_defense.rs: -745,6 +745,19
- crates/mc-core/src/plugin.rs: -2594,6 +2594,7
- crates/mc-core/src/plugin_player.rs: -926,7 +926,7
- crates/mc-core/src/projectiles/payloads.rs: -319,7 +319,7; -1335,7 +1335,7
- crates/mc-core/src/scoreboard.rs: -114,6 +114,7; -131,6 +132,7; -221,6 +223,7; -231,10 +234,19; -285,6 +297,15; -587,12 +608,17; -824,7 +850,12; -837,9 +868,9; -1018,10 +1049,67
- crates/mc-core/src/scoreboard_persistence.rs: -296,6 +296,10; -424,9 +428,6; -442,6 +443,12; -496,6 +503,7
- crates/mc-core/src/special_mobs/mod.rs: -394,7 +394,18
- crates/mc-core/src/special_mobs/wither.rs: -26,7 +26,7
- crates/mc-core/src/statistics.rs: -259,6 +259,9; -292,6 +295,9
- crates/mc-core/src/survival.rs: -94,11 +94,67; -107,6 +163,7; -144,6 +201,14; -176,6 +241,14; -184,7 +257,7; -213,6 +286,14; -523,7 +604,9; -568,7 +651,9
- crates/mc-core/src/tick.rs: -35,6 +35,7
- crates/mc-core/src/tick/auth.rs: -174,7 +174,7
- crates/mc-core/src/tick/damage.rs: -22,6 +22,7; -37,17 +38,23; -107,12 +114,16; -136,18 +147,6; -186,6 +185,7; -344,6 +344,7; -375,11 +376,7; -428,52 +425,25
- crates/mc-core/src/tick/death_message.rs: -0,0 +1,128
- crates/mc-core/src/tick/input/session.rs: -325,7 +325,7
- crates/mc-edge/src/command_tree.rs: -52,6 +52,7; -806,6 +807,32; -1740,6 +1767,43; -1809,6 +1873,53
- crates/mc-edge/src/plan/player_changes.rs: -15,18 +15,19; -678,8 +679,19
- crates/mc-protocol/src/packets/play.rs: -30,6 +30,8
- crates/mc-protocol/src/packets/play/combat_kill.rs: -0,0 +1,61
```

## Not carried

- The `tick/damage.rs` death-*content* hunks (`-344,6`, `-375,11`, `-136,18`) and `tick/death_message.rs`
  — (a) DEATH-MESSAGE-OUTSIDE-BORDER-KEY / DEATH-COMPONENT rows (their own controls carry them) and the
  message-component content this row does not assert.
- `boundary.rs`/`tick/auth.rs`/`tick/input/session.rs`/mc-edge `player_changes`/mc-protocol `combat_kill`
  — (a) the death-screen packet row and the team-visibility rows.
- Every other file — (a) the `/damage` handler, score counters and damage-identity rows as listed in
  SCORE-FOOD.hunks.md's identical listing.
- Test files — (b) kept; `docs/ARCHITECTURE.md` — (b) docs.

## Proving ref (file:line of the failing assertion)

- `cargo:mc-core:tick_tests::damage_scoreboard_tests::damage_scoreboard_death_notification_ignores_broadcast_rule`
  — `crates/mc-core/src/tick/damage_scoreboard_tests.rs:75-79`
  `assert!(state.outbox[&ConnectionId(1)].iter().any(|change| matches!(change, PlayerChange::Died { .. })))`
  fails once the push is back inside `if show` (the dying player receives nothing while the observer
  broadcast stays suppressed, `:80-90`).
- Client ref `harness:entities:native-v2-death-messages` green receipt on the candidate `9c78eee8e`:
  `.analysis/validation/20261123T211907-native-v2-death-messages-9epvsbj2/result.json` (assertion
  `victim_gets_death_screen_despite_show_death_messages_false`; promotion record
  `docs/bugs/promotions/DEATH-NOTIFY.json`).
