# SCORE-CRITERIA-WRITABLE: patch-mode fix identity

Original fix: `7fe490528` (`fix(core): preserve native damage identity and execute damage commands`) —
the `execute_players` guards that let `set`/`add`/`remove`/`operation` write the vanilla counter
criteria (`deathCount`, `playerKillCount`, `totalKillCount`, and the `health|food|air|armor|xp|level`
family) and swap a writable source in `operation ><`. `fix_commit =
"patch:49684fbf13f63f6519…"` names the HEAD equivalents of this row's non-test hunks in
`docs/bugs/promotions/SCORE-CRITERIA-WRITABLE.patch` (a diff from the pre-fix form to the candidate;
`git apply -R` restores the pre-fix code). Re-audit:
`python3 -m tools.harness ledger promote SCORE-CRITERIA-WRITABLE --fix-patch docs/bugs/promotions/SCORE-CRITERIA-WRITABLE.patch --dry-run`.
Rule (docs/bugs/README.md, step 1): every original non-test hunk is carried unless it is another row's or
owner's behaviour, API the kept tests call, or code a later commit rewrote.

## Carried

- `scoreboard.rs @@ -824,7 +850,12 @@ fn execute_players` — `} else if a[1] != "reset" && matches!(
  definition.criteria.as_str(), "health" | "food" | "air" | "armor" | "xp" | "level" )` instead of the
  old `"dummy" | "trigger"` refusal; HEAD `crates/mc-core/src/scoreboard.rs:916-921`.
- `scoreboard.rs @@ -837,9 +868,9 @@ fn execute_players` — the `><` swap target accepts the same
  criteria (`matches!` replacing `!matches!`); HEAD `crates/mc-core/src/scoreboard.rs:934-936`.

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

- The criteria-list line `-231,10` and `automatic()` hunk `-1018,10` — (a) SCORE-FOOD and
  SCORE-TOTAL-KILLS' accepted-criteria and auto-value behaviour; their own controls reverse the lines
  they need. The team-visibility and death-message hunks are TEAM-DEATH-VISIBILITY / DEATH-* rows (a);
  the `/damage` and attribution pipeline hunks are DAMAGE-COMMAND and the damage-identity rows (a).
- Test files of the commit — (b) kept so the control compiles and runs the ref.

## Proving ref (file:line of the failing assertion)

- `cargo:mc-core:tick_tests::damage_scoreboard_tests::damage_scoreboard_counter_criteria_allow_writes_and_swap_sources`
  — `crates/mc-core/src/tick/damage_scoreboard_tests.rs:185`
  `assert_eq!(state.scoreboard.scores["Alice"][criterion].value, 4)` (the `set 2`/`add 3`/`remove 1`
  sequence is refused as read-only once the guard is reversed; the review's observed red is
  `no entry found for key` on the `deaths` write) and `:203-204` for the `><` swap.
