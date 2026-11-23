# DAMAGE-COMMAND: patch-mode fix identity

Original fix, two commits:

1. `7fe490528` (`fix(core): preserve native damage identity and execute damage commands`) — added the
   native `/damage` handler (`crates/mc-core/src/command_damage.rs`), the `Command::Damage` variant and
   its dispatch arm, the damage-attribution pipeline, and the edge command tree/suggestions for `damage`.
2. `838909abc` (`fix(commands): enforce native numeric permission levels`) — introduced the shared
   `crates/mc-core/src/commands.rs` `ROOTS` table (advertising `damage` at level 2) and made the edge
   `command_tree.rs` read it (`use mc_core::commands::{ROOTS, root_permitted}`).

`fix_commit = "patch:ff8339d548af2b08…"` names the HEAD equivalents of this row's non-test hunks in
`docs/bugs/promotions/DAMAGE-COMMAND.patch` (a diff from the pre-fix form to the candidate; `git apply -R`
restores the pre-fix code). Re-audit:
`python3 -m tools.harness ledger promote DAMAGE-COMMAND --fix-patch docs/bugs/promotions/DAMAGE-COMMAND.patch --dry-run`.
Rule (docs/bugs/README.md, step 1): every original non-test hunk is carried unless it is another row's or
owner's behaviour, API the kept tests call, or code a later commit rewrote.

## Carried

- `838909abc commands.rs @@ -9,6 +9,86 @@` (new `ROOTS` table) — the `("damage", 2),` entry, HEAD
  `crates/mc-core/src/commands.rs:38`. This line is also what the edge advertises: `command_tree.rs:31`
  imports `mc_core::commands::ROOTS` and `:550/:562` builds and permission-checks the roots from it, so
  the edge tree/suggestions hunks of `7fe490528` are inert without this entry (see below).
- `7fe490528 commands.rs @@ -326,6 +327,7 @@` — `"damage" => crate::command_damage::parse(args).map(
  Command::Damage),`, HEAD `crates/mc-core/src/commands.rs:444`.

## Original non-test hunks (complete)

```
## 7fe490528 non-test hunks (kept test files: the commit's *_tests.rs, including command_damage_tests.rs)
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

## 838909abc non-test hunks
- crates/mc-core/src/command_context.rs: -366,7 +366,7; -588,18 +588,22; -649,3 +653,7
- crates/mc-core/src/commands.rs: -9,6 +9,86
- crates/mc-edge/src/command_tree.rs: -28,6 +28,7; -38,76 +39,6; -239,9 +170,7
```

## Not carried

- `command_damage.rs @@ -0,0 +1,225 @@` — (b) the kept `command_damage_tests.rs` calls the module
  (`commands::parse` → `Command::Damage` → `command_damage::parse`/`report`); deleting it stops the
  control from compiling. Its presence with the dispatch arm reversed still reproduces the row's symptom
  (`/damage` is an unknown command), which is what the ref asserts.
- `commands.rs @@ -207,6 +207,7 @@` (the `Command::Damage` variant) — (b) the kept test constructs and
  matches the variant; reversing it would break the control's compile.
- `intent.rs @@ -99,6 +99,14 @@` (`DamageSource::Command`) and `survival.rs -107,6`, `-144,6` etc.
  (`command_type`, `GenericKill`, `bypasses_invulnerability`) — (b) `command_damage.rs` constructs
  `DamageSource::Command` and calls `DamageKind::command_type`, so the control keeps them to compile.
- The remaining `survival.rs`/`tick/damage.rs`/`entity_loot.rs`/`advancements.rs`/`bees.rs`/
  `enchantment_thorns.rs`/`physical_defense.rs`/`plugin*.rs`/`projectiles`/`special_mobs`/`statistics.rs`
  hunks — (a) the native damage-identity and death rows' behaviour; reversing them would red their own
  tests first (e.g. death messages, `GenericKill` for `/kill`, indirect magic).
- `scoreboard.rs`/`scoreboard_persistence.rs`/`boundary.rs`/`tick/death_message.rs`/`tick/auth.rs`/
  `tick/input/session.rs`/`mc-edge/plan/player_changes.rs`/`mc-protocol/.../combat_kill.rs` — (a) the
  B08/B09 score and team-death-visibility rows and DEATH-NOTIFY/DEATH-MESSAGE-OUTSIDE-BORDER-KEY.
- `mc-edge/src/command_tree.rs` damage blocks (`-806,6 +807,32` tree, `-1740,6 +1767,43` suggestions) and
  `838909abc`'s edge `ROOTS` rewrite (`-38,76 +39,6`) — (a)/(c) inert behind the carried root: the edge
  builds and permission-checks its roots from `mc_core::commands::ROOTS` (HEAD `command_tree.rs:31`,
  `:550`, `:562`), so with the carried line reversed at `commands.rs:38` no `damage` root exists and the
  tree/suggestion code is unreachable. `838909abc`'s `-28,6` import and `-239,9` permission check are the
  same wiring and stay.
- Test files of both commits — (b) kept so the control compiles and runs the ref; the `command_tree.rs`
  `damage_tree_and_permission_match_native_grammar` test is likewise kept.
- `docs/` hunks — (b) ledger text.

## Proving ref (file:line of the failing assertion)

- `cargo:mc-core:command_damage::tests::damage_command_commits_default_damage_and_reports_actual_rejection`
  — with both carried hunks reversed the test helper `run` (`crates/mc-core/src/command_damage_tests.rs:6`)
  panics on `crate::commands::parse(text, true).unwrap()` because `/damage` is an unknown command again;
  the test's assertions are `command_damage_tests.rs:69-73` (default damage committed, success feedback).
- Client ref `harness:entities:native-v2-command-score-store` is green on the gate-wave tree
  (`20261123T163153-native-v2-command-score-store-qk0xz17p`); it was unrunnable in the control (no pin for
  the reverted tree), so it is not part of the red.
